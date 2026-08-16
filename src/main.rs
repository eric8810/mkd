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
mod ops;
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
    ScrollHandle, point,
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
    Backspace, Delete, Enter, Tab, ShiftTab, SelectAll, Copy, Cut, Paste,
    ToggleEdit, Save, Undo, Redo, HardBreak,
    DeleteToLineEnd, DeleteToLineStart, DeleteWordBack, DeleteWordForward,
    ToggleBold, ToggleItalic, ToggleCode, ToggleStrike, InsertLink,
    SetParagraph, SetHeading1, SetHeading2, SetHeading3, SetCodeBlock, SetQuote,
    WrapBulletList, WrapOrderedList, WrapTaskList,
    Find, FindNext, FindClose,
    WordLeft, WordRight,
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
    scroll_handle: ScrollHandle,
    // 查找（FND-01）
    find_open: bool,
    find_redirect: bool,
    find_query: String,
    find_index: usize,
    find_matches: Vec<(usize, usize)>,
    /// SAV-07：待确认的新文件路径（有未保存更改时）。
    pending_open: Option<PathBuf>,
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
            scroll_handle: ScrollHandle::new(),
            find_open: false,
            find_redirect: false,
            find_query: String::new(),
            find_index: 0,
            find_matches: Vec::new(),
            pending_open: None,
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
        if self.edit_mode && self.editor.dirty {
            // SAV-07：有未保存更改，先确认再切换
            self.pending_open = Some(path);
        } else {
            self.load_path(path);
        }
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
            // NAV-10：光标闪烁定时器
            let this = cx.entity().downgrade();
            cx.spawn(|_, cx: &mut AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    loop {
                        Timer::after(Duration::from_millis(530)).await;
                        this.update(&mut cx, |view, cx| {
                            view.editor.blink_on = !view.editor.blink_on;
                            cx.notify();
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if !self.edit_mode {
            return;
        }
        let mut source = self.editor.to_source();
        // SAV-03：保留原文件尾部换行（往返保真）
        if self.raw_source.ends_with('\n') && !source.ends_with('\n') {
            source.push('\n');
        }
        let saved = self
            .path
            .as_ref()
            .map(|p| std::fs::write(p, &source).is_ok())
            .unwrap_or(false);
        if saved {
            self.raw_source = source;
            self.editor.dirty = false;
            self.error = None;
            self.reparse();
        } else {
            // SAV-06：保存失败明确报错，编辑内容保留在内存
            self.error = Some("保存失败：无法写入文件（磁盘只读或已满？）。编辑内容未丢失。".into());
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
            self.editor.apply(&ops::Op::Paste(text));
            cx.notify();
        }
    }

    /// SCR-01：光标移出可视区时自动滚动跟随。
    fn ensure_cursor_visible(&mut self, window: &mut Window) {
        let Some(bounds) = self.editor.last_bounds else {
            return;
        };
        let line_h = self.editor.line_height;
        let y_top = bounds.top() + px(self.editor.cursor_line as f32 * line_h);
        let y = y_top.to_f64() as f32;
        let offset_y = self.scroll_handle.offset().y.to_f64() as f32;
        let viewport_h = window.bounds().size.height.to_f64() as f32 - 60.0;
        if y < offset_y {
            self.scroll_handle.set_offset(point(px(0.0), px(y)));
        } else if y + line_h > offset_y + viewport_h {
            self.scroll_handle
                .set_offset(point(px(0.0), px(y + line_h - viewport_h)));
        }
    }

    /// FND-01：计算匹配位置（字符索引，供渲染与跳转）。
    fn find_all(&mut self) {
        self.editor.find_matches.clear();
        self.editor.find_len = self.find_query.chars().count();
        if self.find_query.is_empty() {
            self.editor.find_index = 0;
            return;
        }
        for (li, line) in self.editor.lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let mut start = 0;
            while let Some(rel) = chars[start..]
                .iter()
                .position(|&c| line[start..].starts_with(self.find_query.as_str()))
            {
                let col = start + rel;
                // 校验该字符位置确实匹配 query（避免跨码点错位）
                let byte_col = chars[..col].iter().map(|c| c.len_utf8()).sum::<usize>();
                if line[byte_col..].starts_with(self.find_query.as_str()) {
                    self.editor.find_matches.push((li, col));
                }
                start = col + 1;
                if start >= chars.len() {
                    break;
                }
            }
        }
        if self.editor.find_index >= self.editor.find_matches.len() {
            self.editor.find_index = self.editor.find_matches.len().saturating_sub(1);
        }
    }

    /// 跳到下一个/上一个匹配。
    fn find_jump(&mut self, window: &mut Window, dir: i32) {
        let n = self.editor.find_matches.len();
        if n == 0 {
            return;
        }
        self.editor.find_index =
            ((self.editor.find_index as i32 + dir).rem_euclid(n as i32)) as usize;
        let (line, col) = self.editor.find_matches[self.editor.find_index];
        self.editor.cursor_line = line;
        self.editor.cursor_col = col;
        self.editor.find_len = self.find_query.chars().count();
        self.ensure_cursor_visible(window);
    }

    fn find_open(&mut self) {
        self.find_open = true;
        self.find_redirect = true;
        self.find_all();
    }

    /// 用当前选区填充查找词（标准编辑器行为）。
    fn find_with_selection(&mut self) {
        let sel = self.editor.selection_bounds();
        if let Some(((sl, sc), (el, ec))) = sel {
            if (sl, sc) != (el, ec) {
                let mut q = self.editor.line(sl)[sc..el.min(self.editor.line(sl).len())].to_string();
                if el > sl {
                    // 跨行：取首行剩余 + 末行开头
                    q = self.editor.line(sl)[sc..].to_string();
                    q.push_str(&self.editor.line(el)[..ec.min(self.editor.line(el).len())]);
                }
                self.find_query = q;
            }
        }
        self.find_open();
    }

    fn find_close(&mut self) {
        self.find_open = false;
        self.find_redirect = false;
    }

    /// 查找条渲染。
    fn find_bar(&self, t: &Theme) -> AnyElement {
        let count = self.editor.find_matches.len();
        let status = if self.find_query.is_empty() {
            "输入查找内容".to_string()
        } else if count == 0 {
            "无匹配".to_string()
        } else {
            format!("{}/{}", self.find_index + 1, count)
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
            .bg(t.info_bg)
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(t.info_accent)
                    .child("🔍".to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(t.fg)
                    .child(if self.find_query.is_empty() {
                        "输入查找内容".to_string()
                    } else {
                        self.find_query.clone()
                    }),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(t.muted)
                    .child(status),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(t.muted)
                    .child("Enter 下一个  Esc 关闭".to_string()),
            )
            .into_any()
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
            let find_bar = if self.find_open {
                Some(self.find_bar(&t))
            } else {
                None
            };
            return div()
                .size_full()
                .bg(t.bg)
                .flex()
                .flex_col()
                .key_context("mkd-editor")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(|this, _: &EditLeft, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Left, false));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &EditRight, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Right, false));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &EditUp, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Up, false));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &EditDown, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Down, false));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &EditHome, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Home, false));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &EditEnd, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::End, false));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectLeft, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Left, true));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectRight, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Right, true));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectUp, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Up, true));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectDown, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Down, true));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectHome, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::Home, true));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectEnd, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Move(ops::Direction::End, true));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Backspace, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Backspace);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Delete, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Delete);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Enter, window, cx| {
                    if this.find_redirect {
                        this.find_jump(window, 1);
                        cx.notify();
                    } else {
                        this.ensure_cursor_visible(window);
                        this.editor.apply(&ops::Op::Newline);
                        cx.notify();
                    }
                }))
                .on_action(cx.listener(|this, _: &Find, _w, cx| {
                    this.find_with_selection();
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &WordLeft, _w, cx| {
                    this.editor.move_word_left();
                    this.ensure_cursor_visible(_w);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &WordRight, _w, cx| {
                    this.editor.move_word_right();
                    this.ensure_cursor_visible(_w);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &FindNext, window, cx| {
                    this.find_jump(window, 1);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &FindClose, _w, cx| {
                    if this.find_open {
                        this.find_close();
                    } else if this.editor.sel_start.is_some() {
                        // SEL-07：Esc 收起选区回到光标
                        this.editor.sel_start = None;
                    }
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Tab, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Tab);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectAll, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SelectAll);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Copy, _w, cx| this.edit_copy(cx)))
                .on_action(cx.listener(|this, _: &Cut, _w, cx| this.edit_cut(cx)))
                .on_action(cx.listener(|this, _: &Paste, _w, cx| this.edit_paste(cx)))
                .on_action(cx.listener(|this, _: &ShiftTab, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::ShiftTab);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Undo, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Undo);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &Redo, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::Redo);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &HardBreak, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::HardBreak);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &DeleteToLineEnd, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::DeleteToLineEnd);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &DeleteToLineStart, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::DeleteToLineStart);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &DeleteWordBack, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::DeleteWordBack);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &DeleteWordForward, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::DeleteWordForward);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleBold, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::ToggleMark(ops::Mark::Bold));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleItalic, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::ToggleMark(ops::Mark::Italic));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleCode, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::ToggleMark(ops::Mark::Code));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleStrike, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::ToggleMark(ops::Mark::Strike));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &InsertLink, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::InsertLink);
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SetParagraph, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SetBlockType(ops::BlockType::Paragraph));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SetHeading1, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SetBlockType(ops::BlockType::Heading(1)));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SetHeading2, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SetBlockType(ops::BlockType::Heading(2)));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SetHeading3, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SetBlockType(ops::BlockType::Heading(3)));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SetCodeBlock, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SetBlockType(ops::BlockType::CodeBlock));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SetQuote, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::SetBlockType(ops::BlockType::Quote));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &WrapBulletList, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::WrapList(ops::ListKind::Bullet));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &WrapOrderedList, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::WrapList(ops::ListKind::Ordered));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &WrapTaskList, window, cx| {
                    this.ensure_cursor_visible(window);
                    this.editor.apply(&ops::Op::WrapList(ops::ListKind::Task));
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ToggleEdit, w, cx| this.toggle_edit(w, cx)))
                .on_action(cx.listener(|this, _: &Save, _w, cx| this.save(cx)))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _w, cx| {
                    if let Some((line, dcol)) = this.editor.pos_for_point(ev.position) {
                        this.find_redirect = false;
                        if ev.click_count >= 3 {
                            // 三击：选整行（SEL-08）
                            this.editor.select_line_at(line);
                            this.editor.drag_anchor = None;
                        } else if ev.click_count == 2 {
                            // 双击：选词（SEL-04）
                            this.editor.select_word_at(line, dcol);
                            this.editor.drag_anchor = None;
                        } else if ev.modifiers.shift {
                            // Shift+点击：扩展选区
                            if this.editor.sel_start.is_none() {
                                this.editor.sel_start = Some((this.editor.cursor_line, this.editor.cursor_col));
                            }
                            this.editor.cursor_line = line;
                            this.editor.cursor_col = this.editor.source_col_for_display(line, dcol);
                            this.editor.drag_anchor = None;
                        } else {
                            // 单击：定位光标 + 开始拖选（SEL-02）
                            this.editor.cursor_line = line;
                            this.editor.cursor_col = this.editor.source_col_for_display(line, dcol);
                            this.editor.sel_start = None;
                            this.editor.drag_anchor = Some((line, this.editor.cursor_col));
                        }
                        cx.notify();
                    }
                }))
                .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
                    // 拖选：按住左键移动扩展选区
                    if let Some(anchor) = this.editor.drag_anchor {
                        // SEL-09：拖出视口边缘自动滚动
                        if let Some(b) = this.editor.last_bounds {
                            let oy = this.scroll_handle.offset().y;
                            if ev.position.y < b.top() {
                                this.scroll_handle
                                    .set_offset(point(px(0.0), (oy - px(16.0)).max(px(0.0))));
                            } else if ev.position.y > b.bottom() {
                                this.scroll_handle.set_offset(point(px(0.0), oy + px(16.0)));
                            }
                        }
                        if let Some((line, dcol)) = this.editor.pos_for_point(ev.position) {
                            this.editor.cursor_line = line;
                            this.editor.cursor_col = this.editor.source_col_for_display(line, dcol);
                            this.editor.sel_start = Some(anchor);
                            this.ensure_cursor_visible(window);
                            cx.notify();
                        }
                    }
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _ev, _w, cx| {
                    this.editor.drag_anchor = None;
                    cx.notify();
                }))
                .cursor_text()
                .child(toolbar)
                .when_some(
                    self.error.clone(),
                    |this, err| {
                        this.child(
                            div()
                                .w_full()
                                .px_4()
                                .py_1()
                                .bg(gpui::rgb(0xd32f2f))
                                .text_size(px(12.0))
                                .text_color(gpui::white())
                                .child(err.clone()),
                        )
                    },
                )
                .when_some(
                    self.pending_open.clone(),
                    |this, _path| {
                        this.child(
                            div()
                                .w_full()
                                .px_4()
                                .py_1()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .bg(gpui::rgb(0xfff3cd))
                                .border_b_1()
                                .border_color(gpui::rgb(0xf0d9a8))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px(12.0))
                                        .text_color(gpui::rgb(0x8a6d3b))
                                        .child("有未保存的更改，切换文件将丢失编辑内容。".to_string()),
                                )
                                .child(
                                    div()
                                        .id("switch-cancel")
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .bg(gpui::rgb(0xe9ecef))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                            this.pending_open = None;
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .text_color(gpui::rgb(0x333333))
                                                .child("保留并取消".to_string()),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("switch-ok")
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .bg(gpui::rgb(0xd32f2f))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _w, cx| {
                                            if let Some(path) = this.pending_open.take() {
                                                this.load_path(path);
                                            }
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .text_color(gpui::white())
                                                .child("丢弃并切换".to_string()),
                                        ),
                                ),
                        )
                    },
                )
                .when_some(find_bar, |this, bar| this.child(bar))
                .child(
                    div()
                        .id("editor-scroll")
                        .flex_1()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll_handle)
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
        if self.find_redirect {
            // 查找模式：输入进查询框
            self.find_query = text.to_string();
            self.find_index = 0;
            self.find_all();
            cx.notify();
            return;
        }
        if let Some(r) = range {
            self.editor.replace_utf16_range(r);
        }
        self.editor.apply(&ops::Op::Type(text.to_string()));
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
        cx.on_action(move |_: &Quit, cx| {
            // SAV-05：未保存更改时确认退出
            let w = window.clone();
            let dirty = w
                .update(cx, |view, _window, _cx| {
                    view.edit_mode && view.editor.dirty
                })
                .unwrap_or(false);
            if dirty {
                w.update(cx, |view, window, cx| {
                    let answer = window.prompt(
                        gpui::PromptLevel::Warning,
                        "有未保存的更改，确定放弃并退出？",
                        None,
                        &["放弃并退出", "取消"],
                        cx,
                    );
                    cx.spawn(|_this: WeakEntity<MarkdownView>, cx: &mut AsyncApp| { let cx = cx.clone(); async move {
                        if answer.await == Ok(0) {
                            cx.update(|cx| cx.quit()).ok();
                        }
                    }
                    })
                    .detach();
                    let _ = view;
                })
                .ok();
            } else {
                cx.quit();
            }
        });
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
            KeyBinding::new("cmd-z", Undo, Some("mkd-editor")),
            KeyBinding::new("cmd-shift-z", Redo, Some("mkd-editor")),
            KeyBinding::new("cmd-y", Redo, Some("mkd-editor")),
            KeyBinding::new("shift-enter", HardBreak, Some("mkd-editor")),
            KeyBinding::new("shift-tab", ShiftTab, Some("mkd-editor")),
            KeyBinding::new("cmd-b", ToggleBold, Some("mkd-editor")),
            KeyBinding::new("cmd-i", ToggleItalic, Some("mkd-editor")),
            KeyBinding::new("cmd-`", ToggleCode, Some("mkd-editor")),
            KeyBinding::new("cmd-shift-x", ToggleStrike, Some("mkd-editor")),
            KeyBinding::new("cmd-k", InsertLink, Some("mkd-editor")),
            KeyBinding::new("cmd-alt-0", SetParagraph, Some("mkd-editor")),
            KeyBinding::new("cmd-alt-1", SetHeading1, Some("mkd-editor")),
            KeyBinding::new("cmd-alt-2", SetHeading2, Some("mkd-editor")),
            KeyBinding::new("cmd-alt-3", SetHeading3, Some("mkd-editor")),
            KeyBinding::new("cmd-alt-c", SetCodeBlock, Some("mkd-editor")),
            KeyBinding::new("cmd-alt-q", SetQuote, Some("mkd-editor")),
            KeyBinding::new("cmd-shift-8", WrapBulletList, Some("mkd-editor")),
            KeyBinding::new("cmd-shift-9", WrapOrderedList, Some("mkd-editor")),
            KeyBinding::new("cmd-shift-7", WrapTaskList, Some("mkd-editor")),
            // macOS 编辑键（INP-12）
            KeyBinding::new("ctrl-h", Backspace, Some("mkd-editor")),
            KeyBinding::new("ctrl-d", Delete, Some("mkd-editor")),
            KeyBinding::new("ctrl-k", DeleteToLineEnd, Some("mkd-editor")),
            KeyBinding::new("ctrl-u", DeleteToLineStart, Some("mkd-editor")),
            KeyBinding::new("ctrl-w", DeleteWordBack, Some("mkd-editor")),
            KeyBinding::new("alt-backspace", DeleteWordBack, Some("mkd-editor")),
            KeyBinding::new("alt-delete", DeleteWordForward, Some("mkd-editor")),
            // NAV-07：单词级移动（macOS）
            KeyBinding::new("alt-left", WordLeft, Some("mkd-editor")),
            KeyBinding::new("alt-right", WordRight, Some("mkd-editor")),
            // 查找（FND-01）
            KeyBinding::new("cmd-f", Find, Some("mkd-editor")),
            KeyBinding::new("escape", FindClose, Some("mkd-editor")),
            KeyBinding::new("shift-enter", FindNext, Some("mkd-editor")),
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
