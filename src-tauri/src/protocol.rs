//! `zhiyan://` 自定义协议：在内存里解密附件喂给 WebView（§12.2）。
//!
//! ```text
//! zhiyan://blob/<64 位十六进制 plain_hash>
//! zhiyan://thumb/<64 位十六进制 plain_hash>
//! ```
//!
//! ## 为什么必须是自定义协议
//!
//! **绝不能把附件解密到临时文件再用 `file://` 加载。** 那等于把明文写回磁盘，
//! 而且那个临时文件的生命周期没人管得住——进程崩了、系统断电，它就一直躺在
//! 那儿。§8 的整套加密会被这一步抵消掉。
//!
//! 走自定义协议的话，密文从 `blobs\` 读进来，在进程内解密，字节直接进 WebView，
//! 全程不落地。
//!
//! ## 必须带 `Cache-Control: no-store`
//!
//! 这条容易漏。WebView2 会把普通响应缓存到磁盘上——那正是我们刚刚解密出来的
//! 明文。加了 `no-store` 之后它只在内存里待着。
//!
//! ## 输入是不可信的
//!
//! URL 由渲染层拼出来，而渲染层处理的是用户正文。所以路径段**只接受
//! 64 个十六进制字符**，再解成 32 字节交给 `BlobStore` 自己去拼路径——
//! 那串字符从头到尾没有当过路径用，目录穿越无从谈起。

use tauri::http::{header, Request, Response, StatusCode};
use tauri::Manager;

use crate::state::AppState;

/// 处理一次 `zhiyan://` 请求。
pub fn handle(app: &tauri::AppHandle, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    match route(app, request) {
        Ok(r) => r,
        Err(status) => Response::builder()
            .status(status)
            .header(header::CACHE_CONTROL, "no-store")
            .body(Vec::new())
            .expect("构造空响应不会失败"),
    }
}

fn route(
    app: &tauri::AppHandle,
    request: &Request<Vec<u8>>,
) -> Result<Response<Vec<u8>>, StatusCode> {
    // 只读，没有别的动词
    if request.method() != tauri::http::Method::GET {
        return Err(StatusCode::METHOD_NOT_ALLOWED);
    }

    let (kind, hash) = parse_path(request.uri())?;
    let state = app.state::<AppState>();

    let bytes = match kind {
        Kind::Blob => state
            .with_dek(|dek| Ok(state.blobs().get(dek, &hash)?))
            .map_err(|_| StatusCode::NOT_FOUND)?,
        Kind::Thumb => state
            .with_dek(|dek| Ok(state.blobs().get_thumb(dek, &hash)?))
            .map_err(|_| StatusCode::NOT_FOUND)?
            .ok_or(StatusCode::NOT_FOUND)?,
    };

    // MIME 从库里查，**不从 URL 猜也不从字节嗅探**。
    // 让 WebView 自己嗅探等于把「这段字节该怎么解释」交给内容本身决定
    let mime = state
        .read(|s| Ok(s.blob_meta(&hash)?))
        .ok()
        .flatten()
        .map(|m| m.mime)
        .filter(|m| is_safe_image_mime(m))
        .unwrap_or_else(|| "application/octet-stream".into());

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, bytes.len())
        // 明文不许进 WebView2 的磁盘缓存
        .header(header::CACHE_CONTROL, "no-store")
        // 附件只给自己的页面用
        .header("Cross-Origin-Resource-Policy", "same-origin")
        // 即便 MIME 查错了也不让 WebView 去嗅探
        .header("X-Content-Type-Options", "nosniff")
        .body(bytes)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Blob,
    Thumb,
}

/// 白名单。与 `put_blob` 那边保持一致。
fn is_safe_image_mime(m: &str) -> bool {
    matches!(
        m,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    )
}

/// 从 URI 里取出 `(种类, 32 字节哈希)`。
///
/// 各平台的 URL 形状不一样：Windows 上 `zhiyan://blob/ab…` 会被改写成
/// `http://zhiyan.localhost/blob/ab…`，Linux 上 `blob` 落在 host 位置。
/// 所以把 host 与 path 拼起来一起看，只认最后两段。
fn parse_path(uri: &tauri::http::Uri) -> Result<(Kind, [u8; 32]), StatusCode> {
    let mut segs: Vec<&str> = Vec::new();
    if let Some(h) = uri.host() {
        if !h.eq_ignore_ascii_case("zhiyan.localhost") && !h.eq_ignore_ascii_case("localhost") {
            segs.push(h);
        }
    }
    segs.extend(uri.path().split('/').filter(|s| !s.is_empty()));

    let [kind, hex] = segs[segs.len().saturating_sub(2)..] else {
        return Err(StatusCode::BAD_REQUEST);
    };

    let kind = match kind {
        "blob" => Kind::Blob,
        "thumb" => Kind::Thumb,
        _ => return Err(StatusCode::NOT_FOUND),
    };

    // 严格 64 位十六进制。这一段从头到尾没有当过路径用
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let bytes = hex::decode(hex).map_err(|_| StatusCode::BAD_REQUEST)?;
    let hash = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok((kind, hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "a3f9c2e10000000000000000000000000000000000000000000000000000beef";

    fn uri(s: &str) -> tauri::http::Uri {
        s.parse().unwrap()
    }

    #[test]
    fn 认得各平台的_url_形状() {
        let want = hex::decode(HEX).unwrap();
        for u in [
            format!("zhiyan://blob/{HEX}"),
            format!("http://zhiyan.localhost/blob/{HEX}"),
            format!("https://zhiyan.localhost/blob/{HEX}"),
        ] {
            let (kind, hash) = parse_path(&uri(&u)).unwrap_or_else(|e| panic!("{u} → {e}"));
            assert_eq!(kind, Kind::Blob);
            assert_eq!(&hash[..], &want[..]);
        }
    }

    #[test]
    fn 缩略图走另一条路径() {
        let (kind, _) = parse_path(&uri(&format!("zhiyan://thumb/{HEX}"))).unwrap();
        assert_eq!(kind, Kind::Thumb);
    }

    #[test]
    fn 只认十六进制且必须整整六十四位() {
        for bad in [
            format!("zhiyan://blob/{}", &HEX[..63]),
            format!("zhiyan://blob/{HEX}f"),
            "zhiyan://blob/zzzz".into(),
            "zhiyan://blob/".into(),
        ] {
            assert!(parse_path(&uri(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn 目录穿越进不来() {
        // 这一段从来没被当成路径用过，但还是要有一条测试钉住
        for bad in [
            "zhiyan://blob/../../../../windows/system32/config/sam",
            "zhiyan://blob/..%2f..%2fsecret",
            "zhiyan://blob/C:\\Users",
        ] {
            assert!(parse_path(&uri(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn 认不出的种类是_404_而不是当成_blob() {
        assert_eq!(
            parse_path(&uri(&format!("zhiyan://secrets/{HEX}"))),
            Err(StatusCode::NOT_FOUND)
        );
    }

    #[test]
    fn mime_白名单挡住可执行类型() {
        assert!(is_safe_image_mime("image/png"));
        for bad in [
            "text/html",
            "image/svg+xml", // SVG 里能写脚本
            "application/javascript",
            "",
        ] {
            assert!(!is_safe_image_mime(bad), "{bad}");
        }
    }
}
