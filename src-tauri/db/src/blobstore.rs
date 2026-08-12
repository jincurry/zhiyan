//! 附件：内容寻址 + 收敛加密（§7.3 ①、§8.5、§9.1）。
//!
//! ```text
//! %APPDATA%\Zhiyan\
//! ├─ blobs\a3\a3f9c2e1…          附件密文，按 plain_hash 前两位分桶
//! └─ cache\thumbs\a3\a3f9c2e1…   缩略图密文
//! ```
//!
//! ## 为什么外置
//!
//! 原型把图片存成 base64 塞进 JSON：2MB 的图膨胀成 2.7MB 文本，而且每次读写
//! 整条记录都要把它搬一遍。改成内容寻址后数据库里只剩 32 字节哈希。
//!
//! ## 分桶
//!
//! 一级目录取哈希前两位。不分桶的话上万个文件挤在一个目录里，
//! NTFS 上的目录枚举和资源管理器都会明显变慢。
//!
//! ## 缩略图也必须加密
//!
//! 这是最容易漏的一项：缩略图能直接看出原图内容，明文缓存等于 §8 白做。
//! 它用另一把密钥（`info="thumb"||plain_hash`），也另有一套 AAD。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use zhiyan_crypto::{
    keys::{cipher_id, derive_blob_key, derive_thumb_key, deterministic_blob_nonce, plain_hash},
    open, seal_with_nonce, DataKey, Identity,
};

use crate::model::BlobMeta;
use crate::{DbError, Result, Store};

fn io<E: core::fmt::Display>(what: &str) -> impl FnOnce(E) -> DbError + '_ {
    // 只带操作名与错误码，**不带路径**——路径里有 Windows 用户名（§14 ①）
    move |e| DbError::Io(format!("{what}: {e}"))
}

/// 附件的落盘层。
///
/// 只管字节；`blob` 表里的元信息由 [`Store`] 管。两件事分开是因为它们的失败
/// 模式不同：文件系统不参与 SQLite 的事务回滚，混在一起会出现「表里有记录、
/// 磁盘上没文件」的半截状态。
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// `root` 是 `%APPDATA%\Zhiyan\`。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn blob_path(&self, hash: &[u8; 32]) -> PathBuf {
        let hex = hex::encode(hash);
        self.root.join("blobs").join(&hex[..2]).join(hex)
    }

    fn thumb_path(&self, hash: &[u8; 32]) -> PathBuf {
        let hex = hex::encode(hash);
        self.root
            .join("cache")
            .join("thumbs")
            .join(&hex[..2])
            .join(hex)
    }

    /// 写入密文。
    ///
    /// 先写临时文件再 rename：内容寻址下，一个被写了一半的文件会占着「正确」的
    /// 文件名，之后每次读都失败，而去重逻辑还以为它已经存在了。
    fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = path.parent().expect("blob 路径一定有父目录");
        fs::create_dir_all(dir).map_err(io("建目录"))?;

        let tmp = dir.join(format!(
            ".{}.{}.tmp",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&tmp, bytes).map_err(io("写临时文件"))?;
        fs::rename(&tmp, path).map_err(io("改名"))?;
        Ok(())
    }

    /// 附件是否已在本地。
    pub fn contains(&self, hash: &[u8; 32]) -> bool {
        self.blob_path(hash).exists()
    }

    /// 加密并落盘，返回 `(plain_hash, cipher_id)`。
    ///
    /// 收敛加密（§8.5）：同一用户的同一份明文每次算出的密文完全一致，
    /// 所以上传前 `HEAD` 一下 `cipher_id` 就能跳过重复上传。
    pub fn put(&self, dek: &DataKey, bytes: &[u8]) -> Result<([u8; 32], [u8; 32])> {
        let h = plain_hash(bytes);
        let cek = derive_blob_key(dek, &h)?;
        let nonce = deterministic_blob_nonce(&cek);
        let sealed = seal_with_nonce(cek.as_bytes(), Identity::Blob(&h), &nonce, bytes)?;
        let cid = cipher_id(&sealed);
        Self::write_atomic(&self.blob_path(&h), &sealed)?;
        Ok((h, cid))
    }

    /// 读回明文。
    ///
    /// **绝不解密到临时文件再用 `file://` 加载**（§12.2）：那等于把明文写回磁盘，
    /// 而且那个临时文件的生命周期没人管得住。字节在内存里交给
    /// `zhiyan://` 自定义协议（阶段四）。
    pub fn get(&self, dek: &DataKey, hash: &[u8; 32]) -> Result<Vec<u8>> {
        let sealed = fs::read(self.blob_path(hash)).map_err(io("读附件"))?;
        let cek = derive_blob_key(dek, hash)?;
        let plain = open(cek.as_bytes(), Identity::Blob(hash), &sealed)?;
        // AEAD 的 tag 已经把 plain_hash 绑进 AAD 了，这里再核一次是防「文件放错了桶」
        // 这类不涉及密钥的错误
        if plain_hash(&plain) != *hash {
            return Err(DbError::BlobCorrupted);
        }
        Ok(plain)
    }

    /// 缩略图：另一把密钥、另一套 AAD。
    pub fn put_thumb(&self, dek: &DataKey, hash: &[u8; 32], bytes: &[u8]) -> Result<()> {
        let key = derive_thumb_key(dek, hash)?;
        let nonce = deterministic_blob_nonce(&key);
        let sealed = seal_with_nonce(key.as_bytes(), thumb_identity(hash).as_ref(), &nonce, bytes)?;
        Self::write_atomic(&self.thumb_path(hash), &sealed)
    }

    pub fn get_thumb(&self, dek: &DataKey, hash: &[u8; 32]) -> Result<Option<Vec<u8>>> {
        let path = self.thumb_path(hash);
        if !path.exists() {
            return Ok(None);
        }
        let sealed = fs::read(path).map_err(io("读缩略图"))?;
        let key = derive_thumb_key(dek, hash)?;
        Ok(Some(open(
            key.as_bytes(),
            thumb_identity(hash).as_ref(),
            &sealed,
        )?))
    }

    /// 删掉一个附件的密文与缩略图。文件不在也算成功。
    pub fn remove(&self, hash: &[u8; 32]) -> Result<()> {
        for p in [self.blob_path(hash), self.thumb_path(hash)] {
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io("删附件")(e)),
            }
        }
        Ok(())
    }
}

/// 缩略图的 AAD 域。
///
/// 与原图分开：密钥本来就不同，但万一哪天派生写错了，域分隔能让它当场解密失败，
/// 而不是悄悄用错的密钥解出一堆乱码。
struct ThumbId(Vec<u8>);

impl ThumbId {
    fn as_ref(&self) -> Identity<'_> {
        Identity::Context(&self.0)
    }
}

fn thumb_identity(hash: &[u8; 32]) -> ThumbId {
    let mut v = Vec::with_capacity(5 + 32);
    v.extend_from_slice(b"thumb");
    v.extend_from_slice(hash);
    ThumbId(v)
}

impl Store {
    /// §9.6 的 `put_blob`：落盘 + 记元信息，一次做完。
    ///
    /// 宽高由调用方给——前端从 `<img>` 上读得到。为了两个整数把 `image` 及其
    /// 一串编解码器拖进依赖树，换来的是安装包体积和一批解析器攻击面。
    pub fn put_blob(
        &self,
        blobs: &BlobStore,
        dek: &DataKey,
        bytes: &[u8],
        mime: &str,
        w: Option<i64>,
        h: Option<i64>,
    ) -> Result<BlobMeta> {
        let (hash, cid) = blobs.put(dek, bytes)?;

        // refcount 不在这里加：它由 upsert_memo 按正文里实际引用了几次来算。
        // 这里加的话，用户贴了图又撤销，这个附件就永远删不掉了
        self.conn().prepare_cached(
            "INSERT INTO blob(plain_hash, cipher_id, mime, size, width, height, refcount, uploaded) \
             VALUES(?1,?2,?3,?4,?5,?6,0,0) \
             ON CONFLICT(plain_hash) DO UPDATE SET \
               cipher_id=excluded.cipher_id, mime=excluded.mime, size=excluded.size, \
               width=COALESCE(excluded.width, blob.width), \
               height=COALESCE(excluded.height, blob.height)",
        )?
        .execute(rusqlite::params![
            &hash[..],
            &cid[..],
            mime,
            bytes.len() as i64,
            w,
            h
        ])?;

        Ok(BlobMeta {
            sha256: hex::encode(hash),
            cipher_id: Some(hex::encode(cid)),
            mime: mime.to_string(),
            size: bytes.len() as i64,
            w,
            h,
        })
    }

    /// 查一条附件元信息。
    pub fn blob_meta(&self, hash: &[u8; 32]) -> Result<Option<BlobMeta>> {
        let mut st = self.conn().prepare_cached(
            "SELECT cipher_id, mime, size, width, height FROM blob WHERE plain_hash=?1",
        )?;
        match st.query_row([&hash[..]], |r| {
            Ok(BlobMeta {
                sha256: hex::encode(hash),
                cipher_id: Some(hex::encode(r.get::<_, Vec<u8>>(0)?)),
                mime: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                size: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                w: r.get(3)?,
                h: r.get(4)?,
            })
        }) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 清掉没人引用的附件。
    ///
    /// **先删文件再删表行**。反过来的话，删表成功、删文件失败就变成了一个
    /// 谁也不知道的孤儿文件——它不在任何索引里，只能靠人工去翻目录。
    pub fn gc_blobs(&self, blobs: &BlobStore) -> Result<usize> {
        let orphans: Vec<Vec<u8>> = {
            let mut st = self
                .conn()
                .prepare("SELECT plain_hash FROM blob WHERE refcount <= 0")?;
            let rows = st
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };

        let mut n = 0;
        for h in orphans {
            let Ok(hash) = <[u8; 32]>::try_from(h.as_slice()) else {
                continue;
            };
            blobs.remove(&hash)?;
            self.conn()
                .execute("DELETE FROM blob WHERE plain_hash=?1", [&h])?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MemoInput;
    use zhiyan_crypto::KEY_LEN;

    const T0: i64 = 1_754_380_320_000;

    fn dek() -> DataKey {
        DataKey::from_bytes([0x5Au8; KEY_LEN])
    }

    fn fixture() -> (Store, BlobStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let bs = BlobStore::new(dir.path());
        (crate::tests::store(), bs, dir)
    }

    #[test]
    fn 往返() {
        let (_s, bs, _d) = fixture();
        let bytes = b"pretend this is a jpeg \xff\xd8\xff";
        let (h, _cid) = bs.put(&dek(), bytes).unwrap();
        assert_eq!(bs.get(&dek(), &h).unwrap(), bytes);
    }

    #[test]
    fn 磁盘上没有明文() {
        let (_s, bs, dir) = fixture();
        let bytes = b"MAGIC_PLAINTEXT_MARKER_IN_A_BLOB";
        bs.put(&dek(), bytes).unwrap();

        let mut found = false;
        for entry in walk(dir.path()) {
            let data = fs::read(&entry).unwrap();
            assert!(
                !data.windows(bytes.len()).any(|w| w == bytes),
                "明文出现在 {entry:?}"
            );
            found = true;
        }
        assert!(found, "应该写出了文件");
    }

    #[test]
    fn 按哈希前两位分桶() {
        let (_s, bs, dir) = fixture();
        let (h, _) = bs.put(&dek(), b"x").unwrap();
        let hex = hex::encode(h);
        assert!(dir.path().join("blobs").join(&hex[..2]).join(&hex).exists());
    }

    #[test]
    fn 收敛加密下同一内容落到同一个位置() {
        let (_s, bs, _d) = fixture();
        let (h1, c1) = bs.put(&dek(), b"same bytes").unwrap();
        let (h2, c2) = bs.put(&dek(), b"same bytes").unwrap();
        assert_eq!((h1, c1), (h2, c2), "cipher_id 必须稳定，否则去重失效");
    }

    #[test]
    fn 换个用户同一内容的密文不同() {
        // 掺 DEK 的目的：阻断服务端的「确认文件存在」攻击
        let (_s, bs, _d) = fixture();
        let (_, c1) = bs.put(&dek(), b"same photo").unwrap();
        let (_, c2) = bs
            .put(&DataKey::from_bytes([0x11u8; KEY_LEN]), b"same photo")
            .unwrap();
        assert_ne!(c1, c2);
    }

    #[test]
    fn 密钥错了打不开() {
        let (_s, bs, _d) = fixture();
        let (h, _) = bs.put(&dek(), b"secret").unwrap();
        assert!(bs.get(&DataKey::from_bytes([0u8; KEY_LEN]), &h).is_err());
    }

    #[test]
    fn 篡改密文会被发现() {
        let (_s, bs, dir) = fixture();
        let (h, _) = bs.put(&dek(), b"important bytes").unwrap();
        let hex = hex::encode(h);
        let p = dir.path().join("blobs").join(&hex[..2]).join(&hex);
        let mut data = fs::read(&p).unwrap();
        let last = data.len() - 1;
        data[last] ^= 0x01;
        fs::write(&p, data).unwrap();
        assert!(bs.get(&dek(), &h).is_err());
    }

    #[test]
    fn 缩略图另用一把密钥且也是密文() {
        let (_s, bs, dir) = fixture();
        let (h, _) = bs.put(&dek(), b"original image").unwrap();
        let thumb = b"MAGIC_THUMB_BYTES";
        bs.put_thumb(&dek(), &h, thumb).unwrap();

        assert_eq!(
            bs.get_thumb(&dek(), &h).unwrap().as_deref(),
            Some(&thumb[..])
        );

        for entry in walk(&dir.path().join("cache")) {
            let data = fs::read(&entry).unwrap();
            assert!(
                !data.windows(thumb.len()).any(|w| w == thumb),
                "缩略图是明文的"
            );
        }
    }

    #[test]
    fn 没有缩略图时返回_none_而不是报错() {
        let (_s, bs, _d) = fixture();
        let (h, _) = bs.put(&dek(), b"no thumb yet").unwrap();
        assert!(bs.get_thumb(&dek(), &h).unwrap().is_none());
    }

    #[test]
    fn 元信息与引用计数() {
        let (mut s, bs, _d) = fixture();
        let meta = s
            .put_blob(
                &bs,
                &dek(),
                b"an image",
                "image/jpeg",
                Some(1600),
                Some(1200),
            )
            .unwrap();
        assert_eq!(meta.mime, "image/jpeg");
        assert_eq!(meta.size, 8);
        assert_eq!(meta.w, Some(1600));

        // 刚落盘还没人引用
        let rc = |s: &Store| -> i64 {
            s.conn()
                .query_row("SELECT refcount FROM blob", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(rc(&s), 0);

        let m = s
            .upsert_memo(
                &MemoInput {
                    text: "配了张图".into(),
                    blobs: vec![meta.clone()],
                    ..Default::default()
                },
                T0,
            )
            .unwrap();
        assert_eq!(rc(&s), 1);
        assert_eq!(m.blobs.len(), 1);
        assert_eq!(m.blobs[0].sha256, meta.sha256);

        // 把图从正文里去掉，引用计数要跟着回落
        s.upsert_memo(
            &MemoInput {
                id: Some(m.id),
                text: "图撤了".into(),
                ..Default::default()
            },
            T0 + 1,
        )
        .unwrap();
        assert_eq!(rc(&s), 0);
    }

    #[test]
    fn 引用不存在的附件报错而不是静默跳过() {
        // 静默跳过的话，正文里的图会永远打不开，而且没人知道是哪一步丢的
        let (mut s, _bs, _d) = fixture();
        let bogus = BlobMeta {
            sha256: hex::encode([0xAAu8; 32]),
            cipher_id: None,
            mime: "image/png".into(),
            size: 1,
            w: None,
            h: None,
        };
        assert!(matches!(
            s.upsert_memo(
                &MemoInput {
                    text: "假图".into(),
                    blobs: vec![bogus],
                    ..Default::default()
                },
                T0
            ),
            Err(DbError::NotFound)
        ));
    }

    #[test]
    fn 墓碑会释放附件() {
        let (mut s, bs, _d) = fixture();
        let meta = s
            .put_blob(&bs, &dek(), b"attached", "image/png", None, None)
            .unwrap();
        let m = s
            .upsert_memo(
                &MemoInput {
                    text: "带图的一条".into(),
                    blobs: vec![meta],
                    ..Default::default()
                },
                T0,
            )
            .unwrap();
        s.soft_delete(m.id, T0 + 1).unwrap();
        s.purge(m.id, T0 + 2).unwrap();

        assert_eq!(s.gc_blobs(&bs).unwrap(), 1);
        let n: i64 = s
            .conn()
            .query_row("SELECT count(*) FROM blob", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn 还有人引用的附件不会被清掉() {
        let (mut s, bs, _d) = fixture();
        let meta = s
            .put_blob(&bs, &dek(), b"shared", "image/png", None, None)
            .unwrap();
        for i in 0..2 {
            s.upsert_memo(
                &MemoInput {
                    text: format!("第{i}条都配了同一张图"),
                    blobs: vec![meta.clone()],
                    ..Default::default()
                },
                T0 + i,
            )
            .unwrap();
        }
        let rc: i64 = s
            .conn()
            .query_row("SELECT refcount FROM blob", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rc, 2, "两条记录引用同一张图");
        assert_eq!(s.gc_blobs(&bs).unwrap(), 0);

        let h = <[u8; 32]>::try_from(hex::decode(&meta.sha256).unwrap().as_slice()).unwrap();
        assert!(bs.contains(&h));
    }

    #[test]
    fn 删不存在的附件不报错() {
        let (_s, bs, _d) = fixture();
        bs.remove(&[0u8; 32]).unwrap();
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir(dir) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
