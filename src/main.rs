//! mkd — a featherweight macOS-native Markdown reader & editor built on GPUI.
//!
//! Usage:
//!   mkd [path/to/file.md]
//!
//! Key bindings:
//!   cmd-q   quit
//!   cmd-r   reload the current file (preview mode)
//!   cmd-e   toggle edit / preview mode (WYSIWYG editing)
//!   cmd-s   save (edit mode)

mod editor;
mod parse;
mod render;
mod theme;

use std::env;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use gpui::{
    actions, App, Application, AnyElement, AsyncApp, Bounds, ClipboardItem, Context, EntityInputHandler,
    FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, MouseDownEvent, Pixels,
    Point, Render, Timer, TitlebarOptions, UTF16Selection, WeakEntity, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};

use crate::editor::{Editor, EditorElement, EditorSource};
use crate::parse::{Doc, parse_document};
use crate::render::{RenderCtx, render_blocks};
use crate::theme::Theme;

actions!(mdk, [Quit, Reload]);
actions!(mkd_edit, [
    EditLeft, EditRight, EditUp, EditDown, EditHome, EditEnd,
    SelectLeft, SelectRight, SelectUp, SelectDown, SelectHome, SelectEnd,
    Backspace, Delete, Enter, Tab, SelectAll, Copy, Cut, Paste,
    ToggleEdit, Save,
]);

/// Files handed to the app by macOS (Finder double-click / `open -a`) land here,
/// then the view's poll loop picks them up.
static PENDING_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

struct MarkdownView {
    path: Option<PathBuf>,
    doc: Doc,
    error: Option<String>,
    /// 最近一次从文件读入/保存的源码（编辑模式的基准）。
    raw_source: String,
    edit_mode: bool,
    editor: Editor,
    focus_handle: FocusHandle,
}

impl MarkdownView {
    fn empty(focus: FocusHandle) -> Self {
        MarkdownView {
            path: None,
            doc: Doc {
                title: None,
                description: None,
                blocks: Vec::new(),
                footnotes: Vec::new(),
                headings: Vec::new(),
            },
            error: None,
            raw_source: String::new(),
            edit_mode: false,
            editor: Editor::new(""),
            focus_handle: focus,
        }
    }

    fn load(path: Option<PathBuf>, focus: FocusHandle) -> Self {
        let Some(path) = path else {
            let mut v = Self::empty(focus);
            v.error = Some("用法：mkd <文件.md>\n\n把一个 Markdown 文件路径传给 mkd 即可开始阅读。".into());
            return v;
        };
        let mut view = Self::empty(focus);
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
                self.raw_source = source.clone();
                self.doc = parse_document(&source, base_dir.as_deref());
                self.error = None;
            }
            Err(err) => {
                self.path = Some(path);
                self.raw_source = String::new();
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
        self.edit_mode = false;
    }

    fn reparse(&mut self) {
        let base_dir = self.path.as_ref().and_then(|p| p.parent().map(|p| p.to_path_buf()));
        self.doc = parse_document(&self.raw_source, base_dir.as_deref());
        self.error = None;
    }

    fn toggle_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit_mode {
            // 退出编辑：把编辑内容作为新的预览源
            self.raw_source = self.editor.to_source();
            self.reparse();
            self.edit_mode = false;
            self.editor.dirty = false;
        } else {
            self.editor = Editor::new(&self.raw_source);
            self.edit_mode = true;
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.edit_mode {
            return;
        }
        let source = self.editor.to_source();
        let saved = self
            .path
            .as_ref()
            .map(|p| std::fs::write(p, &source).is_ok())
            .unwrap_or(false);
        if saved {
            self.raw_source = source;
            self.editor.dirty = false;
            self.reparse();
        }
        cx.notify();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.load_path(path);
        cx.notify();
    }

    // ---- 编辑按键处理 ----

    fn edit_left(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_left(extend);
        cx.notify();
    }
    fn edit_right(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_right(extend);
        cx.notify();
    }
    fn edit_up(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_up(extend);
        cx.notify();
    }
    fn edit_down(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_down(extend);
        cx.notify();
    }
    fn edit_home(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_home(extend);
        cx.notify();
    }
    fn edit_end(&mut self, extend: bool, cx: &mut Context<Self>) {
        self.editor.move_end(extend);
        cx.notify();
    }
    fn edit_backspace(&mut self, cx: &mut Context<Self>) {
        self.editor.backspace();
        cx.notify();
    }
    fn edit_delete(&mut self, cx: &mut Context<Self>) {
        self.editor.delete();
        cx.notify();
    }
    fn edit_enter(&mut self, cx: &mut Context<Self>) {
        self.editor.insert_newline();
        cx.notify();
    }
    fn edit_tab(&mut self, cx: &mut Context<Self>) {
        self.editor.insert_tab();
        cx.notify();
    }
    fn edit_select_all(&mut self, cx: &mut Context<Self>) {
        self.editor.select_all();
        cx.notify();
    }
    fn edit_copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.editor.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    fn edit_cut(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.editor.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.editor.delete_selection();
            cx.notify();
        }
    }
    fn edit_paste(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.editor.insert_text(&text);
            cx.notify();
        }
    }

    fn editor_toolbar(&self, t: &Theme) -> AnyElement {
        let label = if self.editor.dirty {
            "编辑模式 ● 未保存"
        } else {
            "编辑模式"
        };
        div()
            .w_full()
            .px_4()
            .py_1()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(t.rule)
            .bg(t.neutral_bg)
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(if self.editor.dirty { t.warn_accent } else { t.muted })
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .text_right()
                    .text_size(px(12.0))
                    .text_color(t.muted)
                    .child("⌘E 切换  ⌘S 保存"),
            )
            .into_any()
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::light();

        if self.edit_mode {
            let toolbar = self.editor_toolbar(&t);
            return div()
                .size_full()
                .bg(t.bg)
                .flex()
                .flex_col()
                .key_context("mkd-editor")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(|this, _: &EditLeft, _w, cx| this.edit_left(false, cx)))
                .on_action(cx.listener(|this, _: &EditRight, _w, cx| this.edit_right(false, cx)))
                .on_action(cx.listener(|this, _: &EditUp, _w, cx| this.edit_up(false, cx)))
                .on_action(cx.listener(|this, _: &EditDown, _w, cx| this.edit_down(false, cx)))
                .on_action(cx.listener(|this, _: &EditHome, _w, cx| this.edit_home(false, cx)))
                .on_action(cx.listener(|this, _: &EditEnd, _w, cx| this.edit_end(false, cx)))
                .on_action(cx.listener(|this, _: &SelectLeft, _w, cx| this.edit_left(true, cx)))
                .on_action(cx.listener(|this, _: &SelectRight, _w, cx| this.edit_right(true, cx)))
                .on_action(cx.listener(|this, _: &SelectUp, _w, cx| this.edit_up(true, cx)))
                .on_action(cx.listener(|this, _: &SelectDown, _w, cx| this.edit_down(true, cx)))
                .on_action(cx.listener(|this, _: &SelectHome, _w, cx| this.edit_home(true, cx)))
                .on_action(cx.listener(|this, _: &SelectEnd, _w, cx| this.edit_end(true, cx)))
                .on_action(cx.listener(|this, _: &Backspace, _w, cx| this.edit_backspace(cx)))
                .on_action(cx.listener(|this, _: &Delete, _w, cx| this.edit_delete(cx)))
                .on_action(cx.listener(|this, _: &Enter, _w, cx| this.edit_enter(cx)))
                .on_action(cx.listener(|this, _: &Tab, _w, cx| this.edit_tab(cx)))
                .on_action(cx.listener(|this, _: &SelectAll, _w, cx| this.edit_select_all(cx)))
                .on_action(cx.listener(|this, _: &Copy, _w, cx| this.edit_copy(cx)))
                .on_action(cx.listener(|this, _: &Cut, _w, cx| this.edit_cut(cx)))
                .on_action(cx.listener(|this, _: &Paste, _w, cx| this.edit_paste(cx)))
                .on_action(cx.listener(|this, _: &ToggleEdit, w, cx| this.toggle_edit(w, cx)))
                .on_action(cx.listener(|this, _: &Save, _w, cx| this.save(cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _w, cx| {
                    if let Some((line, dcol)) = this.editor.pos_for_point(ev.position) {
                        this.editor.cursor_line = line;
                        this.editor.cursor_col = this.editor.source_col_for_display(line, dcol);
                        this.editor.sel_start = None;
                        cx.notify();
                    }
                }))
                .cursor_text()
                .child(toolbar)
                .child(
                    div()
                        .id("editor-scroll")
                        .flex_1()
                        .overflow_y_scroll()
                        .child(
                            div()
                                .w_full()
                                .max_w(px(720.0))
                                .mx_auto()
                                .px_8()
                                .py_4()
                                .child(EditorElement { input: cx.entity() }),
                        ),
                );
        }

        // ---- 预览模式 ----
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
            .key_context("mkd")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &ToggleEdit, w, cx| this.toggle_edit(w, cx)))
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

impl Focusable for MarkdownView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EditorSource for MarkdownView {
    fn editor(&self) -> &Editor {
        &self.editor
    }
    fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }
    fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for MarkdownView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if !self.edit_mode {
            return None;
        }
        let e = &self.editor;
        let (sl, sc) = e.pos_from_utf16(range.start);
        let (el, ec) = e.pos_from_utf16(range.end);
        adjusted.replace(range);
        Some(if sl == el {
            e.lines[sl][sc..ec].to_string()
        } else {
            let mut out = e.lines[sl][sc..].to_string();
            for l in sl + 1..el {
                out.push('\n');
                out.push_str(&e.lines[l]);
            }
            out.push('\n');
            out.push_str(&e.lines[el][..ec]);
            out
        })
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !self.edit_mode {
            return None;
        }
        let e = &self.editor;
        let (sl, sc) = e.sel_start?;
        let cur = (e.cursor_line, e.cursor_col);
        let s16 = e.utf16_from_pos(sl, sc);
        let c16 = e.utf16_from_pos(cur.0, cur.1);
        Some(UTF16Selection {
            range: s16.min(c16)..s16.max(c16),
            reversed: (sl, sc) > cur,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if !self.edit_mode {
            return None;
        }
        let e = &self.editor;
        let (ms, me) = e.marked?;
        let line = e.cursor_line;
        Some(e.utf16_from_pos(line, ms)..e.utf16_from_pos(line, me))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.editor.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.edit_mode {
            return;
        }
        if let Some(r) = range {
            self.editor.replace_utf16_range(r);
        }
        self.editor.insert_text(text);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.edit_mode {
            return;
        }
        if let Some(r) = range {
            self.editor.replace_utf16_range(r);
        }
        self.editor.marked = None;
        if !new_text.contains('\n') {
            let start = self.editor.cursor_col;
            self.editor.insert_text(new_text);
            self.editor.marked = Some((start, self.editor.cursor_col));
        } else {
            self.editor.insert_text(new_text);
        }
        if let Some(ns) = new_selected_range {
            let (l, c) = self.editor.pos_from_utf16(ns.start);
            self.editor.cursor_line = l;
            self.editor.cursor_col = c;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if !self.edit_mode {
            return None;
        }
        let e = &self.editor;
        let (sl, sc) = e.pos_from_utf16(range.start);
        let (_el, ec) = e.pos_from_utf16(range.end);
        e.bounds_for_range_utf8(sl, sc, ec)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        if !self.edit_mode {
            return None;
        }
        let e = &self.editor;
        let (line, dcol) = e.pos_for_point(point)?;
        let scol = e.source_col_for_display(line, dcol);
        Some(e.utf16_from_pos(line, scol))
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let mut edit_mode_start = false;
    let mut initial_path: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--edit" => edit_mode_start = true,
            _ => {
                if !a.starts_with('-') && initial_path.is_none() {
                    initial_path = Some(PathBuf::from(a));
                }
            }
        }
    }

    let app = Application::new();
    // Files opened from Finder / `open -a` arrive here (no `&mut App` available,
    // so stash the path and let the view's poll loop pick it up).
    app.on_open_urls(|urls| {
        if let Some(path) = urls.into_iter().find_map(|u| url_to_path(&u)) {
            *PENDING_PATH.lock().unwrap() = Some(path);
        }
    });

    app.run(move |cx: &mut App| {
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
                        MarkdownView::load(initial_path, cx.focus_handle())
                    })
                },
            )
            .unwrap();

        if edit_mode_start {
            window
                .update(cx, |view, window, cx| view.toggle_edit(window, cx))
                .ok();
        }
        cx.activate(true);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(move |_: &Reload, cx| {
            window
                .update(cx, |view, _window, cx| view.reload(cx))
                .ok();
        });
        cx.bind_keys([
            // 全局
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-r", Reload, Some("mkd")),
            KeyBinding::new("cmd-e", ToggleEdit, Some("mkd")),
            KeyBinding::new("cmd-e", ToggleEdit, Some("mkd-editor")),
            // 编辑模式
            KeyBinding::new("cmd-s", Save, Some("mkd-editor")),
            KeyBinding::new("left", EditLeft, Some("mkd-editor")),
            KeyBinding::new("right", EditRight, Some("mkd-editor")),
            KeyBinding::new("up", EditUp, Some("mkd-editor")),
            KeyBinding::new("down", EditDown, Some("mkd-editor")),
            KeyBinding::new("home", EditHome, Some("mkd-editor")),
            KeyBinding::new("end", EditEnd, Some("mkd-editor")),
            KeyBinding::new("shift-left", SelectLeft, Some("mkd-editor")),
            KeyBinding::new("shift-right", SelectRight, Some("mkd-editor")),
            KeyBinding::new("shift-up", SelectUp, Some("mkd-editor")),
            KeyBinding::new("shift-down", SelectDown, Some("mkd-editor")),
            KeyBinding::new("shift-home", SelectHome, Some("mkd-editor")),
            KeyBinding::new("shift-end", SelectEnd, Some("mkd-editor")),
            KeyBinding::new("backspace", Backspace, Some("mkd-editor")),
            KeyBinding::new("delete", Delete, Some("mkd-editor")),
            KeyBinding::new("enter", Enter, Some("mkd-editor")),
            KeyBinding::new("tab", Tab, Some("mkd-editor")),
            KeyBinding::new("cmd-a", SelectAll, Some("mkd-editor")),
            KeyBinding::new("cmd-c", Copy, Some("mkd-editor")),
            KeyBinding::new("cmd-x", Cut, Some("mkd-editor")),
            KeyBinding::new("cmd-v", Paste, Some("mkd-editor")),
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
