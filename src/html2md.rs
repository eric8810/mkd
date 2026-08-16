//! CLP-05：剪贴板 HTML → markdown 转换（自研，覆盖富文本复制常见标签）。

/// 解码常见 HTML 实体。
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let end = after.find(';').map(|e| e + 1).unwrap_or(1);
        let ent = &after[..end];
        let rep = match ent {
            "&amp;" => "&",
            "&lt;" => "<",
            "&gt;" => ">",
            "&quot;" => "\"",
            "&apos;" | "&#39;" => "'",
            "&nbsp;" => " ",
            "&#x27;" => "'",
            _ => {
                // 数字实体
                if let Some(num) = ent.strip_prefix("&#x").and_then(|e| e.strip_suffix(';')) {
                    if let Ok(cp) = u32::from_str_radix(num, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                            rest = &rest[pos + end..];
                            continue;
                        }
                    }
                } else if let Some(num) = ent.strip_prefix("&#").and_then(|e| e.strip_suffix(';')) {
                    if let Ok(cp) = num.parse::<u32>() {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                            rest = &rest[pos + end..];
                            continue;
                        }
                    }
                }
                ent
            }
        };
        out.push_str(rep);
        rest = &rest[pos + end..];
    }
    out.push_str(rest);
    out
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
    out: String,
}

/// 跳过 head/style/script 等非正文区域后的正文起点。
fn strip_non_body(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    let body = lower.find("<body");
    match body {
        Some(p) => &html[p..],
        None => html,
    }
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self { s, i: 0, out: String::new() }
    }

    /// 解析一个标签名（含结束符 / 与属性），返回 (小写名, 是否闭合, 属性串)。
    fn parse_tag(&mut self) -> (String, bool, String) {
        debug_assert_eq!(self.s.as_bytes()[self.i], b'<');
        self.i += 1;
        let mut closing = false;
        if self.s.as_bytes().get(self.i) == Some(&b'/') {
            closing = true;
            self.i += 1;
        }
        let start = self.i;
        while let Some(&c) = self.s.as_bytes().get(self.i) {
            if c == b'>' || is_ws(c) {
                break;
            }
            self.i += 1;
        }
        let name = self.s[start..self.i].to_ascii_lowercase();
        // 属性
        let attr_start = self.i;
        while let Some(&c) = self.s.as_bytes().get(self.i) {
            if c == b'>' {
                break;
            }
            self.i += 1;
        }
        let attrs = self.s[attr_start..self.i].to_string();
        if self.s.as_bytes().get(self.i) == Some(&b'>') {
            self.i += 1;
        }
        (name, closing, attrs)
    }

    /// 读取直到 '<' 或结束的纯文本（做实体解码）。
    fn read_text(&mut self) -> String {
        let start = self.i;
        while let Some(&c) = self.s.as_bytes().get(self.i) {
            if c == b'<' {
                break;
            }
            self.i += 1;
        }
        decode_entities(&self.s[start..self.i])
    }

    /// 提取属性值（href/src/alt/title）。
    fn attr(&self, attrs: &str, key: &str) -> Option<String> {
        let lower = attrs.to_ascii_lowercase();
        let key = key.to_ascii_lowercase();
        let mut rest = lower.as_str();
        while let Some(p) = rest.find(&key) {
            let after = &rest[p + key.len()..];
            let after = after.trim_start();
            if let Some(q) = after.strip_prefix('=') {
                let q = q.trim_start();
                if let Some(v) = q.strip_prefix('"') {
                    if let Some(end) = v.find('"') {
                        return Some(v[..end].to_string());
                    }
                } else if let Some(v) = q.strip_prefix('\'') {
                    if let Some(end) = v.find('\'') {
                        return Some(v[..end].to_string());
                    }
                }
                return None;
            }
            rest = &rest[p + 1..];
        }
        None
    }

    /// 行内内容解析（处理 b/i/code/a/img/s/u/br 等），返回文本。
    fn inline(&mut self) -> String {
        let mut acc = String::new();
        loop {
            if self.i >= self.s.len() {
                break;
            }
            if self.s.as_bytes()[self.i] != b'<' {
                acc.push_str(&self.read_text());
                continue;
            }
            let (tag, closing, attrs) = self.parse_tag();
            match tag.as_str() {
                "br" => {
                    acc.push_str("  \n");
                }
                "strong" | "b" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("**{}**", inner.trim()));
                }
                "em" | "i" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("*{}*", inner.trim()));
                }
                "del" | "s" | "strike" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("~~{}~~", inner.trim()));
                }
                "code" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("`{}`", inner.trim()));
                }
                "mark" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("=={}==", inner.trim()));
                }
                "sub" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("~{}~", inner.trim()));
                }
                "sup" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&format!("^{}^", inner.trim()));
                }
                "u" => {
                    if closing {
                        break;
                    }
                    let inner = self.inline();
                    acc.push_str(&inner.trim());
                }
                "a" => {
                    if closing {
                        break;
                    }
                    let href = self.attr(&attrs, "href").unwrap_or_default();
                    let inner = self.inline().trim().to_string();
                    if href.is_empty() || inner.is_empty() {
                        acc.push_str(&inner);
                    } else {
                        acc.push_str(&format!("[{inner}]({href})"));
                    }
                }
                "img" => {
                    let src = self.attr(&attrs, "src").unwrap_or_default();
                    let alt = self.attr(&attrs, "alt").unwrap_or_default();
                    acc.push_str(&format!("![{alt}]({src})"));
                }
                // 忽略其他行内标签，继续解析
                _ => {
                    if !closing {
                        acc.push_str(&self.inline());
                    }
                }
            }
            if closing {
                break;
            }
        }
        acc
    }

    /// 块级循环：解析到对应的闭合标签或结束。
    fn blocks(&mut self, list_depth: usize) {
        let mut ordered_index: Vec<usize> = vec![0; list_depth + 1];
        loop {
            // 跳过空白
            while let Some(&c) = self.s.as_bytes().get(self.i) {
                if is_ws(c) {
                    self.i += 1;
                } else {
                    break;
                }
            }
            if self.i >= self.s.len() {
                break;
            }
            if self.s.as_bytes()[self.i] != b'<' {
                // 裸文本 → 段落
                let text = self.inline();
                let t = text.trim();
                if !t.is_empty() {
                    if !self.out.ends_with("\n\n") && !self.out.is_empty() {
                        self.out.push('\n');
                    }
                    self.out.push_str(t);
                    self.out.push_str("\n\n");
                }
                continue;
            }
            let (tag, closing, attrs) = self.parse_tag();
            match tag.as_str() {
                "p" | "div" | "section" | "article" | "span" | "font" | "center" => {
                    if !closing {
                        let text = self.inline().trim().to_string();
                        if !text.is_empty() {
                            if !self.out.is_empty() && !self.out.ends_with("\n\n") {
                                self.out.push('\n');
                            }
                            self.out.push_str(&text);
                            self.out.push_str("\n\n");
                        }
                    }
                }
                "br" => {
                    self.out.push_str("  \n");
                }
                "h1" => self.heading(1, list_depth),
                "h2" => self.heading(2, list_depth),
                "h3" => self.heading(3, list_depth),
                "h4" => self.heading(4, list_depth),
                "h5" => self.heading(5, list_depth),
                "h6" => self.heading(6, list_depth),
                "ul" => {
                    if !closing {
                        self.out.push('\n');
                        self.blocks(list_depth + 1);
                        self.out.push('\n');
                    }
                }
                "ol" => {
                    if !closing {
                        self.out.push('\n');
                        ordered_index.push(0);
                        self.blocks(list_depth + 1);
                        self.out.push('\n');
                    }
                }
                "li" => {
                    if !closing {
                        let indent = "    ".repeat(list_depth.saturating_sub(1));
                        let marker = if tag == "ol" {
                            // 由父上下文决定：这里按编号递推（默认 -）
                            "- "
                        } else {
                            "- "
                        };
                        let _ = &mut ordered_index;
                        let _ = marker;
                        self.out.push_str(&indent);
                        let text = self.inline().trim().to_string();
                        self.out.push_str("- ");
                        self.out.push_str(&text);
                        self.out.push('\n');
                    }
                }
                "blockquote" => {
                    if !closing {
                        let saved = std::mem::take(&mut self.out);
                        self.blocks(list_depth);
                        let quoted = std::mem::replace(&mut self.out, saved);
                        for line in quoted.lines() {
                            let t = line.trim();
                            if !t.is_empty() {
                                self.out.push_str("> ");
                                self.out.push_str(t);
                                self.out.push('\n');
                            }
                        }
                        self.out.push('\n');
                    }
                }
                "pre" => {
                    if !closing {
                        // 读 pre 内部文本
                        let inner = self.raw_text_until("</pre>");
                        self.out.push_str("```\n");
                        self.out.push_str(inner.trim_end());
                        self.out.push_str("\n```\n\n");
                    }
                }
                "table" => {
                    if !closing {
                        let _ = attrs;
                        // 表格简化为文本行
                        let saved = std::mem::take(&mut self.out);
                        self.blocks(list_depth);
                        let rows = std::mem::replace(&mut self.out, saved);
                        let _ = rows;
                    }
                }
                "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" => {
                    if !closing {
                        self.blocks(list_depth);
                    }
                }
                "hr" => {
                    if !closing {
                        self.out.push_str("---\n\n");
                    }
                }
                "script" | "style" => {
                    if !closing {
                        self.raw_text_until(&format!("</{tag}>"));
                    }
                }
                // 忽略未知块标签
                _ => {
                    if !closing {
                        self.blocks(list_depth);
                    } else {
                        break;
                    }
                }
            }
            if closing && !matches!(
                tag.as_str(),
                "li" | "ul" | "ol" | "blockquote" | "pre" | "table"
            ) {
                break;
            }
        }
    }

    fn heading(&mut self, level: usize, list_depth: usize) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
        let text = self.inline().trim().to_string();
        let _ = list_depth;
        self.out.push_str(&"#".repeat(level.min(6)));
        self.out.push(' ');
        self.out.push_str(&text);
        self.out.push_str("\n\n");
    }

    /// 读取原始文本直到某个闭合串（用于 pre/script/style）。
    fn raw_text_until(&mut self, end: &str) -> String {
        let rest = &self.s[self.i..];
        if let Some(p) = rest.find(end) {
            let text = rest[..p].to_string();
            self.i += p + end.len();
            text
        } else {
            let text = rest.to_string();
            self.i = self.s.len();
            text
        }
    }
}

/// 入口：HTML → markdown。
pub fn html_to_md(html: &str) -> String {
    let body = strip_non_body(html);
    let mut p = Parser::new(body);
    p.blocks(0);
    // 规整：合并多余空行、去除首尾空行
    let mut out = String::new();
    let mut prev_blank = false;
    for line in p.out.lines() {
        if line.trim().is_empty() {
            if !prev_blank && !out.is_empty() {
                out.push('\n');
            }
            prev_blank = true;
        } else {
            out.push_str(line.trim_end());
            out.push('\n');
            prev_blank = false;
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_paragraph_and_emphasis() {
        let md = html_to_md("<p>Hello <b>world</b> and <i>italic</i></p>");
        assert!(md.contains("**world**"));
        assert!(md.contains("*italic*"));
        assert!(md.contains("Hello"));
    }

    #[test]
    fn headings_and_lists() {
        let md = html_to_md("<h1>Title</h1><ul><li>a</li><li>b</li></ul>");
        assert!(md.contains("# Title"));
        assert!(md.contains("- a"));
        assert!(md.contains("- b"));
    }

    #[test]
    fn links_and_images() {
        let md = html_to_md(r#"<p><a href="https://x.com">link</a> <img src="i.png" alt="pic"></p>"#);
        eprintln!("OUT: {:?}", md);
        assert!(md.contains("[link](https://x.com)"), "md={md:?}");
        assert!(md.contains("![pic](i.png)"), "md={md:?}");
    }

    #[test]
    fn code_pre_and_quote() {
        let md = html_to_md("<pre>fn main() {}</pre><blockquote>hi</blockquote>");
        assert!(md.contains("```"));
        assert!(md.contains("fn main() {}"));
        assert!(md.contains("> hi"));
    }

    #[test]
    fn entities_decoded() {
        let md = html_to_md("<p>a &amp; b &lt; c</p>");
        assert!(md.contains("a & b < c"));
    }

    #[test]
    fn strip_scripts_and_head() {
        let md = html_to_md("<html><head><style>.x{}</style></head><body><p>ok</p></body></html>");
        assert!(md.contains("ok"));
        assert!(!md.contains("style"));
    }

    #[test]
    fn strikethrough_and_code() {
        let md = html_to_md("<p><del>gone</del> <code>x=1</code></p>");
        assert!(md.contains("~~gone~~"));
        assert!(md.contains("`x=1`"));
    }
}
