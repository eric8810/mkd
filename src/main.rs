//! mkd — a featherweight macOS-native Markdown reader built on GPUI.
//!
//! Usage:
//!   mkd [path/to/file.md]
//!
//! Key bindings:
//!   cmd-q  quit
//!   cmd-r  reload the current file

use std::env;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use gpui::{
    actions, App, Application, AnyElement, AsyncApp, Bounds, Context, ElementId, FontStyle,
    FontWeight, HighlightStyle, Hsla, KeyBinding, Render, StrikethroughStyle, StyledText,
    TextStyle, Timer, TitlebarOptions, UnderlineStyle, WeakEntity, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, relative, rgb, size,
};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

actions!(mdk, [Quit, Reload]);

/// Files handed to the app by macOS (Finder double-click / `open -a`) land here,
/// then the view's poll loop picks them up.
static PENDING_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Document model
// ---------------------------------------------------------------------------

/// Inline content of a block: the full text plus highlight ranges to apply.
type Inline = (String, Vec<(Range<usize>, HighlightStyle)>);

#[derive(Debug, Clone)]
enum Block {
    Heading { level: u32, inline: Inline },
    Paragraph(Inline),
    List { ordered: bool, start: u64, items: Vec<Vec<Block>> },
    Code { lang: Option<String>, text: String },
    Quote(Vec<Block>),
    Table { head: Vec<Inline>, rows: Vec<Vec<Inline>> },
    Rule,
    Html(String),
}

// ---------------------------------------------------------------------------
// Parsing (pulldown-cmark)
// ---------------------------------------------------------------------------

const INLINE_CODE_BG: u32 = 0xeff1f3;
const INLINE_CODE_FG: u32 = 0xad3da4;
const LINK_COLOR: u32 = 0x0a66c2;

/// `rgb()` yields `Rgba`; most style fields want `Hsla`.
fn hsl(c: u32) -> Hsla {
    rgb(c).into()
}

fn parse_inline(parser: &mut Parser) -> Inline {
    let mut segments: Vec<(String, HighlightStyle)> = Vec::new();
    let mut style = HighlightStyle::default();
    let mut stack: Vec<HighlightStyle> = Vec::new();

    loop {
        match parser.next() {
            Some(Event::Text(t)) => segments.push((t.to_string(), style)),
            Some(Event::Code(c)) => {
                let mut s = style;
                s.background_color = Some(hsl(INLINE_CODE_BG));
                s.color = Some(hsl(INLINE_CODE_FG));
                segments.push((c.to_string(), s));
            }
            Some(Event::SoftBreak) => segments.push((" ".to_string(), style)),
            Some(Event::HardBreak) => segments.push(("\n".to_string(), style)),
            Some(Event::InlineMath(m)) | Some(Event::DisplayMath(m)) => {
                segments.push((m.to_string(), style))
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
                Tag::Link { .. } => {
                    stack.push(style);
                    style.underline = Some(UnderlineStyle {
                        thickness: px(1.0),
                        color: None,
                        wavy: false,
                    });
                    style.color = Some(hsl(LINK_COLOR));
                }
                Tag::Image { .. } => stack.push(style),
                _ => {}
            },
            Some(Event::End(tag)) => match tag {
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image => {
                    style = stack.pop().unwrap_or_default();
                }
                _ => break,
            },
            _ => break,
        }
    }

    // Merge consecutive segments that share an identical style.
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
    (text, highlights)
}

fn parse_table(parser: &mut Parser) -> Block {
    let mut head: Vec<Inline> = Vec::new();
    let mut rows: Vec<Vec<Inline>> = Vec::new();

    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::TableHead) => {
                head = parse_table_row(parser);
            }
            Event::Start(Tag::TableRow) => {
                rows.push(parse_table_row(parser));
            }
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
            Event::Start(Tag::TableCell) => cells.push(parse_inline(parser)),
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => break,
            _ => {}
        }
    }
    cells
}

fn parse_list(parser: &mut Parser, start: Option<u64>) -> Block {
    let mut items = Vec::new();
    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(Tag::Item) => items.push(parse_blocks(parser)),
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

fn parse_blocks(parser: &mut Parser) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some(ev) = parser.next() {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => blocks.push(Block::Paragraph(parse_inline(parser))),
                Tag::Heading { level, .. } => blocks.push(Block::Heading {
                    level: level as u32,
                    inline: parse_inline(parser),
                }),
                Tag::List(start) => blocks.push(parse_list(parser, start)),
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) if !l.is_empty() => Some(l.to_string()),
                        _ => None,
                    };
                    let mut code = String::new();
                    while let Some(ev2) = parser.next() {
                        match ev2 {
                            Event::Text(t) => code.push_str(&t),
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    blocks.push(Block::Code { lang, text: code });
                }
                Tag::BlockQuote(_) => blocks.push(Block::Quote(parse_blocks(parser))),
                Tag::Table(_) => blocks.push(parse_table(parser)),
                _ => {
                    // Unknown container: consume until its matching End.
                    let _ = parse_blocks(parser);
                }
            },
            Event::End(_) => break,
            Event::Rule => blocks.push(Block::Rule),
            Event::Html(h) => blocks.push(Block::Html(h.to_string())),
            _ => {}
        }
    }
    blocks
}

fn parse_document(source: &str) -> Vec<Block> {
    let mut parser = Parser::new_ext(source, Options::all());
    parse_blocks(&mut parser)
}

/// Convert a `file://` URL (as delivered by macOS `on_open_urls`) into a path.
fn url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if rest.is_empty() {
        return None;
    }
    Some(PathBuf::from(percent_decode(rest)))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

struct Theme {
    bg: Hsla,
    fg: Hsla,
    heading: Hsla,
    muted: Hsla,
    code_bg: Hsla,
    code_fg: Hsla,
    quote_border: Hsla,
    rule: Hsla,
}

impl Theme {
    fn light() -> Self {
        Theme {
            bg: hsl(0xffffff),
            fg: hsl(0x24292f),
            heading: hsl(0x1f2328),
            muted: hsl(0x59636e),
            code_bg: hsl(0xf6f8fa),
            code_fg: hsl(0x1f2328),
            quote_border: hsl(0xd1d9e0),
            rule: hsl(0xd1d9e0),
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_inline(inline: &Inline, color: Hsla, font_size: f32, weight: FontWeight) -> StyledText {
    let style = TextStyle {
        color,
        font_size: px(font_size).into(),
        line_height: relative(1.55),
        font_weight: weight,
        ..Default::default()
    };
    StyledText::new(inline.0.clone()).with_default_highlights(&style, inline.1.iter().cloned())
}

fn render_paragraph(inline: &Inline, t: &Theme) -> AnyElement {
    div()
        .w_full()
        .mb_2()
        .child(render_inline(inline, t.fg, 15.0, FontWeight::NORMAL))
        .into_any()
}

fn render_heading(level: u32, inline: &Inline, t: &Theme) -> AnyElement {
    let (size, top) = match level {
        1 => (28.0, 10.0),
        2 => (23.0, 8.0),
        3 => (19.0, 6.0),
        _ => (16.0, 6.0),
    };
    div()
        .w_full()
        .mt(px(top))
        .mb_1()
        .child(render_inline(inline, t.heading, size, FontWeight::BOLD))
        .into_any()
}

fn render_code_block(code: &Block, t: &Theme, uid: &mut usize) -> AnyElement {
    let Block::Code { lang, text } = code else {
        unreachable!()
    };
    let id = *uid;
    *uid += 1;
    let mut el = div()
        .id(ElementId::named_usize("code", id))
        .w_full()
        .my_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(t.code_bg)
        .overflow_scroll()
        .flex()
        .flex_col();
    if let Some(lang) = lang {
        el = el.child(
            div()
                .mb_1()
                .text_size(px(11.0))
                .text_color(t.muted)
                .child(lang.clone()),
        );
    }
    // Render each source line as its own element so newlines survive layout.
    for line in text.split('\n') {
        el = el.child(
            div()
                .font_family("Menlo")
                .text_size(px(13.0))
                .line_height(relative(1.5))
                .text_color(t.code_fg)
                .whitespace_nowrap()
                .child(line.to_string()),
        );
    }
    el.into_any()
}

fn render_quote(inner: &[Block], t: &Theme, uid: &mut usize) -> AnyElement {
    div()
        .w_full()
        .my_1()
        .pl_3()
        .border_l_2()
        .border_color(t.quote_border)
        .text_color(t.muted)
        .children(render_blocks(inner, t, uid))
        .into_any()
}

fn render_list(list: &Block, t: &Theme, uid: &mut usize) -> AnyElement {
    let Block::List {
        ordered,
        start,
        items,
    } = list
    else {
        unreachable!()
    };
    div()
        .w_full()
        .my_1()
        .flex()
        .flex_col()
        .gap_1()
        .children(items.iter().enumerate().map(|(idx, item)| {
            let marker = if *ordered {
                format!("{}.", start + idx as u64)
            } else {
                "•".to_string()
            };
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .w(px(18.0))
                        .text_right()
                        .text_color(t.muted)
                        .child(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(render_blocks(item, t, uid)),
                )
        }))
        .into_any()
}

fn render_table(table: &Block, t: &Theme, uid: &mut usize) -> AnyElement {
    let Block::Table { head, rows } = table else {
        unreachable!()
    };
    let render_cell = |inline: &Inline, bold: bool| {
        div()
            .flex_1()
            .px_2()
            .py_1()
            .child(render_inline(
                inline,
                t.fg,
                14.0,
                if bold {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                },
            ))
    };
    let header = if head.is_empty() {
        None
    } else {
        Some(
            div()
                .flex()
                .flex_row()
                .border_b_1()
                .border_color(t.rule)
                .bg(t.code_bg)
                .children(head.iter().map(|c| render_cell(c, true))),
        )
    };
    let id = *uid;
    *uid += 1;
    div()
        .id(ElementId::named_usize("table", id))
        .w_full()
        .my_2()
        .overflow_scroll()
        .child(
            div()
                .w_full()
                .border_1()
                .border_color(t.rule)
                .rounded_sm()
                .children(header)
                .children(rows.iter().map(|row| {
                    div()
                        .flex()
                        .flex_row()
                        .border_b_1()
                        .border_color(t.rule)
                        .children(row.iter().map(|c| render_cell(c, false)))
                })),
        )
        .into_any()
}

fn render_blocks(blocks: &[Block], t: &Theme, uid: &mut usize) -> Vec<AnyElement> {
    blocks
        .iter()
        .map(|block| match block {
            Block::Heading { level, inline } => render_heading(*level, inline, t),
            Block::Paragraph(inline) => render_paragraph(inline, t),
            Block::List { .. } => render_list(block, t, uid),
            Block::Code { .. } => render_code_block(block, t, uid),
            Block::Quote(inner) => render_quote(inner, t, uid),
            Block::Table { .. } => render_table(block, t, uid),
            Block::Rule => div().w_full().h_px().my_4().bg(t.rule).into_any(),
            Block::Html(h) => div()
                .w_full()
                .mb_2()
                .text_color(t.muted)
                .child(h.clone())
                .into_any(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct MarkdownView {
    path: Option<PathBuf>,
    doc: Vec<Block>,
    error: Option<String>,
}

impl MarkdownView {
    fn load(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return MarkdownView {
                path: None,
                doc: Vec::new(),
                error: Some("用法：mkd <文件.md>\n\n把一个 Markdown 文件路径传给 mkd 即可开始阅读。".into()),
            };
        };
        let mut view = MarkdownView {
            path: None,
            doc: Vec::new(),
            error: None,
        };
        // Initial load: no need to notify, the first render picks this up.
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                view.path = Some(path);
                view.doc = parse_document(&source);
            }
            Err(err) => {
                view.path = Some(path);
                view.error = Some(format!("无法读取文件：{err}"));
            }
        }
        view
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                self.path = Some(path);
                self.doc = parse_document(&source);
                self.error = None;
            }
            Err(err) => {
                self.path = Some(path);
                self.doc = Vec::new();
                self.error = Some(format!("无法读取文件：{err}"));
            }
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                self.doc = parse_document(&source);
                self.error = None;
            }
            Err(err) => self.error = Some(format!("无法读取文件：{err}")),
        }
        cx.notify();
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::light();
        let content: Vec<AnyElement> = if let Some(err) = &self.error {
            vec![div()
                .w_full()
                .p_4()
                .text_size(px(15.0))
                .text_color(t.muted)
                .child(err.clone())
                .into_any()]
        } else {
            let mut uid = 0;
            render_blocks(&self.doc, &t, &mut uid)
        };

        div()
            .size_full()
            .bg(t.bg)
            .flex()
            .flex_col()
            .child(
                div()
                    .id("scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(720.0))
                            .mx_auto()
                            .px_8()
                            .py_6()
                            .flex()
                            .flex_col()
                            .children(content),
                    ),
            )
    }
}

fn main() {
    let initial_path = env::args().nth(1).map(PathBuf::from);

    let app = Application::new();
    // Files opened from Finder / `open -a` arrive here (no `&mut App` available,
    // so stash the path and let the view's poll loop pick it up).
    app.on_open_urls(|urls| {
        if let Some(path) = urls.into_iter().find_map(|u| url_to_path(&u)) {
            *PENDING_PATH.lock().unwrap() = Some(path);
        }
    });

    app.run(|cx: &mut App| {
        let title: String = initial_path
            .as_deref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| "mkd".into());

        let window = cx
            .open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some(title.into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(860.0), px(680.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        cx.spawn(|this: WeakEntity<MarkdownView>, cx: &mut AsyncApp| {
                            let mut cx = cx.clone();
                            async move {
                                loop {
                                    Timer::after(Duration::from_millis(200)).await;
                                    let pending = PENDING_PATH.lock().unwrap().take();
                                    if let Some(path) = pending {
                                        this.update(&mut cx, |view, cx| view.open(path, cx)).ok();
                                    }
                                }
                            }
                        })
                        .detach();
                        MarkdownView::load(initial_path)
                    })
                },
            )
            .unwrap();

        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(move |_: &Reload, cx| {
            window
                .update(cx, |view, _window, cx| view.reload(cx))
                .ok();
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-r", Reload, None),
        ]);
    });
}
