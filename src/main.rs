//! mkd — a featherweight macOS-native Markdown reader built on GPUI.
//!
//! Usage:
//!   mkd [path/to/file.md]
//!
//! Key bindings:
//!   cmd-q  quit
//!   cmd-r  reload the current file

mod parse;
mod render;
mod theme;

use std::env;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use gpui::{
    actions, App, Application, AnyElement, AsyncApp, Bounds, Context, FontWeight, KeyBinding,
    Render, Timer, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};

use crate::parse::{Doc, parse_document};
use crate::render::{RenderCtx, render_blocks};
use crate::theme::Theme;

actions!(mdk, [Quit, Reload]);

/// Files handed to the app by macOS (Finder double-click / `open -a`) land here,
/// then the view's poll loop picks them up.
static PENDING_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

struct MarkdownView {
    path: Option<PathBuf>,
    doc: Doc,
    error: Option<String>,
}

impl MarkdownView {
    fn load(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return MarkdownView {
                path: None,
                doc: Doc {
                    title: None,
                    description: None,
                    blocks: Vec::new(),
                    footnotes: Vec::new(),
                    headings: Vec::new(),
                },
                error: Some("用法：mkd <文件.md>\n\n把一个 Markdown 文件路径传给 mkd 即可开始阅读。".into()),
            };
        };
        let mut view = MarkdownView {
            path: None,
            doc: Doc {
                title: None,
                description: None,
                blocks: Vec::new(),
                footnotes: Vec::new(),
                headings: Vec::new(),
            },
            error: None,
        };
        view.load_path(path);
        view
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.load_path(path);
        cx.notify();
    }

    fn load_path(&mut self, path: PathBuf) {
        let base_dir = path.parent().map(|p| p.to_path_buf());
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                self.path = Some(path);
                self.doc = parse_document(&source, base_dir.as_deref());
                self.error = None;
            }
            Err(err) => {
                self.path = Some(path);
                self.doc = Doc {
                    title: None,
                    description: None,
                    blocks: Vec::new(),
                    footnotes: Vec::new(),
                    headings: Vec::new(),
                };
                self.error = Some(format!("无法读取文件：{err}"));
            }
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let base_dir = path.parent().map(|p| p.to_path_buf());
        match std::fs::read_to_string(&path) {
            Ok(source) => {
                self.doc = parse_document(&source, base_dir.as_deref());
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
        let mut content: Vec<AnyElement> = Vec::new();

        if let Some(err) = &self.error {
            content.push(
                div()
                    .w_full()
                    .p_4()
                    .text_size(px(15.0))
                    .text_color(t.muted)
                    .child(err.clone())
                    .into_any(),
            );
        } else {
            // Frontmatter-derived title / description.
            if let Some(title) = &self.doc.title {
                content.push(
                    div()
                        .w_full()
                        .mt_1()
                        .mb_1()
                        .text_size(px(30.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.heading)
                        .child(title.clone())
                        .into_any(),
                );
            }
            if let Some(desc) = &self.doc.description {
                content.push(
                    div()
                        .w_full()
                        .mb_3()
                        .text_size(px(15.0))
                        .text_color(t.muted)
                        .child(desc.clone())
                        .into_any(),
                );
            }

            let mut uid = 0;
            let ctx = RenderCtx {
                theme: &t,
                headings: &self.doc.headings,
            };
            content.extend(render_blocks(&self.doc.blocks, &ctx, &mut uid));

            // Footnotes, rendered at the end with a divider.
            if !self.doc.footnotes.is_empty() {
                content.push(div().w_full().h_px().my_4().bg(t.rule).into_any());
                for (label, blocks) in &self.doc.footnotes {
                    content.push(
                        div()
                            .w_full()
                            .mb_2()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.0))
                                    .text_color(t.muted)
                                    .child(format!("[{label}]")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(12.0))
                                    .children(render_blocks(blocks, &ctx, &mut uid)),
                            )
                            .into_any(),
                    );
                }
            }
        }

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
                                        this.update(&mut cx, |view, cx| view.open(path, cx))
                                            .ok();
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
