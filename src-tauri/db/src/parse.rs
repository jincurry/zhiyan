//! 从正文里解析标签与行内引用（§9.6：`upsert_memo` 内部解析标签/引用，写三张表）。
//!
//! ## 为什么要在 Rust 侧再实现一遍
//!
//! 前端 `util.js` 里已有一份（渲染高亮要用）。两份实现是**有意的重复**，但必须
//! 逐字对齐：前端那份决定用户看到什么被高亮成标签，这份决定 `memo_tag` 表里
//! 存了什么。两者分叉的表现是「界面上高亮着的标签，点侧栏筛不出来」——
//! 一个很难查的 bug，所以下面每条规则都照抄 `util.js` 的正则，并注明出处。
//!
//! 换成让前端把解析结果传过来也不行：那样正文与标签的一致性就由前端保证了，
//! 而正文可能被同步、迁移、导入等根本不经过前端的路径改写。

/// 标签正则（`util.js` 的 `TAGRE`）：
/// `/(^|[\s(（])#([^\s#，。、；：!？"'()（）[\]]+)/g`
///
/// 注意 JS 那份**没有** `m` 标志，所以 `^` 只匹配整串开头，不是每行开头。
fn is_tag_boundary(prev: Option<char>) -> bool {
    match prev {
        None => true, // 串首
        Some(c) => c.is_whitespace() || c == '(' || c == '（',
    }
}

/// 标签正文的终止字符集，与 `util.js` 的字符类逐字对应。
fn ends_tag(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '#' | '，'
                | '。'
                | '、'
                | '；'
                | '：'
                | '!'
                | '？'
                | '"'
                | '\''
                | '('
                | ')'
                | '（'
                | '）'
                | '['
                | ']'
        )
}

/// 解析标签，保序去重。
///
/// `/` 不在终止集里，所以 `#读书/认知` 是一个两级标签而非两个——这是 §2.2
/// 「两级标签」的实现基础。
pub fn parse_tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] != '#' {
            i += 1;
            continue;
        }
        if !is_tag_boundary(i.checked_sub(1).map(|p| chars[p])) {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < chars.len() && !ends_tag(chars[end]) {
            end += 1;
        }
        if end == start {
            // 光一个 `#`（Markdown 标题）不是标签
            i += 1;
            continue;
        }
        let tag: String = chars[start..end].iter().collect();
        if !out.contains(&tag) {
            out.push(tag);
        }
        // JS 的 lastIndex 会跳过整个匹配
        i = end;
    }
    out
}

/// 行内引用正则（`util.js` 的 `REFRE`）：`/\[\[([^\]^\n]*)\^([0-9a-fA-F-]+)\]\]/g`
///
/// 返回被引用的 id 字符串，保序去重。**不在这里解析成 `Uuid`**——正文里可能残留
/// 迁移前的旧格式（§7.3 的 `int → UUIDv4`），解析失败应当被跳过而不是让整条保存失败。
pub fn parse_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0usize;

    while i + 4 < n {
        if !(chars[i] == '[' && chars[i + 1] == '[') {
            i += 1;
            continue;
        }
        // label：`[^\]^\n]*`
        let mut j = i + 2;
        while j < n && !matches!(chars[j], ']' | '^' | '\n') {
            j += 1;
        }
        if j >= n || chars[j] != '^' {
            i += 1;
            continue;
        }
        // id：`[0-9a-fA-F-]+`
        let id_start = j + 1;
        let mut k = id_start;
        while k < n && (chars[k].is_ascii_hexdigit() || chars[k] == '-') {
            k += 1;
        }
        if k == id_start || k + 1 >= n || chars[k] != ']' || chars[k + 1] != ']' {
            i += 1;
            continue;
        }
        let id: String = chars[id_start..k].iter().collect();
        if !out.contains(&id) {
            out.push(id);
        }
        i = k + 2;
    }
    out
}

/// 去掉 Markdown 标记留纯文本（`util.js` 的 `plain`）。字数统计用。
///
/// **刻意写成和 JS 一样的五趟顺序替换**，而不是合成一趟扫描。
/// 合成一趟看着更快，但趟与趟之间是有依赖的：`>` 在第四趟被去掉之后，
/// `> - 列表项` 才会在第五趟被认成列表项。一趟扫描下这类顺序关系极难摆对，
/// 而摆错的代价是两端字数对不上——一个没人会去查的数字差。
///
/// 这里跑的是几十到几百字的一条笔记，五趟和一趟的差别测不出来。
fn plain(text: &str) -> String {
    let s = replace_refs(text);
    let s = strip_tags(&s);
    let s = strip_links(&s);
    let s: String = s
        .chars()
        .filter(|c| !matches!(c, '*' | '~' | '=' | '`' | '>' | '#'))
        .collect();
    let s = strip_list_markers(&s);
    s.trim().to_string()
}

/// 第一趟：`[[摘要^id]]` → `「摘要」`。
fn replace_refs(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < n {
        if i + 1 < n && chars[i] == '[' && chars[i + 1] == '[' {
            let mut j = i + 2;
            while j < n && !matches!(chars[j], ']' | '^' | '\n') {
                j += 1;
            }
            if j < n && chars[j] == '^' {
                let id_start = j + 1;
                let mut k = id_start;
                while k < n && (chars[k].is_ascii_hexdigit() || chars[k] == '-') {
                    k += 1;
                }
                if k > id_start && k + 1 < n && chars[k] == ']' && chars[k + 1] == ']' {
                    out.push('「');
                    out.extend(&chars[i + 2..j]);
                    out.push('」');
                    i = k + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 第二趟：`#标签` 整个去掉，只留前面那个分隔符（JS 的 `.replace(TAGRE, '$1')`）。
///
/// 「标签不计入字数」这个取舍未必是我会自己定的，但**必须与前端一致**：
/// 时间脊的节点大小由前端的 `countChars` 算，统计面板的累计字数由这里算。
fn strip_tags(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < n {
        if chars[i] == '#' && is_tag_boundary(i.checked_sub(1).map(|p| chars[p])) {
            let mut j = i + 1;
            while j < n && !ends_tag(chars[j]) {
                j += 1;
            }
            if j > i + 1 {
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 第三趟：`[文字](链接)` → `文字`。
fn strip_links(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;

    while i < n {
        if chars[i] == '[' {
            // `[^\]]*` 不跨过 `]`
            let close = (i + 1..n).take_while(|&j| chars[j] != ']').count() + i + 1;
            let opens_paren = close + 1 < n && chars[close] == ']' && chars[close + 1] == '(';
            // `[^)]*` 不跨过 `)`
            let paren = if opens_paren {
                (close + 2..n).find(|&j| chars[j] == ')')
            } else {
                None
            };
            if let Some(paren) = paren {
                out.extend(&chars[i + 1..close]);
                i = paren + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 第五趟：行首的 `- ` / `+ ` 列表符号（JS 的 `/^\s*[-+]\s+/gm`）。
fn strip_list_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let rest = line.trim_start_matches([' ', '\t', '\r']);
        let stripped = rest
            .strip_prefix(['-', '+'])
            .filter(|after| after.starts_with([' ', '\t', '\r']))
            .map(|after| after.trim_start_matches([' ', '\t', '\r']));
        match stripped {
            Some(body) => out.push_str(body),
            None => out.push_str(line),
        }
    }
    out
}

/// 字数（`util.js` 的 `countChars`）：**中文按字、英文按词**。
///
/// 纯按字符数会让一段英文的计数虚高。时间脊的节点大小由它驱动，
/// 虚高会让「写了几句英文的那天」在脊上显得格外重。
pub fn count_chars(text: &str) -> i64 {
    let t = plain(text);
    let mut n = 0i64;
    let mut in_word = false;
    for ch in t.chars() {
        if ch.is_ascii_alphanumeric() {
            if !in_word {
                n += 1;
                in_word = true;
            }
        } else {
            in_word = false;
            if !ch.is_whitespace() {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 标签基本形() {
        assert_eq!(parse_tags("今天读了 #读书 和 #产品"), vec!["读书", "产品"]);
        assert_eq!(parse_tags("#串首也算"), vec!["串首也算"]);
    }

    #[test]
    fn 两级标签不被斜杠切断() {
        // §2.2 的两级标签全靠这条
        assert_eq!(parse_tags("#读书/认知 很有意思"), vec!["读书/认知"]);
    }

    #[test]
    fn 中文标点终止标签() {
        assert_eq!(parse_tags("记一笔 #工作，然后休息"), vec!["工作"]);
        assert_eq!(parse_tags("（#括号里的）"), vec!["括号里的"]);
    }

    #[test]
    fn 词中间的井号不是标签() {
        // C# 里的 # 前面是字母，不构成标签边界
        assert_eq!(parse_tags("用 C#写的"), Vec::<String>::new());
        assert_eq!(parse_tags("a#b"), Vec::<String>::new());
    }

    #[test]
    fn 光一个井号不是标签() {
        assert_eq!(parse_tags("# 一级标题"), Vec::<String>::new());
        assert_eq!(parse_tags("## 二级"), Vec::<String>::new());
    }

    #[test]
    fn 标签去重且保序() {
        assert_eq!(parse_tags("#a #b #a #c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn 引用解析() {
        let t = "见 [[那天的想法^3f9a1c2e-0000-4000-8000-000000000001]] 与 [[另一条^abc123]]";
        assert_eq!(
            parse_refs(t),
            vec!["3f9a1c2e-0000-4000-8000-000000000001", "abc123"]
        );
    }

    #[test]
    fn 引用空标签也认() {
        assert_eq!(parse_refs("[[^dead-beef]]"), vec!["dead-beef"]);
    }

    #[test]
    fn 残缺引用被跳过而不是崩溃() {
        for t in [
            "[[没有脱字符]]",
            "[[label^]]",
            "[[label^xyz]]", // xyz 不是 hex
            "[[label^abc]",
            "[[",
            "[[a^b",
        ] {
            assert!(parse_refs(t).is_empty(), "{t:?} 不该解析出引用");
        }
    }

    #[test]
    fn 字数中文按字英文按词() {
        assert_eq!(count_chars("今天很好"), 4);
        assert_eq!(count_chars("hello world"), 2);
        assert_eq!(count_chars("今天 hello 很好"), 5);
    }

    #[test]
    fn 字数忽略_markdown_符号() {
        assert_eq!(count_chars("**加粗**"), 2);
        assert_eq!(count_chars("[链接](https://example.com)"), 2);
    }

    #[test]
    fn 字数不含标签且与前端口径一致() {
        // util.js 的 plain() 把整个 `#标签` 连同前导分隔符一起去掉
        assert_eq!(count_chars("今天很好 #日记"), 4);
        assert_eq!(count_chars("#只有标签"), 0);
    }

    #[test]
    fn 字数把引用算成被引的摘要() {
        // plain() 把 [[摘要^id]] 换成「摘要」，id 不计入
        assert_eq!(count_chars("[[那天^abc]]"), 4); // 「 那 天 」
    }
}
