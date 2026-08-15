//! Markdown parsing: VitePress-flavoured preprocessing on top of pulldown-cmark.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, UnderlineStyle, px};
use pulldown_cmark::{
    CodeBlockKind, Event, Options, Parser, Tag, TagEnd,
};

use crate::theme::hsl;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// Inline content: the full text plus highlight ranges to apply.
pub type Inline = (String, Vec<(Range<usize>, HighlightStyle)>);

#[derive(Debug, Clone)]
pub struct Doc {
    pub title: Option<String>,
    pub description: Option<String>,
    pub blocks: Vec<Block>,
    pub footnotes: Vec<(String, Vec<Block>)>,
    /// (level, plain heading text) — used to render `[[toc]]`.
    pub headings: Vec<(u32, String)>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Heading { level: u32, inline: Inline },
    Paragraph { inline: Inline, task: Option<bool> },
    List { ordered: bool, start: u64, items: Vec<Vec<Block>> },
    Code {
        lang: String,
        text: String,
        highlight: Vec<(usize, usize)>,
        line_numbers: bool,
        title: Option<String>,
    },
    Quote(Vec<Block>),
    Table { head: Vec<Inline>, rows: Vec<Vec<Inline>> },
    Container { kind: String, title: Option<String>, inner: Vec<Block> },
    DefinitionList(Vec<(Inline, Vec<Vec<Block>>)>),
    Math { text: String },
    Rule,
    Html(String),
}

// ---------------------------------------------------------------------------
// Inline styling constants
// ---------------------------------------------------------------------------

const INLINE_CODE_BG: u32 = 0xeff1f3;
const INLINE_CODE_FG: u32 = 0xad3da4;
const LINK_COLOR: u32 = 0x0a66c2;
const MARK_BG: u32 = 0xfff3a0;

// ---------------------------------------------------------------------------
// Preprocessing (fence-aware)
// ---------------------------------------------------------------------------

const MAX_INCLUDE_DEPTH: usize = 10;

/// Runs VitePress block-level preprocessors:
/// 1. `<!--@include: path-->` file inclusion (recursive, depth-limited)
/// 2. `<<< path` code-snippet imports
/// 3. `::: type` custom containers → `<vp-container>` markers
fn preprocess(source: &str, base_dir: Option<&Path>, depth: usize) -> String {
    if depth > MAX_INCLUDE_DEPTH {
        return String::new();
    }
    let mut out = String::new();
    let mut fence: Option<char> = None;
    let mut fence_len = 0usize;
    // Tight lists emit bare Text events (no Paragraph wrapper), which our
    // parser would drop. Force lists loose by separating items with a blank line.
    let mut prev_was_item = false;

    for line in source.lines() {
        let trimmed = line.trim_start();

        // Track fenced code blocks so we never mangle their contents.
        if let Some(f) = trimmed.chars().next() {
            if f == '`' || f == '~' {
                let run = trimmed.chars().take_while(|&c| c == f).count();
                if run >= 3 {
                    if let Some(open) = fence {
                        if open == f && run >= fence_len {
                            fence = None;
                        }
                    } else {
                        fence = Some(f);
                        fence_len = run;
                    }
                    out.push_str(line);
                    out.push('\n');
                    prev_was_item = false;
                    continue;
                }
            }
        }

        if fence.is_none() {
            let is_item = is_list_item_line(trimmed);
            if prev_was_item && is_item {
                out.push('\n');
            }
            prev_was_item = is_item;
            // 1. Markdown include
            if let Some(rest) = trimmed.strip_prefix("<!--@include:") {
                if let Some(end) = rest.find("-->") {
                    let spec = rest[..end].trim().trim_matches('"').trim_matches('\'');
                    if let Some(content) = load_text_file(spec, base_dir) {
                        out.push_str(&preprocess(&content, base_dir, depth + 1));
                        out.push('\n');
                        continue;
                    }
                }
            }

            // 2. Code snippet import: `<<< @/path/file.js{2,5} title="x"`
            if let Some(spec) = trimmed.strip_prefix("<<<") {
                let spec = spec.trim();
                if !spec.is_empty() {
                    if let Some(block) = import_code(spec, base_dir) {
                        out.push_str(&block);
                        continue;
                    }
                }
            }

            // 3. Custom containers
            if let Some(spec) = trimmed.strip_prefix(":::") {
                let spec = spec.trim();
                if spec.is_empty() {
                    // Blank line first so the previous marker's HTML block ends.
                    out.push('\n');
                    out.push_str("</vp-container>\n");
                } else {
                    let mut parts = spec.splitn(2, char::is_whitespace);
                    let kind = parts.next().unwrap_or("").trim();
                    let title = parts
                        .next()
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty());
                    let mut tag = format!("<vp-container type=\"{kind}\"");
                    if let Some(t) = title {
                        tag.push_str(&format!(" title=\"{}\"", t.replace('"', "&quot;")));
                    }
                    tag.push('>');
                    out.push_str(&tag);
                    out.push('\n');
                    // Blank line so pulldown-cmark ends the marker's HTML block
                    // here and parses the inner content as Markdown.
                    out.push('\n');
                }
                continue;
            }
        }

        out.push_str(line);
        out.push('\n');
    }
    out
}

fn load_text_file(spec: &str, base_dir: Option<&Path>) -> Option<String> {
    if spec.contains("..") {
        // allow relative traversal? keep it simple: allow.
    }
    let path = resolve_path(spec, base_dir)?;
    std::fs::read_to_string(path).ok()
}

/// True for lines that start a markdown list item (`- x`, `* x`, `+ x`, `1. x`, `1) x`).
fn is_list_item_line(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    match chars.next() {
        Some('-') | Some('*') | Some('+') => matches!(chars.next(), Some(c) if c.is_whitespace()),
        Some(c) if c.is_ascii_digit() => {
            let rest = &trimmed[1..];
            let end = rest
                .find(|ch: char| !ch.is_ascii_digit())
                .unwrap_or(rest.len());
            let after = &rest[end..];
            if after.is_empty() {
                return false;
            }
            let mut it = after.chars();
            match it.next() {
                Some('.') | Some(')') => matches!(it.next(), Some(c) if c.is_whitespace()),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Resolve `@/...` (current file's directory) or a relative path against `base_dir`.
fn resolve_path(spec: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let rel = spec.strip_prefix("@/").unwrap_or(spec);
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    base_dir.map(|d| d.join(rel))
}

/// Parse `path{2,5,7-9} title="..."` into (path, line ranges, title).
fn parse_import_spec(spec: &str) -> (String, Vec<(usize, usize)>, Option<String>) {
    let mut path = spec.to_string();
    let mut ranges = Vec::new();
    let mut title = None;

    if let Some(idx) = spec.find("title=") {
        let mut t = spec[idx + 6..].trim().to_string();
        if let Some(stripped) = t.strip_prefix('"') {
            t = stripped.to_string();
        }
        if let Some(stripped) = t.strip_suffix('"') {
            t = stripped.to_string();
        }
        title = Some(t);
        path = spec[..idx].trim().to_string();
    }
    if let Some(idx) = spec.find('{') {
        if let Some(rel) = spec[idx..].find('}') {
            let end = idx + rel;
            ranges = parse_line_ranges(&spec[idx + 1..end]);
            path = spec[..idx].trim().to_string();
        }
    }
    (path, ranges, title)
}

/// `<<< @/snippets/foo.js{2,5} title="bar"` → a normal fenced code block with the file content.
fn import_code(spec: &str, base_dir: Option<&Path>) -> Option<String> {
    let (path_spec, ranges, title) = parse_import_spec(spec);
    let path = resolve_path(&path_spec, base_dir)?;
    let content = std::fs::read_to_string(&path).ok()?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("txt")
        .to_string();

    let selected: String = if ranges.is_empty() {
        content.clone()
    } else {
        let lines: Vec<&str> = content.lines().collect();
        let mut out = String::new();
        for (start, end) in &ranges {
            for ln in *start..=*end {
                if let Some(l) = lines.get(ln - 1) {
                    out.push_str(l);
                    out.push('\n');
                }
            }
        }
        out
    };

    let mut block = format!("```{ext}");
    if let Some(t) = title {
        block.push_str(&format!(" title=\"{}\"", t.replace('"', "&quot;")));
    }
    block.push('\n');
    block.push_str(&selected);
    if !selected.ends_with('\n') {
        block.push('\n');
    }
    block.push_str("```\n");
    Some(block)
}

/// Parse `1,3-5,8` (1-based inclusive) into (start, end) pairs.
fn parse_line_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let a: usize = a.trim().parse().unwrap_or(0);
            let b: usize = b.trim().parse().unwrap_or(a);
            if a > 0 && b >= a {
                out.push((a, b));
            }
        } else if let Ok(n) = part.parse::<usize>() {
            if n > 0 {
                out.push((n, n));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct ParseCtx {
    title: Option<String>,
    description: Option<String>,
    footnotes: Vec<(String, Vec<Block>)>,
    headings: Vec<(u32, String)>,
}

pub fn parse_document(source: &str, base_dir: Option<&Path>) -> Doc {
    let source = preprocess(source, base_dir, 0);

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    opts.insert(Options::ENABLE_MATH);
    opts.insert(Options::ENABLE_GFM);
    opts.insert(Options::ENABLE_DEFINITION_LIST);
    // NOTE: superscript/subscript are handled by `split_supsub` on raw Text —
    // pulldown-cmark splits `^x^` into separate Text events which breaks matching.

    let mut parser = Parser::new_ext(&source, opts);
    let mut ctx = ParseCtx {
        title: None,
        description: None,
        footnotes: Vec::new(),
        headings: Vec::new(),
    };
    let blocks = parse_blocks(&mut parser, &mut ctx);

    Doc {
        title: ctx.title,
        description: ctx.description,
        blocks,
        footnotes: ctx.footnotes,
        headings: ctx.headings,
    }
}

fn parse_blocks(parser: &mut Parser, ctx: &mut ParseCtx) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut pending_task: Option<bool> = None;

    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    let (inline, task) = parse_inline(parser);
                    blocks.push(Block::Paragraph {
                        inline,
                        task: task.or(pending_task.take()),
                    });
                }
                Tag::Heading { level, .. } => {
                    let (inline, _) = parse_inline(parser);
                    ctx.headings.push((level as u32, inline.0.trim().to_string()));
                    blocks.push(Block::Heading {
                        level: level as u32,
                        inline,
                    });
                }
                Tag::List(start) => blocks.push(parse_list(parser, start, ctx)),
                Tag::CodeBlock(kind) => {
                    let info = match kind {
                        CodeBlockKind::Fenced(lang) => parse_code_info(&lang),
                        CodeBlockKind::Indented => CodeInfo::default(),
                    };
                    let mut text = String::new();
                    while let Some(ev2) = parser.next() {
                        match ev2 {
                            Event::Text(t) => text.push_str(&t),
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    blocks.push(Block::Code {
                        lang: info.lang,
                        text,
                        highlight: info.highlight,
                        line_numbers: info.line_numbers,
                        title: info.title,
                    });
                }
                Tag::BlockQuote(_) => blocks.push(Block::Quote(parse_blocks(parser, ctx))),
                Tag::Table(_) => blocks.push(parse_table(parser)),
                Tag::MetadataBlock(_) => {
                    let mut yaml = String::new();
                    while let Some(ev2) = parser.next() {
                        match ev2 {
                            Event::Text(t) => yaml.push_str(&t),
                            Event::End(TagEnd::MetadataBlock(_)) => break,
                            _ => {}
                        }
                    }
                    for line in yaml.lines() {
                        let line = line.trim();
                        if let Some(v) = line.strip_prefix("title:") {
                            ctx.title = Some(unquote(v.trim()));
                        } else if let Some(v) = line.strip_prefix("description:") {
                            ctx.description = Some(unquote(v.trim()));
                        }
                    }
                }
                Tag::DefinitionList => blocks.push(parse_definition_list(parser)),
                Tag::FootnoteDefinition(label) => {
                    let inner = parse_blocks(parser, ctx);
                    ctx.footnotes.push((label.to_string(), inner));
                }
                _ => {
                    // Unknown container (e.g. HtmlBlock wrapping raw HTML): keep its
                    // contents at this level instead of discarding them.
                    blocks.append(&mut parse_blocks(parser, ctx));
                }
            },
            Event::TaskListMarker(checked) => pending_task = Some(checked),
            Event::Html(html) => {
                let t = html.trim();
                if t.starts_with("<vp-container") {
                    let (kind, title) = parse_container_tag(t);
                    // The open tag's HtmlBlock is closed immediately after the
                    // Html event; consume it so the inner content parses.
                    let _ = parser.next();
                    let inner = parse_blocks(parser, ctx);
                    blocks.push(Block::Container { kind, title, inner });
                } else if t.starts_with("</vp-container") {
                    break;
                } else {
                    blocks.push(Block::Html(html_to_text(&html)));
                }
            }
            Event::DisplayMath(m) => blocks.push(Block::Math {
                text: m.to_string(),
            }),
            Event::End(_) => break,
            Event::Rule => blocks.push(Block::Rule),
            _ => {}
        }
    }
    blocks
}

fn parse_list(parser: &mut Parser, start: Option<u64>, ctx: &mut ParseCtx) -> Block {
    let mut items = Vec::new();
    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::Item) => items.push(parse_blocks(parser, ctx)),
            Event::End(TagEnd::List(_)) => break,
            _ => {}
        }
    }
    Block::List {
        ordered: start.is_some(),
        start: start.unwrap_or(1),
        items,
    }
}

fn parse_table(parser: &mut Parser) -> Block {
    let mut head: Vec<Inline> = Vec::new();
    let mut rows: Vec<Vec<Inline>> = Vec::new();
    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::TableHead) => head = parse_table_row(parser),
            Event::Start(Tag::TableRow) => rows.push(parse_table_row(parser)),
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    Block::Table { head, rows }
}

fn parse_table_row(parser: &mut Parser) -> Vec<Inline> {
    let mut cells = Vec::new();
    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::TableCell) => cells.push(parse_inline(parser).0),
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => break,
            _ => {}
        }
    }
    cells
}

fn parse_definition_list(parser: &mut Parser) -> Block {
    let mut items: Vec<(Inline, Vec<Vec<Block>>)> = Vec::new();
    let mut term: Option<Inline> = None;
    let mut defs: Vec<Vec<Block>> = Vec::new();

    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::DefinitionListTitle) => {
                if let Some(t) = term.take() {
                    items.push((t, std::mem::take(&mut defs)));
                }
                term = Some(parse_inline(parser).0);
            }
            Event::Start(Tag::DefinitionListDefinition) => {
                // Definition content arrives as bare inline text (no Paragraph
                // wrapper), so collect it directly.
                let inline = parse_inline(parser).0;
                defs.push(vec![Block::Paragraph {
                    inline,
                    task: None,
                }]);
            }
            Event::End(TagEnd::DefinitionList) => break,
            _ => {}
        }
    }
    if let Some(t) = term.take() {
        items.push((t, defs));
    }
    Block::DefinitionList(items)
}

// ---------------------------------------------------------------------------
// Inline parsing
// ---------------------------------------------------------------------------

fn parse_inline(parser: &mut Parser) -> (Inline, Option<bool>) {
    let mut task: Option<bool> = None;
    let mut segments: Vec<(String, HighlightStyle)> = Vec::new();
    let mut style = HighlightStyle::default();
    let mut stack: Vec<HighlightStyle> = Vec::new();

    loop {
        match parser.next() {
            Some(Event::Text(t)) => {
                // VitePress markdown-it-mark: ==highlight==, plus intraword
                // superscript/subscript that pulldown-cmark leaves literal.
                for (text, s) in split_mark(&t, style) {
                    for (text, s) in split_supsub(&text, s) {
                        segments.push((replace_emoji(&text), s));
                    }
                }
            }
            Some(Event::Code(c)) => {
                let mut s = style;
                s.background_color = Some(hsl(INLINE_CODE_BG));
                s.color = Some(hsl(INLINE_CODE_FG));
                segments.push((c.to_string(), s));
            }
            Some(Event::TaskListMarker(checked)) => {
                task = Some(checked);
            }
            Some(Event::SoftBreak) => segments.push((" ".to_string(), style)),
            Some(Event::HardBreak) => segments.push(("\n".to_string(), style)),
            Some(Event::InlineMath(m)) | Some(Event::DisplayMath(m)) => {
                let mut s = style;
                s.font_style = Some(FontStyle::Italic);
                segments.push((replace_emoji(&m), s));
            }
            Some(Event::FootnoteReference(label)) => {
                let mut s = style;
                s.color = Some(hsl(LINK_COLOR));
                segments.push((format!("[{}]", label), s));
            }
            Some(Event::InlineHtml(html)) => {
                let t = html.trim();
                if t == "<mark>" || t == "<mark >" {
                    stack.push(style);
                    style.background_color = Some(hsl(MARK_BG));
                } else if t == "</mark>" || t == "</mark >" {
                    style = stack.pop().unwrap_or_default();
                } else {
                    let text = html_to_text(&html);
                    if !text.trim().is_empty() {
                        segments.push((text, style));
                    }
                }
            }
            Some(Event::Start(tag)) => match tag {
                Tag::Emphasis => {
                    stack.push(style);
                    style.font_style = Some(FontStyle::Italic);
                }
                Tag::Strong => {
                    stack.push(style);
                    style.font_weight = Some(FontWeight::BOLD);
                }
                Tag::Strikethrough => {
                    stack.push(style);
                    style.strikethrough = Some(StrikethroughStyle {
                        thickness: px(1.0),
                        color: None,
                    });
                }
                Tag::Superscript | Tag::Subscript => {
                    stack.push(style);
                    style.font_style = Some(FontStyle::Italic);
                    style.fade_out = Some(0.25);
                }
                Tag::Link { .. } => {
                    stack.push(style);
                    style.underline = Some(UnderlineStyle {
                        thickness: px(1.0),
                        color: None,
                        wavy: false,
                    });
                    style.color = Some(hsl(LINK_COLOR));
                }
                Tag::Image { .. } => {
                    stack.push(style);
                    style.fade_out = Some(0.15);
                }
                _ => {}
            },
            Some(Event::End(tag)) => match tag {
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::Link
                | TagEnd::Image => {
                    style = stack.pop().unwrap_or_default();
                }
                _ => break,
            },
            _ => break,
        }
    }

    // Merge consecutive segments with identical style.
    let mut merged: Vec<(String, HighlightStyle)> = Vec::new();
    for (text, seg_style) in segments {
        if text.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.1 == seg_style {
                last.0.push_str(&text);
                continue;
            }
        }
        merged.push((text, seg_style));
    }

    let mut text = String::new();
    let mut highlights = Vec::new();
    for (seg_text, seg_style) in merged {
        let start = text.len();
        text.push_str(&seg_text);
        if seg_style != HighlightStyle::default() {
            highlights.push((start..text.len(), seg_style));
        }
    }
    ((text, highlights), task)
}

/// Split `==text==` mark spans out of plain text (markdown-it-mark).
fn split_mark(text: &str, base: HighlightStyle) -> Vec<(String, HighlightStyle)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("==") {
        let after = &rest[i + 2..];
        if let Some(j) = after.find("==") {
            let inner = &after[..j];
            if !inner.is_empty() && !inner.contains('=') {
                if i > 0 {
                    out.push((rest[..i].to_string(), base));
                }
                let mut s = base;
                s.background_color = Some(hsl(MARK_BG));
                out.push((inner.to_string(), s));
                rest = &after[j + 2..];
                continue;
            }
        }
        out.push((rest.to_string(), base));
        return out;
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), base));
    }
    out
}

/// Split intraword `^sup^` / `~sub~` spans (markdown-it-sup / markdown-it-sub).
/// pulldown-cmark only handles these when bounded by whitespace/punctuation,
/// so we catch the word-adjacent cases here. `~~strike~~` is left untouched.
fn split_supsub(text: &str, base: HighlightStyle) -> Vec<(String, HighlightStyle)> {
    if !text.contains(['^', '~']) {
        return vec![(text.to_string(), base)];
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    while i < chars.len() {
        let (off, c) = chars[i];
        if c == '^' || c == '~' {
            // Skip double tildes (strikethrough remnants).
            if c == '~' && i + 1 < chars.len() && chars[i + 1].1 == '~' {
                cur.push('~');
                cur.push('~');
                i += 2;
                continue;
            }
            let after = &text[off + c.len_utf8()..];
            if let Some(rel) = after.find(c) {
                let inner = &after[..rel];
                if !inner.is_empty()
                    && !inner.contains([' ', '\t'])
                    && !inner.contains(['^', '~'])
                {
                    if !cur.is_empty() {
                        out.push((std::mem::take(&mut cur), base));
                    }
                    let mut s = base;
                    s.font_style = Some(FontStyle::Italic);
                    s.fade_out = Some(0.25);
                    out.push((inner.to_string(), s));
                    i += 1 + inner.chars().count() + 1;
                    continue;
                }
            }
        }
        cur.push(c);
        i += 1;
    }
    if !cur.is_empty() {
        out.push((cur, base));
    }
    out
}

/// Replace `:shortcode:` emoji tokens with their Unicode glyphs.
fn replace_emoji(text: &str) -> String {
    if !text.contains(':') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(':') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(end_rel) = after.find(':') else {
            out.push(':');
            out.push_str(after);
            return out;
        };
        let name = &after[..end_rel];
        let valid = !name.is_empty()
            && name.len() <= 64
            && name.bytes().all(|b| {
                b.is_ascii_alphanumeric() || b == b'_' || b == b'+' || b == b'-'
            });
        if valid {
            if let Some(emoji) = emojis::get_by_shortcode(name) {
                out.push_str(emoji.as_str());
                rest = &after[end_rel + 1..];
                continue;
            }
        }
        out.push(':');
        rest = after;
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Fenced code info string
// ---------------------------------------------------------------------------

struct CodeInfo {
    lang: String,
    title: Option<String>,
    highlight: Vec<(usize, usize)>,
    line_numbers: bool,
}

impl Default for CodeInfo {
    fn default() -> Self {
        CodeInfo {
            lang: String::new(),
            title: None,
            highlight: Vec::new(),
            line_numbers: false,
        }
    }
}

/// Parse VitePress code info: `js {1,3-5} title="x" :line-numbers`
fn parse_code_info(info: &str) -> CodeInfo {
    let mut out = CodeInfo::default();
    for token in tokenize_info(info) {
        if let Some(inner) = token.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
            out.highlight = parse_line_ranges(inner);
        } else if let Some(t) = token.strip_prefix("title=") {
            out.title = Some(unquote(t));
        } else if token == ":line-numbers" {
            out.line_numbers = true;
        } else if out.lang.is_empty() {
            out.lang = token;
        }
    }
    out
}

/// Tokenize an info string, keeping `"..."` fragments intact.
fn tokenize_info(info: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in info.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            ' ' | '\t' if !in_quote => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

// ---------------------------------------------------------------------------
// Small text helpers
// ---------------------------------------------------------------------------

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').unwrap_or(s);
    let s = s.strip_suffix('"').unwrap_or(s);
    let s = s.strip_prefix('\'').unwrap_or(s);
    s.strip_suffix('\'').unwrap_or(s).to_string()
}

fn parse_container_tag(tag: &str) -> (String, Option<String>) {
    let mut kind = String::new();
    let mut title = None;
    for (k, v) in extract_attrs(tag) {
        match k.as_str() {
            "type" => kind = v,
            "title" => title = Some(v),
            _ => {}
        }
    }
    (kind, title)
}

fn extract_attrs(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find('=') {
        let before = &rest[..eq];
        let key = before
            .rsplit(|c: char| c.is_whitespace() || c == '<' || c == '>')
            .next()
            .unwrap_or("")
            .to_string();
        let after = &rest[eq + 1..];
        let Some(quote_start) = after.find(['"', '\'']) else {
            break;
        };
        let q = after[quote_start..].chars().next().unwrap();
        let value_start = quote_start + 1;
        let Some(rel) = after[value_start..].find(q) else {
            break;
        };
        let value = after[value_start..value_start + rel].to_string();
        out.push((key, value));
        rest = &after[value_start + rel + 1..];
    }
    out
}

/// Strip HTML tags, decode entities, and turn block tags into newlines.
fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag_buf = String::new();
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            tag_buf.clear();
            continue;
        }
        if c == '>' {
            in_tag = false;
            let t = tag_buf
                .trim_start_matches('/')
                .trim_end_matches('/')
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase();
            if matches!(t.as_str(), "br" | "hr" | "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr" | "table") {
                out.push('\n');
            }
            continue;
        }
        if in_tag {
            tag_buf.push(c);
        } else {
            out.push(c);
        }
    }
    decode_entities(&out)
}

fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        // find ';'
        if let Some(rel) = after.find(';') {
            let ent = &after[..=rel];
            let decoded = match ent {
                "&amp;" => "&",
                "&lt;" => "<",
                "&gt;" => ">",
                "&quot;" => "\"",
                "&#39;" => "'",
                "&apos;" => "'",
                "&nbsp;" => " ",
                "&hellip;" => "…",
                "&mdash;" => "—",
                "&ndash;" => "–",
                "&copy;" => "©",
                "&reg;" => "®",
                "&times;" => "×",
                "&divide;" => "÷",
                _ => {
                    // numeric entities
                    if let Some(num) = ent
                        .strip_prefix("&#x")
                        .and_then(|n| n.strip_suffix(';'))
                        .and_then(|n| u32::from_str_radix(n, 16).ok())
                        .or_else(|| {
                            ent.strip_prefix("&#")
                                .and_then(|n| n.strip_suffix(';'))
                                .and_then(|n| n.parse::<u32>().ok())
                        })
                    {
                        if let Some(ch) = char::from_u32(num) {
                            out.push(ch);
                            rest = &rest[i + ent.len()..];
                            continue;
                        }
                    }
                    ent
                }
            };
            out.push_str(decoded);
            rest = &rest[i + ent.len()..];
        } else {
            out.push_str(after);
            return out;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbg_doc(src: &str) -> Vec<Block> {
        parse_document(src, None).blocks
    }

    #[test]
    fn container_parses() {
        let blocks = dbg_doc("::: tip\n\n内容\n\n:::\n");
        assert!(matches!(
            blocks.as_slice(),
            [Block::Container { kind, inner, .. }]
                if kind == "tip" && inner.len() == 1
        ));
    }

    #[test]
    fn container_with_title() {
        let blocks = dbg_doc("::: warning 标题\n\n内容\n\n:::\n");
        assert!(matches!(
            blocks.as_slice(),
            [Block::Container { kind, title, .. }]
                if kind == "warning" && title.as_deref() == Some("标题")
        ));
    }

    #[test]
    fn nested_containers() {
        let blocks = dbg_doc("::: tip\n\n内层\n\n::: info\n\n深层\n\n:::\n\n结尾\n\n:::\n");
        assert!(matches!(
            blocks.as_slice(),
            [Block::Container { inner, .. }] if inner.len() == 3
        ));
    }

    #[test]
    fn fence_keeps_container_marker_literal() {
        let blocks = dbg_doc("```rs\n::: tip\n```\n\n普通文本\n");
        assert!(matches!(
            blocks.as_slice(),
            [Block::Code { text, .. }, Block::Paragraph { .. }]
                if text.trim() == "::: tip"
        ));
    }

    #[test]
    fn subscript_superscript_intraword() {
        let blocks = dbg_doc("H~2~O 和 E=mc^2^\n");
        let Block::Paragraph { inline, .. } = &blocks[0] else {
            panic!("expected paragraph");
        };
        assert_eq!(inline.0, "H2O 和 E=mc2");
        assert_eq!(inline.1.len(), 2, "both sub and sup styled");
    }

    #[test]
    fn mark_highlight() {
        let blocks = dbg_doc("这是 ==重点== 内容\n");
        let Block::Paragraph { inline, .. } = &blocks[0] else {
            panic!("expected paragraph");
        };
        assert_eq!(inline.0, "这是 重点 内容");
        assert!(inline.1[0].1.background_color.is_some());
    }

    #[test]
    fn emoji_shortcodes() {
        let blocks = dbg_doc(":tada: 庆祝\n");
        let Block::Paragraph { inline, .. } = &blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(inline.0.contains('🎉'));
    }

    #[test]
    fn tight_list_items_keep_content() {
        let blocks = dbg_doc("- 甲\n- 乙\n- 丙\n");
        assert!(matches!(
            blocks.as_slice(),
            [Block::List { items, .. }] if items.len() == 3
        ));
        let Block::List { items, .. } = &blocks[0] else {
            unreachable!()
        };
        assert!(matches!(&items[0][0], Block::Paragraph { .. }));
    }

    #[test]
    fn task_list_marker() {
        let blocks = dbg_doc("- [x] 完成\n- [ ] 待办\n");
        let Block::List { items, .. } = &blocks[0] else {
            panic!("expected list");
        };
        let Block::Paragraph { task, .. } = &items[0][0] else {
            panic!("expected paragraph");
        };
        assert_eq!(*task, Some(true));
        let Block::Paragraph { task, .. } = &items[1][0] else {
            panic!("expected paragraph");
        };
        assert_eq!(*task, Some(false));
    }

    #[test]
    fn frontmatter_title() {
        let src = "---\ntitle: 我的文档\ndescription: 说明\n---\n\n正文\n";
        let doc = parse_document(src, None);
        assert_eq!(doc.title.as_deref(), Some("我的文档"));
        assert_eq!(doc.description.as_deref(), Some("说明"));
        assert_eq!(doc.blocks.len(), 1);
    }

    #[test]
    fn code_info_highlight_and_lines() {
        let blocks = dbg_doc("```ts {1,3-5} :line-numbers title=\"x.ts\"\na\nb\nc\n```\n");
        let Block::Code {
            lang,
            highlight,
            line_numbers,
            title,
            ..
        } = &blocks[0]
        else {
            panic!("expected code");
        };
        assert_eq!(lang, "ts");
        assert_eq!(title.as_deref(), Some("x.ts"));
        assert!(line_numbers);
        assert!(highlight.contains(&(1, 1)));
        assert!(highlight.contains(&(3, 5)));
    }

    #[test]
    fn toc_paragraph_preserved() {
        let blocks = dbg_doc("[[toc]]\n");
        assert!(matches!(
            blocks.as_slice(),
            [Block::Paragraph { inline, .. }] if inline.0.trim() == "[[toc]]"
        ));
    }

    #[test]
    fn footnote_collected() {
        let src = "正文[^1]\n\n[^1]: 脚注内容\n";
        let doc = parse_document(src, None);
        assert_eq!(doc.footnotes.len(), 1);
        assert_eq!(doc.footnotes[0].0, "1");
    }
}
