//! 行级所见即所得编辑器。
//!
//! 源码按行拆分，每行用自写的行内解析器（pulldown-cmark 不暴露字节偏移，
//! 因此无法用于「渲染文本 ↔ 源码」映射）产出：
//!   - 显示文本（去标记后的渲染文本）
//!   - 样式段
//!   - 显示字符 → 源码字节偏移的映射
//! 编辑操作全部在行级源码上进行，即时重新解析 → 所见即所得。

use std::ops::Range;

use gpui::{
    App, Bounds, Element, ElementId, ElementInputHandler, FocusHandle, Font, FontStyle, FontWeight,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, PaintQuad, Pixels, Point,
    ShapedLine, Style, StrikethroughStyle, TextRun, UnderlineStyle, Window, fill, point, px, rgba,
    relative, size,
};

use crate::theme::Theme;

// ---------------------------------------------------------------------------
// 行内解析
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum TextStyle {
    Plain,
    Bold,
    Italic,
    BoldItalic,
    Code,
    Strike,
    Mark,
    Link,
}

#[derive(Clone, Debug)]
struct Seg {
    text: String,
    style: TextStyle,
}

/// 行级样式。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineStyle {
    Plain,
    Heading(u8),
    Quote,
    Bullet,
    Ordered,
    Fence,
    CodeLine,
    Rule,
}

#[derive(Clone, Debug)]
pub struct ParsedLine {
    /// 渲染显示文本（去标记）。
    pub display: String,
    /// 显示文本每个字符对应的源码字节偏移（单调递增）。
    pub map: Vec<usize>,
    /// 行首标记长度（如 `# `、`- `、`> `），这些源码不进入显示文本。
    pub prefix_len: usize,
    pub line_style: LineStyle,
    segs: Vec<Seg>,
}

/// 从源码行解析出显示文本与映射。标记不跨行。
pub fn parse_line(src: &str, in_fence: bool) -> ParsedLine {
    let (line_style, prefix_len) = detect_line_style(src);
    let body = &src[prefix_len..];

    if in_fence || line_style == LineStyle::Fence {
        let display = if line_style == LineStyle::Fence {
            body.trim_start_matches('`').trim_start_matches('~').to_string()
        } else {
            body.to_string()
        };
        let map: Vec<usize> = display
            .char_indices()
            .map(|(i, _)| prefix_len + i)
            .collect();
        return ParsedLine {
            display: display.clone(),
            map,
            prefix_len,
            line_style: if in_fence {
                LineStyle::CodeLine
            } else {
                line_style
            },
            // 关键：run 长度必须与 display 完全一致，否则 shape_line 越界
            segs: vec![Seg {
                text: display,
                style: TextStyle::Code,
            }],
        };
    }

    let mut display = String::new();
    let mut map = Vec::new();
    let mut segs: Vec<Seg> = Vec::new();
    parse_inline_body(body, prefix_len, src, &mut display, &mut map, &mut segs);

    ParsedLine {
        display,
        map,
        prefix_len,
        line_style,
        segs,
    }
}

fn detect_line_style(src: &str) -> (LineStyle, usize) {
    let s = src.as_bytes();
    if s.len() >= 3 && s[0] == b'`' && s[1] == b'`' && s[2] == b'`' {
        return (LineStyle::Fence, 0);
    }
    if s.len() >= 3 && s[0] == b'~' && s[1] == b'~' && s[2] == b'~' {
        return (LineStyle::Fence, 0);
    }
    let t = src.trim_start();
    if t.starts_with("---") || t.starts_with("***") || t.starts_with("___") {
        return (LineStyle::Rule, 0);
    }
    // 标题
    let n = src.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&n) {
        let after = &src[n..];
        if after.is_empty() || after.starts_with(' ') {
            let extra = if after.is_empty() { 0 } else { 1 };
            return (LineStyle::Heading(n as u8), n + extra);
        }
    }
    // 引用
    if src.starts_with('>') {
        let after = &src[1..];
        let plen = 1 + if after.starts_with(' ') { 1 } else { 0 };
        return (LineStyle::Quote, plen);
    }
    // 无序列表
    for marker in ["- ", "* ", "+ "] {
        if src.starts_with(marker) {
            return (LineStyle::Bullet, marker.len());
        }
    }
    // 有序列表
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
        return (LineStyle::Ordered, i + 2);
    }
    (LineStyle::Plain, 0)
}

fn parse_inline_body(
    src: &str,
    base: usize,
    full_src: &str,
    display: &mut String,
    map: &mut Vec<usize>,
    segs: &mut Vec<Seg>,
) {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut plain_start: Option<usize> = Some(0);

    macro_rules! flush_plain {
        ($style:expr, $from:expr, $to:expr) => {{
            let text = &full_src[base + $from..base + $to];
            for (j, c) in text.char_indices() {
                display.push(c);
                map.push(base + $from + j);
            }
            if !text.is_empty() {
                segs.push(Seg {
                    text: text.to_string(),
                    style: $style,
                });
            }
        }};
    }

    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        let open_len = if c == b'*'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'*'
            && bytes[i + 2] == b'*'
        {
            3
        } else if c == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            2
        } else if c == b'`' {
            1
        } else if c == b'~' && i + 1 < bytes.len() && bytes[i + 1] == b'~' {
            2
        } else if c == b'=' && i + 1 < bytes.len() && bytes[i + 1] == b'=' {
            2
        } else if c == b'*' {
            1
        } else if c == b'[' {
            if let Some((text_len, total)) = find_link(&src[i..]) {
                if let Some(ps) = plain_start.take() {
                    flush_plain!(TextStyle::Plain, ps, i);
                }
                let inner = &src[i + 1..i + 1 + text_len];
                parse_inline_body(inner, base + i + 1, full_src, display, map, segs);
                mark_last_link(segs);
                i += total;
                plain_start = Some(i);
                continue;
            }
            0
        } else {
            0
        };

        if open_len > 0 {
            if let Some(rel) = find_closer(&src[i + open_len..], c, open_len) {
                if let Some(ps) = plain_start.take() {
                    flush_plain!(TextStyle::Plain, ps, i);
                }
                let inner = &src[i + open_len..i + open_len + rel];
                let style = match (c, open_len) {
                    (b'*', 3) => TextStyle::BoldItalic,
                    (b'*', 2) => TextStyle::Bold,
                    (b'*', 1) => TextStyle::Italic,
                    (b'`', _) => TextStyle::Code,
                    (b'~', 2) => TextStyle::Strike,
                    (b'=', 2) => TextStyle::Mark,
                    _ => TextStyle::Plain,
                };
                let mut inner_display = String::new();
                let mut inner_map = Vec::new();
                let mut inner_segs = Vec::new();
                parse_inline_body(
                    inner,
                    base + i + open_len,
                    full_src,
                    &mut inner_display,
                    &mut inner_map,
                    &mut inner_segs,
                );
                let has_italic = inner_segs.iter().any(|s| s.style == TextStyle::Italic);
                for s in &mut inner_segs {
                    if s.style == TextStyle::Plain {
                        s.style = if has_italic && style == TextStyle::Bold {
                            TextStyle::BoldItalic
                        } else {
                            style
                        };
                    }
                }
                display.push_str(&inner_display);
                map.extend_from_slice(&inner_map);
                segs.extend(inner_segs);
                i += open_len + rel + open_len;
                plain_start = Some(i);
                continue;
            }
        }
        i += 1;
    }
    if let Some(ps) = plain_start.take() {
        flush_plain!(TextStyle::Plain, ps, src.len());
    }
}

/// 把链接文本段标记为 Link 样式。
fn mark_last_link(segs: &mut Vec<Seg>) {
    for s in segs.iter_mut().rev() {
        if s.style == TextStyle::Plain {
            s.style = TextStyle::Link;
        } else {
            break;
        }
    }
}

/// 找 `[text](url)` 链接，返回 (text 长度, 总长度)。
fn find_link(s: &str) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && bytes[i] != b']' {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b']' {
        return None;
    }
    let text_len = i - 1;
    let after = &s[i + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let rest = &after[1..];
    let end = rest.find(')')?;
    Some((text_len, i + 1 + 1 + end + 1))
}

/// 找与开标记配对的关闭标记相对偏移。
fn find_closer(s: &str, delim: u8, open_len: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == delim {
            if open_len == 3 {
                if i + 2 < bytes.len() && bytes[i + 1] == delim && bytes[i + 2] == delim {
                    return Some(i);
                }
            } else if open_len == 2 {
                if i + 1 < bytes.len() && bytes[i + 1] == delim {
                    return Some(i);
                }
            } else if delim == b'*' {
                let prev_is_star = i > 0 && bytes[i - 1] == b'*';
                let next_is_star = i + 1 < bytes.len() && bytes[i + 1] == b'*';
                if !prev_is_star && !next_is_star {
                    return Some(i);
                }
            } else {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// 编辑器状态
// ---------------------------------------------------------------------------

pub struct Editor {
    pub lines: Vec<String>,
    pub cursor_line: usize,
    pub cursor_col: usize, // 源码字节偏移（行内）
    pub sel_start: Option<(usize, usize)>,
    pub dirty: bool,
    pub marked: Option<(usize, usize)>, // 行内 IME 组合区间（源码偏移）
    pub line_height: f32,
    pub font_size: f32,
    pub last_shapes: Vec<Option<ShapedLine>>,
    pub last_bounds: Option<Bounds<Pixels>>,
}

impl Editor {
    pub fn new(source: &str) -> Self {
        let lines: Vec<String> = source.split('\n').map(|l| l.to_string()).collect();
        Editor {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            sel_start: None,
            dirty: false,
            marked: None,
            line_height: 22.0,
            font_size: 15.0,
            last_shapes: Vec::new(),
            last_bounds: None,
        }
    }

    pub fn to_source(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn line_len(&self, i: usize) -> usize {
        self.line(i).len()
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend {
            self.sel_start = None;
        } else if self.sel_start.is_none() {
            self.sel_start = Some((self.cursor_line, self.cursor_col));
        }
        if self.cursor_col > 0 {
            self.cursor_col = self.prev_boundary(self.cursor_line, self.cursor_col);
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.line_len(self.cursor_line);
        }
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend {
            self.sel_start = None;
        } else if self.sel_start.is_none() {
            self.sel_start = Some((self.cursor_line, self.cursor_col));
        }
        let len = self.line_len(self.cursor_line);
        if self.cursor_col < len {
            self.cursor_col = self.next_boundary(self.cursor_line, self.cursor_col);
        } else if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
    }

    pub fn move_up(&mut self, extend: bool) {
        if !extend {
            self.sel_start = None;
        } else if self.sel_start.is_none() {
            self.sel_start = Some((self.cursor_line, self.cursor_col));
        }
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
        } else {
            self.cursor_col = 0;
        }
    }

    pub fn move_down(&mut self, extend: bool) {
        if !extend {
            self.sel_start = None;
        } else if self.sel_start.is_none() {
            self.sel_start = Some((self.cursor_line, self.cursor_col));
        }
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));
        } else {
            self.cursor_col = self.line_len(self.cursor_line);
        }
    }

    pub fn move_home(&mut self, extend: bool) {
        if !extend {
            self.sel_start = None;
        } else if self.sel_start.is_none() {
            self.sel_start = Some((self.cursor_line, self.cursor_col));
        }
        self.cursor_col = 0;
    }

    pub fn move_end(&mut self, extend: bool) {
        if !extend {
            self.sel_start = None;
        } else if self.sel_start.is_none() {
            self.sel_start = Some((self.cursor_line, self.cursor_col));
        }
        self.cursor_col = self.line_len(self.cursor_line);
    }

    fn prev_boundary(&self, line: usize, col: usize) -> usize {
        let s = self.line(line);
        s[..col]
            .char_indices()
            .rev()
            .next()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0)
    }

    fn next_boundary(&self, line: usize, col: usize) -> usize {
        let s = self.line(line);
        match s[col..].chars().next() {
            Some(c) => col + c.len_utf8(),
            None => col,
        }
    }

    fn selection_bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        let (sl, sc) = self.sel_start?;
        let cur = (self.cursor_line, self.cursor_col);
        if (sl, sc) == cur {
            return None;
        }
        Some(if (sl, sc) < cur { ((sl, sc), cur) } else { (cur, (sl, sc)) })
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((sl, sc), (el, ec)) = self.selection_bounds()?;
        if sl == el {
            return Some(self.lines[sl][sc..ec].to_string());
        }
        let mut out = String::new();
        out.push_str(&self.lines[sl][sc..]);
        for l in sl + 1..el {
            out.push('\n');
            out.push_str(&self.lines[l]);
        }
        out.push('\n');
        out.push_str(&self.lines[el][..ec]);
        Some(out)
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(((sl, sc), (el, ec))) = self.selection_bounds() else {
            return false;
        };
        if sl == el {
            self.lines[sl].replace_range(sc..ec, "");
        } else {
            self.lines[sl] = self.lines[sl][..sc].to_string() + &self.lines[el][ec..];
            for _ in sl + 1..=el {
                self.lines.remove(sl + 1);
            }
        }
        self.cursor_line = sl;
        self.cursor_col = sc;
        self.sel_start = None;
        true
    }

    /// 在光标处插入文本（可含换行）。
    pub fn insert_text(&mut self, text: &str) {
        if self.sel_start.is_some() {
            self.delete_selection();
        }
        self.marked = None;
        let (line, col) = (self.cursor_line, self.cursor_col);
        let head = self.lines[line][..col].to_string();
        let tail = self.lines[line][col..].to_string();
        let nl_count = text.matches('\n').count();
        if nl_count == 0 {
            self.lines[line] = head + text + &tail;
            self.cursor_col = col + text.len();
        } else {
            let mut parts: Vec<&str> = text.split('\n').collect();
            let mut new_lines = Vec::with_capacity(nl_count + 1);
            new_lines.push(head + parts.remove(0));
            let last = parts.pop().unwrap_or("");
            for p in parts {
                new_lines.push(p.to_string());
            }
            new_lines.push(last.to_string() + &tail);
            self.lines.splice(line..=line, new_lines);
            self.cursor_line = line + nl_count;
            self.cursor_col = last.len();
        }
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.sel_start.is_some() {
            self.dirty = self.delete_selection();
            return;
        }
        self.marked = None;
        let (line, col) = (self.cursor_line, self.cursor_col);
        if col > 0 {
            let prev = self.prev_boundary(line, col);
            self.lines[line].replace_range(prev..col, "");
            self.cursor_col = prev;
            self.dirty = true;
        } else if line > 0 {
            let prev_len = self.line_len(line - 1);
            let joined = self.lines[line - 1].clone() + &self.lines[line];
            self.lines.remove(line);
            self.lines[line - 1] = joined;
            self.cursor_line = line - 1;
            self.cursor_col = prev_len;
            self.dirty = true;
        }
    }

    pub fn delete(&mut self) {
        if self.sel_start.is_some() {
            self.dirty = self.delete_selection();
            return;
        }
        self.marked = None;
        let (line, col) = (self.cursor_line, self.cursor_col);
        let len = self.line_len(line);
        if col < len {
            let next = self.next_boundary(line, col);
            self.lines[line].replace_range(col..next, "");
            self.dirty = true;
        } else if line + 1 < self.lines.len() {
            let tail = self.lines[line + 1].clone();
            self.lines.remove(line + 1);
            self.lines[line].push_str(&tail);
            self.dirty = true;
        }
    }

    pub fn insert_newline(&mut self) {
        self.insert_text("\n");
    }

    pub fn insert_tab(&mut self) {
        self.insert_text("    ");
    }

    pub fn select_all(&mut self) {
        let last = self.lines.len().saturating_sub(1);
        self.sel_start = Some((0, 0));
        self.cursor_line = last;
        self.cursor_col = self.line_len(last);
    }

    /// 光标对应的显示列（用于绘制光标位置）。
    pub fn cursor_display_col(&self, parsed: &ParsedLine) -> usize {
        let col = self.cursor_col.min(self.line(self.cursor_line).len());
        if col <= parsed.prefix_len {
            return 0;
        }
        // 显示位置 = 源码中起点 < col 的被显示字符个数
        let target = col;
        let mut lo = 0usize;
        let mut hi = parsed.map.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if parsed.map[mid] < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// 显示列 → 源码字节偏移（行内）。
    pub fn source_col_for_display(&self, line: usize, display_col: usize) -> usize {
        let parsed = parse_line(self.line(line), false);
        let d = display_col;
        if d == 0 {
            return parsed.prefix_len;
        }
        if d >= parsed.map.len() {
            return self.line(line).len();
        }
        parsed.map[d]
    }

    // ---- UTF-16 全局坐标映射（IME / 剪贴板接口用） ----

    /// 全局 UTF-16 偏移 → (line, utf8 col)。
    pub fn pos_from_utf16(&self, mut utf16: usize) -> (usize, usize) {
        for (i, l) in self.lines.iter().enumerate() {
            let l16 = l.encode_utf16().count();
            if utf16 <= l16 {
                return (i, utf16_to_utf8(l, utf16));
            }
            utf16 -= l16 + 1; // include '\n'
        }
        let last = self.lines.len().saturating_sub(1);
        (last, self.lines.get(last).map(|l| l.len()).unwrap_or(0))
    }

    /// (line, utf8 col) → 全局 UTF-16 偏移。
    pub fn utf16_from_pos(&self, line: usize, col: usize) -> usize {
        let mut off = 0usize;
        for (i, l) in self.lines.iter().enumerate() {
            if i == line {
                return off + utf8_to_utf16(l, col.min(l.len()));
            }
            off += l.encode_utf16().count() + 1;
        }
        off
    }

    /// 删除全局 UTF-16 范围（IME / 平台输入用），光标移到范围起点。
    pub fn replace_utf16_range(&mut self, r: Range<usize>) {
        let (sl, sc) = self.pos_from_utf16(r.start);
        let (el, ec) = self.pos_from_utf16(r.end);
        if sl == el {
            if ec > sc {
                self.lines[sl].replace_range(sc..ec, "");
            }
        } else {
            self.lines[sl] = self.lines[sl][..sc].to_string() + &self.lines[el][ec..];
            for _ in sl + 1..=el {
                self.lines.remove(sl + 1);
            }
        }
        self.cursor_line = sl;
        self.cursor_col = sc;
        self.sel_start = None;
    }

    /// 屏幕点 → (line, 显示列)，依赖 last_bounds/last_shapes。
    pub fn pos_for_point(&self, p: Point<Pixels>) -> Option<(usize, usize)> {
        let bounds = self.last_bounds?;
        if p.y < bounds.top() {
            return Some((0, 0));
        }
        let line = ((p.y - bounds.top()).to_f64() / self.line_height as f64) as usize;
        let line = line.min(self.lines.len().saturating_sub(1));
        let shape = self.last_shapes.get(line)?.as_ref()?;
        let x = (p.x - bounds.left()).to_f64() as f32;
        let ix = shape.index_for_x(px(x))?;
        let display_col = ix.min(shape.len());
        Some((line, display_col))
    }

    /// 行内 utf8 范围 → 屏幕矩形（IME 候选窗定位用）。
    pub fn bounds_for_range_utf8(
        &self,
        line: usize,
        start: usize,
        end: usize,
    ) -> Option<Bounds<Pixels>> {
        let bounds = self.last_bounds?;
        let shape = self.last_shapes.get(line)?.as_ref()?;
        let parsed = parse_line(self.line(line), false);
        let ds = self.display_col_for_source(&parsed, start);
        let de = self.display_col_for_source(&parsed, end);
        let top = bounds.top() + px(line as f32 * self.line_height);
        let bottom = top + px(self.line_height);
        Some(Bounds::from_corners(
            point(
                bounds.left() + shape.x_for_index(ds.min(shape.len())),
                top,
            ),
            point(
                bounds.left() + shape.x_for_index(de.min(shape.len())),
                bottom,
            ),
        ))
    }

    /// 源码偏移 → 显示列（单调映射，光标在 src_col 之前）。
    fn display_col_for_source(&self, parsed: &ParsedLine, src_col: usize) -> usize {
        if src_col <= parsed.prefix_len {
            return 0;
        }
        let mut lo = 0usize;
        let mut hi = parsed.map.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if parsed.map[mid] < src_col {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

fn utf8_to_utf16(s: &str, col: usize) -> usize {
    s[..col].encode_utf16().count()
}

fn utf16_to_utf8(s: &str, utf16: usize) -> usize {
    let mut count = 0usize;
    for (i, c) in s.char_indices() {
        if count >= utf16 {
            return i;
        }
        count += c.len_utf16();
    }
    s.len()
}

// ---------------------------------------------------------------------------
// 编辑器渲染元素
// ---------------------------------------------------------------------------

/// 渲染编辑器。`V` 是持有 `Editor` 的视图类型。
pub struct EditorElement<V> {
    pub input: gpui::Entity<V>,
}

impl<V: 'static + EditorSource + gpui::EntityInputHandler> IntoElement for EditorElement<V> {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct PrepaintState {
    pub lines: Vec<ShapedLine>,
    pub caret: Option<PaintQuad>,
    pub selection: Option<PaintQuad>,
}

/// 由视图提供：从视图中读出/写入编辑器状态。
pub trait EditorSource {
    fn editor(&self) -> &Editor;
    fn editor_mut(&mut self) -> &mut Editor;
    fn focus_handle(&self) -> FocusHandle;
}

impl<V> Element for EditorElement<V>
where
    V: 'static + EditorSource + gpui::EntityInputHandler,
{
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let editor = self.input.read(cx).editor();
        style.size.height = px(editor.lines.len() as f32 * editor.line_height).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let t = Theme::light();
        // 快照编辑器状态
        let (lines, cursor_line, cursor_col, sel_start, marked, font_size, line_height_f) = {
            let view = self.input.read(cx);
            let e = view.editor();
            (
                e.lines.clone(),
                e.cursor_line,
                e.cursor_col,
                e.sel_start,
                e.marked,
                e.font_size,
                e.line_height,
            )
        };
        let line_height = px(line_height_f);

        let mut shapes = Vec::with_capacity(lines.len());
        let mut in_fence = false;
        let mut y = bounds.top();
        let mut caret: Option<PaintQuad> = None;

        for (li, src) in lines.iter().enumerate() {
            let parsed = parse_line(src, in_fence);
            if parsed.line_style == LineStyle::Fence {
                in_fence = !in_fence;
            }
            let runs = build_runs(&parsed, &t, font_size);
            let shape = window
                .text_system()
                .shape_line(parsed.display.clone().into(), px(font_size), &runs, None);
            if li == cursor_line {
                let dcol = self
                    .input
                    .read(cx)
                    .editor()
                    .cursor_display_col(&parsed);
                let x = bounds.left() + shape.x_for_index(dcol.min(shape.len()));
                caret = Some(fill(
                    Bounds::new(point(x, y), size(px(2.), line_height)),
                    t.heading,
                ));
            }
            shapes.push(shape);
            y += line_height;
        }

        let selection = sel_start.and_then(|(sl, sc)| {
            let cur = (cursor_line, cursor_col);
            if (sl, sc) == cur {
                return None;
            }
            let (start, end) = if (sl, sc) < cur {
                ((sl, sc), cur)
            } else {
                (cur, (sl, sc))
            };
            let top = bounds.top() + (start.0 as f32 * line_height_f).into();
            let bottom = bounds.top() + ((end.0 + 1) as f32 * line_height_f).into();
            Some(fill(
                Bounds::new(
                    point(bounds.left(), top),
                    size(bounds.right() - bounds.left(), bottom - top),
                ),
                rgba(0x0a66c260),
            ))
        });

        // 缓存布局供命中测试 / IME 定位。
        let cache_shapes: Vec<Option<ShapedLine>> = shapes.iter().map(|s| Some(s.clone())).collect();
        self.input.update(cx, |view, _| {
            let e = view.editor_mut();
            e.last_shapes = cache_shapes;
            e.last_bounds = Some(bounds);
        });

        let _ = marked;
        PrepaintState {
            lines: shapes,
            caret,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(sel) = prepaint.selection.take() {
            window.paint_quad(sel);
        }
        let line_height = px(self.input.read(cx).editor().line_height);
        let mut y = bounds.top();
        for shape in prepaint.lines.iter() {
            shape
                .paint(point(bounds.left(), y), line_height, window, cx)
                .unwrap();
            y += line_height;
        }
        if focus.is_focused(window) {
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        }
    }
}

fn build_runs(parsed: &ParsedLine, t: &Theme, _font_size: f32) -> Vec<TextRun> {
    let default_font = Font {
        family: ".SystemUIFont".into(),
        features: Default::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    };
    let mut runs: Vec<TextRun> = Vec::new();
    for seg in &parsed.segs {
        let mut font = default_font.clone();
        let mut color = t.fg;
        let mut bg = None;
        match seg.style {
            TextStyle::Bold => font.weight = FontWeight::BOLD,
            TextStyle::Italic => font.style = FontStyle::Italic,
            TextStyle::BoldItalic => {
                font.weight = FontWeight::BOLD;
                font.style = FontStyle::Italic;
            }
            TextStyle::Code => {
                font.family = "Menlo".into();
                color = t.code_fg;
                bg = Some(t.code_bg);
            }
            TextStyle::Mark => bg = Some(t.line_highlight),
            TextStyle::Link => color = t.info_accent,
            TextStyle::Strike | TextStyle::Plain => {}
        }
        runs.push(TextRun {
            len: seg.text.len(),
            font,
            color,
            background_color: bg,
            underline: if seg.style == TextStyle::Link {
                Some(UnderlineStyle {
                    color: None,
                    thickness: px(1.0),
                    wavy: false,
                })
            } else {
                None
            },
            strikethrough: if seg.style == TextStyle::Strike {
                Some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: None,
                })
            } else {
                None
            },
        });
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_line_map() {
        let p = parse_line("hello 世界", false);
        assert_eq!(p.display, "hello 世界");
        assert_eq!(p.map.len(), p.display.chars().count());
        assert_eq!(p.map[0], 0);
        assert_eq!(*p.map.last().unwrap(), 9); // '界' 起点
    }

    #[test]
    fn bold_removed_from_display() {
        let p = parse_line("**加粗** 和 *斜体*", false);
        assert_eq!(p.display, "加粗 和 斜体");
        // '加' -> 源码偏移 2（** 后）
        assert_eq!(p.map[0], 2);
    }

    #[test]
    fn heading_prefix() {
        let p = parse_line("## 标题", false);
        assert_eq!(p.line_style, LineStyle::Heading(2));
        assert_eq!(p.prefix_len, 3); // "## "
        assert_eq!(p.display, "标题");
    }

    #[test]
    fn bullet_prefix() {
        let p = parse_line("- 项目", false);
        assert_eq!(p.line_style, LineStyle::Bullet);
        assert_eq!(p.prefix_len, 2);
        assert_eq!(p.display, "项目");
    }

    #[test]
    fn code_span() {
        let p = parse_line("用 `x = 1` 表示", false);
        assert_eq!(p.display, "用 x = 1 表示");
        assert_eq!(p.map[2], 5); // x 在源码 offset 5（"用 " 3字节 + " " + "`"）
    }

    #[test]
    fn link_removed() {
        let p = parse_line("看[文档](https://a.b)", false);
        assert_eq!(p.display, "看文档");
    }

    #[test]
    fn bold_italic_triple() {
        let p = parse_line("***粗斜体***", false);
        assert_eq!(p.display, "粗斜体");
        assert_eq!(p.segs[0].style, TextStyle::BoldItalic);
    }

    #[test]
    fn mark_removed() {
        let p = parse_line("==重点==", false);
        assert_eq!(p.display, "重点");
    }

    #[test]
    fn display_col_roundtrip() {
        let src = "**bold** text";
        let p = parse_line(src, false);
        let mut e = Editor::new(src);
        e.cursor_col = 4; // 光标在 "bo" 之后（源码）
        assert_eq!(e.cursor_display_col(&p), 2);
        // 显示 col 2 -> 源码 "bo" 后
        assert_eq!(e.source_col_for_display(0, 2), 4);
    }

    #[test]
    fn insert_mid_bold() {
        let mut e = Editor::new("**bold**");
        e.cursor_col = 4; // 在 "bold" 中间
        e.insert_text("X");
        assert_eq!(e.to_source(), "**boXld**");
        assert!(e.dirty);
    }

    #[test]
    fn insert_newline_splits() {
        let mut e = Editor::new("ab\ncd");
        e.cursor_line = 0;
        e.cursor_col = 1;
        e.insert_newline();
        assert_eq!(e.lines, vec!["a", "b", "cd"]);
        assert_eq!((e.cursor_line, e.cursor_col), (1, 0));
    }

    #[test]
    fn backspace_joins_lines() {
        let mut e = Editor::new("abc\ndef");
        e.cursor_line = 1;
        e.cursor_col = 0;
        e.backspace();
        assert_eq!(e.to_source(), "abcdef");
        assert_eq!((e.cursor_line, e.cursor_col), (0, 3));
    }

    #[test]
    fn selection_delete_and_text() {
        let mut e = Editor::new("hello world");
        e.sel_start = Some((0, 0));
        e.cursor_line = 0;
        e.cursor_col = 5;
        assert_eq!(e.selected_text().as_deref(), Some("hello"));
        e.delete_selection();
        assert_eq!(e.to_source(), " world");
    }

    #[test]
    fn utf16_pos_roundtrip() {
        let e = Editor::new("你好\nworld");
        let (l, c) = e.pos_from_utf16(2); // 2 = line0 末尾（"你好" 之后）
        assert_eq!((l, c), (0, 6));
        let off = e.utf16_from_pos(1, 0); // 'w' 起点
        assert_eq!(off, 3);
    }}
