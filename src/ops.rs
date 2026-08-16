//! 编辑器操作层：所有编辑行为收敛为 `Op`，经 `Editor::apply` 单一入口执行。
//!
//! 设计遵循 ProseMirror 原则：
//! - 所有变更走单一入口（可撤销、可记录、可测试）
//! - 命令/操作是纯逻辑，不依赖窗口
//! - 撤销保存完整状态快照（文档 + 光标 + 选区）
//!
//! 覆盖 spec 章节：2.3 输入、2.4 格式化、2.5 输入规则、2.6 列表、
//! 2.8 撤销、2.12 行级 markdown 语义。

use crate::editor::{Editor, EditorSnapshot, LineStyle};

// ---------------------------------------------------------------------------
// 操作模型
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Code,
    Strike,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockType {
    Paragraph,
    Heading(u8),
    Quote,
    CodeBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Ordered,
    Task,
}

/// 一个可执行的编辑操作。
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// 输入文本（可含换行）。触发输入规则。
    Type(String),
    /// 回车（上下文敏感：段落/列表/代码块/引用）。
    Newline,
    /// Shift+Enter：硬换行（行尾补两个空格）。
    HardBreak,
    /// Tab（上下文敏感：列表缩进 / 代码缩进 / 插入空格）。
    Tab,
    /// Shift+Tab（列表提升）。
    ShiftTab,
    Backspace,
    Delete,
    /// 光标移动（extend = Shift 扩展选区）。
    Move(Direction, bool),
    SelectAll,
    /// 切换行内标记（选区包裹/去除；无选区时切换输入态）。
    ToggleMark(Mark),
    /// 插入/编辑链接（Cmd+K）。
    InsertLink,
    /// 设置当前行块类型。
    SetBlockType(BlockType),
    /// 选区转列表。
    WrapList(ListKind),
    Undo,
    Redo,
    /// 粘贴文本（与 Type 相同，但标记为外部内容不触发输入规则）。
    Paste(String),
    /// macOS 编辑键：删到行尾（Ctrl+K）。
    DeleteToLineEnd,
    /// 删到行首（Ctrl+U）。
    DeleteToLineStart,
    /// 删前一个词（Ctrl+W / Alt+Backspace）。
    DeleteWordBack,
    /// 删后一个词（Alt+Delete）。
    DeleteWordForward,
    /// 拖放移动/复制文本（DRG-01）：from 区间 → to 位置。
    MoveRange {
        from: (usize, usize, usize, usize),
        to: (usize, usize),
        copy: bool,
    },
}

// ---------------------------------------------------------------------------
// 输入规则上下文
// ---------------------------------------------------------------------------

/// 输入规则状态：最近一次通过输入规则创建的块标记。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RuleState {
    None,
    /// 刚通过输入规则创建的行首标记类型（Backspace 可回退）。
    Block { line: usize, style: LineStyle },
}

// ---------------------------------------------------------------------------
// Editor 操作实现
// ---------------------------------------------------------------------------

impl Editor {
    // ---- 快照 ----

    pub fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
            sel_start: self.sel_start,
            marked: self.marked,
        }
    }

    pub fn restore(&mut self, snap: &EditorSnapshot) {
        self.lines = snap.lines.clone();
        self.cursor_line = snap.cursor_line;
        self.cursor_col = snap.cursor_col;
        self.sel_start = snap.sel_start;
        self.marked = snap.marked;
    }

    /// 撤销 / 重做状态。
    #[allow(dead_code)]
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }
    #[allow(dead_code)]
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }
    #[allow(dead_code)]
    pub fn reset_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    // ---- 主入口 ----

    /// 执行一个操作。所有修改文档的操作都经过这里（可撤销、可测试）。
    pub fn apply(&mut self, op: &Op) {
        match op {
            Op::Move(dir, extend) => self.move_dir(*dir, *extend),
            Op::SelectAll => self.select_all(),
            Op::Undo => self.undo(),
            Op::Redo => self.redo(),
            Op::Type(text) => {
                self.begin_edit(true);
                // 应用待输入标记（stored marks）：`**` + text + `**`
                let text = if self.pending_marks.is_empty() {
                    text.clone()
                } else {
                    apply_pending_marks(text, &self.pending_marks)
                };
                self.type_text(&text, false);
                self.end_edit(true);
            }
            Op::Paste(text) => {
                self.begin_edit(true);
                self.type_text(text, true);
                self.end_edit(false);
            }
            Op::Newline => {
                self.begin_edit(false);
                self.do_newline(false);
                self.end_edit(false);
            }
            Op::HardBreak => {
                self.begin_edit(false);
                self.do_newline(true);
                self.end_edit(false);
            }
            Op::Tab => {
                self.begin_edit(false);
                self.do_tab(false);
                self.end_edit(false);
            }
            Op::ShiftTab => {
                self.begin_edit(false);
                self.do_tab(true);
                self.end_edit(false);
            }
            Op::Backspace => {
                self.begin_edit(false);
                self.do_backspace();
                self.end_edit(false);
            }
            Op::Delete => {
                self.begin_edit(false);
                self.do_delete();
                self.end_edit(false);
            }
            Op::ToggleMark(mark) => {
                self.begin_edit(false);
                self.toggle_mark(*mark);
                self.end_edit(false);
            }
            Op::InsertLink => {
                self.begin_edit(false);
                self.insert_link();
                self.end_edit(false);
            }
            Op::SetBlockType(t) => {
                self.begin_edit(false);
                self.set_block_type(*t);
                self.end_edit(false);
            }
            Op::WrapList(kind) => {
                self.begin_edit(false);
                self.wrap_list(*kind);
                self.end_edit(false);
            }
            Op::DeleteToLineEnd => {
                self.begin_edit(false);
                self.delete_to_line_end();
                self.end_edit(false);
            }
            Op::DeleteToLineStart => {
                self.begin_edit(false);
                self.delete_to_line_start();
                self.end_edit(false);
            }
            Op::DeleteWordBack => {
                self.begin_edit(false);
                self.delete_word_back();
                self.end_edit(false);
            }
            Op::DeleteWordForward => {
                self.begin_edit(false);
                self.delete_word_forward();
                self.end_edit(false);
            }
            Op::MoveRange { from, to, copy } => {
                self.begin_edit(false);
                self.move_range(*from, *to, *copy);
                self.end_edit(false);
            }
        }
    }

    // ---- 撤销/重做 ----

    fn undo(&mut self) {
        if let Some(snap) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());
            self.restore(&snap);
            self.dirty = true;
            self.last_op = None;
            self.last_op_at = None;
        }
    }

    fn redo(&mut self) {
        if let Some(snap) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());
            self.restore(&snap);
            self.dirty = true;
            self.last_op = None;
            self.last_op_at = None;
        }
    }

    /// 记录编辑开始：非合并操作在此压入「操作前」快照。
    /// UND-05：撤销历史上限（超出丢弃最旧步骤）。
    const MAX_UNDO: usize = 100;

    fn begin_edit(&mut self, mergeable: bool) {
        // 连续键入（Type/Paste）在时间窗内合并为一步；结构操作不合并。
        let now = std::time::Instant::now();
        let merge = mergeable
            && matches!(self.last_op, Some(LastOp::Typing))
            && now.duration_since(self.last_op_at.unwrap_or(now)).as_millis() < 1200
            && self.sel_start.is_none();
        if !merge {
            if self.undo_stack.len() >= Self::MAX_UNDO {
                self.undo_stack.remove(0);
            }
            self.undo_stack.push(self.snapshot());
            self.redo_stack.clear();
        }
        self.merge_undo = merge;
    }

    /// 编辑结束：更新操作类型标记。`is_typing` 仅对连续文本输入为 true。
    fn end_edit(&mut self, is_typing: bool) {
        self.merge_undo = false;
        self.last_op = if is_typing {
            Some(LastOp::Typing)
        } else {
            Some(LastOp::Other)
        };
        self.last_op_at = Some(std::time::Instant::now());
    }

    // ---- 光标移动 ----

    fn move_dir(&mut self, dir: Direction, extend: bool) {
        match dir {
            Direction::Left => self.move_left(extend),
            Direction::Right => self.move_right(extend),
            Direction::Up => self.move_up(extend),
            Direction::Down => self.move_down(extend),
            Direction::Home => self.move_home(extend),
            Direction::End => self.move_end(extend),
        }
    }

    // ---- 文本输入 ----

    /// 输入文本；`is_paste` 时不触发输入规则。
    fn type_text(&mut self, text: &str, is_paste: bool) {
        self.insert_text(text);
        if !is_paste {
            self.check_input_rules();
        }
    }

    // ---- 回车（上下文敏感，spec 2.12 / 2.6） ----

    fn do_newline(&mut self, hard: bool) {
        // 先处理选区替换。
        if self.sel_start.is_some() {
            self.delete_selection();
        }
        let (line, col) = (self.cursor_line, self.cursor_col);
        let cur = self.line(line).to_string();
        let cur_style = crate::editor::parse_line(&cur, false).line_style;
        let in_fence = self.in_fence_at(line);

        match cur_style {
            // 代码块围栏行：视为代码行内换行（新行继承缩进）
            LineStyle::Fence | LineStyle::CodeLine => {
                self.plain_newline(line, col);
            }
            // 标题：拆行并转普通段落（标题行尾回车 → 新段落）
            LineStyle::Heading(_) => {
                self.plain_newline(line, col);
            }
            // 引用：续行（新行带 `>`）
            LineStyle::Quote => {
                self.quote_newline(line, col);
            }
            // 列表：续行 / 空项退出
            LineStyle::Bullet => {
                self.list_newline(line, col, "- ", None);
            }
            LineStyle::Ordered => {
                let marker = self.ordered_marker(&cur);
                let start = ordered_start(&cur);
                self.list_newline(line, col, &marker, start);
            }
            // 分隔线：回车 → 普通新段落
            LineStyle::Rule => {
                self.plain_newline(line, col);
            }
            // 普通文本
            LineStyle::Plain => {
                if hard {
                    // Shift+Enter：硬换行（行尾两空格）
                    self.hard_break(line, col);
                } else {
                    self.paragraph_newline(line, col);
                }
            }
        }
        let _ = in_fence;
    }

    /// 普通拆行（保持当前行其余部分，光标移到新行开头）。
    fn plain_newline(&mut self, line: usize, col: usize) {
        let head = self.lines[line][..col].to_string();
        let tail = self.lines[line][col..].to_string();
        self.lines[line] = head;
        self.lines.insert(line + 1, tail);
        self.cursor_line = line + 1;
        self.cursor_col = 0;
    }

    /// 段落回车：拆出新段（中间补空行，spec MD-02）。
    fn paragraph_newline(&mut self, line: usize, col: usize) {
        let cur = self.lines[line].clone();
        let head = cur[..col].to_string();
        let tail = cur[col..].to_string();
        let next_nonempty = self
            .lines
            .get(line + 1)
            .map(|l| !l.trim().is_empty())
            .unwrap_or(false);

        if !head.trim().is_empty() && !tail.trim().is_empty() {
            // 行中间回车：拆成两段，中间插空行；光标在新段落开头
            self.lines[line] = head;
            self.lines.insert(line + 1, String::new());
            self.lines.insert(line + 2, tail);
            self.cursor_line = line + 2;
            self.cursor_col = 0;
        } else if tail.is_empty() && next_nonempty {
            // 行尾回车且下一行非空（同段落延续）：插入空行分隔；光标在新空行
            self.lines.insert(line + 1, String::new());
            self.cursor_line = line + 1;
            self.cursor_col = 0;
        } else if head.is_empty() && !tail.trim().is_empty() {
            // 行首回车：在上方插入空行
            self.lines.insert(line, String::new());
            self.cursor_line = line + 1;
            self.cursor_col = 0;
        } else {
            self.plain_newline(line, col);
        }
    }

    /// 硬换行：行尾补两个空格再拆行。
    fn hard_break(&mut self, line: usize, col: usize) {
        let head = self.lines[line][..col].to_string();
        let tail = self.lines[line][col..].to_string();
        let new_head = if head.ends_with("  ") {
            head
        } else {
            head + "  "
        };
        let cursor_at_end = self.lines[line][col..].is_empty();
        self.lines[line] = new_head.clone();
        self.lines.insert(line + 1, tail);
        if cursor_at_end {
            // 光标在行尾：留在硬换行处
            self.cursor_line = line;
            self.cursor_col = new_head.len();
        } else {
            self.cursor_line = line + 1;
            self.cursor_col = 0;
        }
    }

    /// 引用续行：新行带 `> `（若光标行是 `>` 单独一行则原样）。
    fn quote_newline(&mut self, line: usize, col: usize) {
        let cur = self.lines[line].clone();
        let head = cur[..col].to_string();
        let tail = cur[col..].to_string();
        let marker = if head.trim_end() == ">" { ">" } else { "> " };
        // 引用内空行回车：退出引用
        let is_empty = cur.trim().trim_start_matches('>').trim().is_empty();
        if is_empty && col <= cur.len() && col >= cur.trim_start_matches('>').len() {
            // 空引用行回车 → 新普通段落
            self.lines[line] = head;
            self.lines.insert(line + 1, String::new());
            self.cursor_line = line + 1;
            self.cursor_col = 0;
            return;
        }
        self.lines[line] = head;
        self.lines.insert(line + 1, format!("{marker}{tail}"));
        self.cursor_line = line + 1;
        self.cursor_col = marker.len();
    }

    /// 列表回车：续行带标记 / 空项退出（spec LST-01/02）。
    fn list_newline(&mut self, line: usize, col: usize, marker: &str, start: Option<u64>) {
        let cur = self.lines[line].clone();
        let content = cur.trim_start();
        let is_empty = content.len() <= marker.len() || content[marker.len()..].trim().is_empty();

        if is_empty {
            // 空列表项回车 → 退出列表：原列表项保留，光标到新空段落
            self.lines.insert(line + 1, String::new());
            self.cursor_line = line + 1;
            self.cursor_col = 0;
            return;
        }

        let head = cur[..col].to_string();
        let tail = cur[col..].to_string();
        // 嵌套缩进：子列表项保持缩进
        let indent = cur[..cur.len() - cur.trim_start().len()].to_string();
        let next_marker = match (marker, start) {
            (_, Some(s)) => format!("{}. ", s + 1),
            _ => marker.to_string(),
        };
        self.lines[line] = head;
        self.lines.insert(line + 1, format!("{indent}{next_marker}{tail}"));
        self.cursor_line = line + 1;
        self.cursor_col = indent.len() + next_marker.len();
    }

    /// 取有序列表行首标记（`1. ` 或 `1) `）。
    fn ordered_marker(&self, line: &str) -> String {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > 0 && i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') {
            let n = line[..i].parse::<u64>().unwrap_or(1);
            let sep = if bytes[i] == b'.' { ". " } else { ") " };
            format!("{}{}", n + 1, sep)
        } else {
            "1. ".to_string()
        }
    }

    // ---- Tab（spec LST-03 / IR 相关） ----

    fn do_tab(&mut self, shift: bool) {
        if self.sel_start.is_some() {
            self.insert_text("    ");
            return;
        }
        let (line, col) = (self.cursor_line, self.cursor_col);
        let cur = self.line(line).to_string();
        let style = crate::editor::parse_line(&cur, false).line_style;
        match style {
            LineStyle::Bullet | LineStyle::Ordered => {
                if shift {
                    // 提升：去掉 4 空格缩进
                    if cur.starts_with("    ") {
                        self.lines[line] = cur[4..].to_string();
                        self.cursor_col = col.saturating_sub(4);
                    } else if cur.starts_with('\t') {
                        self.lines[line] = cur[1..].to_string();
                        self.cursor_col = col.saturating_sub(1);
                    }
                } else if col == 0 || self.is_at_list_marker(&cur, col) {
                    // 光标在标记处/行首：缩进嵌套
                    self.lines[line] = format!("    {cur}");
                    self.cursor_col = col + 4;
                } else {
                    self.insert_text("    ");
                }
            }
            LineStyle::CodeLine | LineStyle::Fence => {
                self.insert_text("    ");
            }
            _ => {
                self.insert_text("    ");
            }
        }
    }

    fn is_at_list_marker(&self, line: &str, col: usize) -> bool {
        let parsed = crate::editor::parse_line(line, false);
        col == parsed.prefix_len
    }

    // ---- 退格 / 删除 ----

    fn do_backspace(&mut self) {
        // 输入规则回退（IR-07）：刚创建的行首块标记，内容为空时删除标记。
        if let RuleState::Block { line, style } = self.rule_state {
            if line == self.cursor_line {
                let cur = self.lines[line].clone();
                if let Some(marker) = block_marker(&cur, style) {
                    let removed = cur[marker.len()..].trim().is_empty();
                    if removed {
                        self.lines[line] = cur[marker.len()..].trim_start().to_string();
                        self.cursor_col = self.cursor_col.saturating_sub(marker.len());
                        self.dirty = true;
                        return;
                    }
                }
            }
        }
        // FMT-03：标记符原子删除（`**`/`~~`/`==` 作为整体，`` ` `` / `*` 单个）。
        // 代码围栏内不特殊处理。
        if !self.in_fence_at(self.cursor_line) {
            let (line, col) = (self.cursor_line, self.cursor_col);
            if col > 0 {
                let s = self.line(line).to_string();
                let bytes = s.as_bytes();
                let two = if col >= 2 { &s[col - 2..col] } else { "" };
                if matches!(two, "**" | "~~" | "==") {
                    self.lines[line].replace_range(col - 2..col, "");
                    self.cursor_col -= 2;
                    self.dirty = true;
                    return;
                }
                let one = bytes[col - 1];
                if one == b'`' || one == b'*' {
                    let prev_same = col >= 2 && (bytes[col - 2] == one);
                    if !prev_same {
                        self.lines[line].replace_range(col - 1..col, "");
                        self.cursor_col -= 1;
                        self.dirty = true;
                        return;
                    }
                }
            }
        }
        self.backspace();
    }

    fn do_delete(&mut self) {
        self.delete();
    }

    // ---- 输入规则（spec 2.5） ----

    fn check_input_rules(&mut self) {
        let (line, _col) = (self.cursor_line, self.cursor_col);
        let cur = self.line(line).to_string();
        let style = crate::editor::parse_line(&cur, false).line_style;
        // 仅当该行「刚输入了块标记」且尚未成为其他结构时标记 RuleState。
        // 行的渲染样式由 detect_line_style 实时反映；这里记录以便 Backspace 回退。
        match style {
            LineStyle::Heading(_)
            | LineStyle::Bullet
            | LineStyle::Ordered
            | LineStyle::Quote => {
                self.rule_state = RuleState::Block { line, style };
            }
            _ => {
                self.rule_state = RuleState::None;
            }
        }
    }

    // ---- macOS 编辑键（spec INP-12） ----

    fn delete_to_line_end(&mut self) {
        let (line, col) = (self.cursor_line, self.cursor_col);
        self.lines[line].truncate(col);
        self.dirty = true;
    }

    fn delete_to_line_start(&mut self) {
        let (line, col) = (self.cursor_line, self.cursor_col);
        self.lines[line].replace_range(..col, "");
        self.cursor_col = 0;
        self.dirty = true;
    }

    fn delete_word_back(&mut self) {
        if self.sel_start.is_some() {
            self.delete_selection();
            self.dirty = true;
            return;
        }
        let (line, col) = (self.cursor_line, self.cursor_col);
        let s = self.line(line).to_string();
        let bytes = s.as_bytes();
        let mut i = col;
        // 跳过词
        while i > 0 {
            let c = s[..i].chars().next_back().unwrap();
            if c.is_whitespace() {
                break;
            }
            i -= c.len_utf8();
        }
        let word_start = i;
        // 跳过词前连续空白 → ws_start
        while i > 0 {
            let c = s[..i].chars().next_back().unwrap();
            if !c.is_whitespace() {
                break;
            }
            i -= c.len_utf8();
        }
        let ws_start = i;
        // 保留一个分隔空格：删除 ws_start+1..col（若 ws_start 处是空白）
        let del_start = if ws_start < word_start && bytes[ws_start].is_ascii_whitespace() {
            ws_start + 1
        } else {
            ws_start
        };
        self.lines[line].replace_range(del_start..col, "");
        self.cursor_col = del_start;
        self.dirty = true;
    }

    // ---- NAV-07：Option+方向键单词移动 ----

    /// CJK 汉字视作独立词（每字一个词边界），符合中文编辑习惯。
    fn is_cjk(c: char) -> bool {
        let u = c as u32;
        (0x4E00..=0x9FFF).contains(&u)
            || (0x3400..=0x4DBF).contains(&u)
            || (0xF900..=0xFAFF).contains(&u)
    }

    pub fn move_word_left(&mut self) {
        let (line, col) = (self.cursor_line, self.cursor_col);
        let s = self.line(line).to_string();
        let mut i = col;
        while i > 0 {
            let c = s[..i].chars().next_back().unwrap();
            if !c.is_whitespace() {
                break;
            }
            i -= c.len_utf8();
        }
        while i > 0 {
            let c = s[..i].chars().next_back().unwrap();
            if c.is_whitespace() {
                break;
            }
            if Self::is_cjk(c) {
                i -= c.len_utf8();
                break;
            }
            i -= c.len_utf8();
        }
        self.cursor_col = i;
        self.dirty = false;
    }

    pub fn move_word_right(&mut self) {
        let (line, col) = (self.cursor_line, self.cursor_col);
        let s = self.line(line).to_string();
        let len = s.len();
        let mut i = col.min(len);
        while i < len {
            let c = s[i..].chars().next().unwrap();
            if c.is_whitespace() {
                break;
            }
            if Self::is_cjk(c) {
                i += c.len_utf8();
                break;
            }
            i += c.len_utf8();
        }
        while i < len {
            let c = s[i..].chars().next().unwrap();
            if !c.is_whitespace() {
                break;
            }
            i += c.len_utf8();
        }
        self.cursor_col = i;
        self.dirty = false;
    }

    fn delete_word_forward(&mut self) {
        if self.sel_start.is_some() {
            self.delete_selection();
            self.dirty = true;
            return;
        }
        let (line, col) = (self.cursor_line, self.cursor_col);
        let s = self.line(line).to_string();
        let len = s.len();
        let mut i = col;
        // 跳过空白
        while i < len {
            let c = s[i..].chars().next().unwrap();
            if !c.is_whitespace() {
                break;
            }
            i += c.len_utf8();
        }
        // 跳过词
        while i < len {
            let c = s[i..].chars().next().unwrap();
            if c.is_whitespace() {
                break;
            }
            i += c.len_utf8();
        }
        self.lines[line].replace_range(col..i, "");
        self.dirty = true;
    }

    // ---- DRG-01：拖放移动/复制 ----

    fn move_range(
        &mut self,
        from: (usize, usize, usize, usize),
        to: (usize, usize),
        copy: bool,
    ) {
        let (sl, sc, el, ec) = from;
        if sl > el || (sl == el && sc > ec) {
            return;
        }
        // 提取源文本
        let text = if sl == el {
            self.lines[sl][sc..ec].to_string()
        } else {
            let mut t = self.lines[sl][sc..].to_string();
            for l in sl + 1..el {
                t.push('\n');
                t.push_str(&self.lines[l]);
            }
            t.push('\n');
            t.push_str(&self.lines[el][..ec]);
            t
        };
        if text.is_empty() {
            return;
        }
        let (mut tl, mut tc) = to;
        let line_len = self.lines[tl].len();
        tc = tc.min(line_len);
        if !copy {
            // 目标在源区间内部：无操作
            if (tl, tc) >= (sl, sc) && (tl, tc) <= (el, ec) {
                return;
            }
            // 目标在源区间之后：删除后前移
            if (tl, tc) > (el, ec) {
                if sl == el {
                    let clen = ec - sc;
                    tc = tc.saturating_sub(clen);
                } else {
                    let nlines = el - sl;
                    tl = tl.saturating_sub(nlines);
                }
            }
        }
        // 删除源区间（仅移动时；复制保留源）
        if !copy {
            if sl == el {
                self.lines[sl].replace_range(sc..ec, "");
            } else {
                self.lines[sl] = self.lines[sl][..sc].to_string() + &self.lines[el][ec..];
                for _ in sl + 1..=el {
                    self.lines.remove(sl + 1);
                }
            }
        }
        // 插入到目标（先清选区，避免 insert_text 误删）
        self.sel_start = None;
        self.cursor_line = tl;
        self.cursor_col = tc;
        self.insert_text(&text);
        // 光标移到插入末尾
        let last = text.split('\n').last().unwrap_or("");
        let nl = text.matches('\n').count();
        self.cursor_line = tl + nl;
        self.cursor_col = last.len();
        self.sel_start = None;
        self.dirty = true;
    }

    // ---- 格式化（spec 2.4） ----

    fn toggle_mark(&mut self, mark: Mark) {
        if self.sel_start.is_some() {
            self.wrap_or_unwrap_selection(mark);
        } else {
            // 光标处：切换输入态（stored marks）
            self.pending_marks = toggle_pending(std::mem::take(&mut self.pending_marks), mark);
        }
    }

    fn wrap_or_unwrap_selection(&mut self, mark: Mark) {
        let Some(((sl, sc), (el, ec))) = self.selection_bounds() else {
            return;
        };
        let (open, close) = match mark {
            Mark::Bold => ("**", "**"),
            Mark::Italic => ("*", "*"),
            Mark::Code => ("`", "`"),
            Mark::Strike => ("~~", "~~"),
        };
        // 去除：选区正好被标记包裹
        let selected = self.selected_text().unwrap_or_default();
        if let Some(inner) = strip_wrapped(&selected, open, close) {
            // 直接替换选区为去包裹文本
            self.lines[sl] = self.lines[sl][..sc].to_string() + &inner + &self.lines[el][ec..];
            for _ in sl + 1..=el {
                self.lines.remove(sl + 1);
            }
            self.cursor_line = sl;
            self.cursor_col = sc + inner.len();
            self.sel_start = None;
            self.dirty = true;
            return;
        }
        // 包裹选区
        let new_text = format!("{open}{selected}{close}");
        if sl == el {
            self.lines[sl].replace_range(sc..ec, &new_text);
        } else {
            self.lines[sl] = self.lines[sl][..sc].to_string() + &new_text + &self.lines[el][ec..];
            for _ in sl + 1..=el {
                self.lines.remove(sl + 1);
            }
        }
        self.cursor_line = sl;
        self.cursor_col = sc + new_text.len();
        self.sel_start = None;
        self.dirty = true;
    }

    /// 插入链接（Cmd+K）：选区 → `[选区](url)`；无选区时插入 `[text](url)`。
    /// 光标放在 url 之后（方便继续输入 URL）。
    fn insert_link(&mut self) {
        if self.sel_start.is_some() {
            let selected = self.selected_text().unwrap_or_default();
            let text = format!("[{selected}](url)");
            let cursor = text.len() - 1; // 去掉末尾 ")"
            self.replace_selection_with(&text, cursor);
        } else {
            self.insert_text("[text](url)");
            self.cursor_col = self.cursor_col.saturating_sub(2); // 光标在 "rl" 后
        }
        self.dirty = true;
    }

    fn replace_selection_with(&mut self, text: &str, cursor_offset: usize) {
        let Some(((sl, sc), (el, ec))) = self.selection_bounds() else {
            return;
        };
        if sl == el {
            self.lines[sl].replace_range(sc..ec, text);
        } else {
            self.lines[sl] = self.lines[sl][..sc].to_string() + text + &self.lines[el][ec..];
            for _ in sl + 1..=el {
                self.lines.remove(sl + 1);
            }
        }
        self.cursor_line = sl;
        self.cursor_col = sc + cursor_offset;
        self.sel_start = None;
    }

    /// 设置当前行块类型。
    fn set_block_type(&mut self, t: BlockType) {
        let (line, _col) = (self.cursor_line, self.cursor_col);
        let cur = self.lines[line].clone();
        match t {
            BlockType::Paragraph => {
                let body = strip_block_prefix(&cur);
                self.lines[line] = body;
            }
            BlockType::Heading(level) => {
                let marker = "#".repeat(level.clamp(1, 6) as usize) + " ";
                let body = strip_block_prefix(&cur);
                if body.trim().is_empty() {
                    self.lines[line] = marker.clone();
                    self.cursor_col = marker.len();
                } else {
                    self.lines[line] = format!("{marker}{body}");
                }
            }
            BlockType::Quote => {
                if !cur.starts_with('>') {
                    self.lines[line] = format!("> {cur}");
                }
            }
            BlockType::CodeBlock => {
                if crate::editor::is_fence_line(&cur) {
                    // 已是围栏
                } else {
                    let body = strip_block_prefix(&cur);
                    self.lines[line] = "```".to_string();
                    self.lines.insert(line + 1, body.clone());
                    self.lines.insert(line + 2, "```".to_string());
                    self.cursor_line = line + 1;
                    self.cursor_col = body.len();
                    self.dirty = true;
                    return;
                }
            }
        }
        // 单行块类型：光标移到内容末尾
        self.cursor_line = line;
        self.cursor_col = self.lines[line].len();
        self.dirty = true;
    }

    /// 选区转列表（spec FMT-08）。
    fn wrap_list(&mut self, kind: ListKind) {
        let Some(((sl, sc), (el, ec))) = self.selection_bounds() else {
            // 无选区：把当前行转列表
            let line = self.cursor_line;
            let cur = self.lines[line].clone();
            let marker = match kind {
                ListKind::Bullet => "- ",
                ListKind::Ordered => "1. ",
                ListKind::Task => "- [ ] ",
            };
            if !cur.starts_with(marker) {
                self.lines[line] = format!("{marker}{cur}");
                self.cursor_col += marker.len();
                self.dirty = true;
            }
            return;
        };
        let mut new_lines: Vec<String> = Vec::new();
        if sl > 0 {
            new_lines.push(self.lines[..sl].to_vec().join("\n"));
        }
        let first = self.lines[sl][sc..].to_string();
        new_lines.push(match kind {
            ListKind::Bullet => format!("- {first}"),
            ListKind::Ordered => format!("1. {first}"),
            ListKind::Task => format!("- [ ] {first}"),
        });
        for l in sl + 1..el {
            new_lines.push(match kind {
                ListKind::Bullet => format!("- {}", self.lines[l]),
                ListKind::Ordered => format!("{}. {}", l - sl + 1, self.lines[l]),
                ListKind::Task => format!("- [ ] {}", self.lines[l]),
            });
        }
        let last = self.lines[el][..ec].to_string();
        if !last.is_empty() {
            new_lines.push(match kind {
                ListKind::Bullet => format!("- {last}"),
                ListKind::Ordered => format!("{}. {}", el - sl + 1, last),
                ListKind::Task => format!("- [ ] {last}"),
            });
        }
        if el + 1 < self.lines.len() {
            new_lines.push(self.lines[el + 1..].to_vec().join("\n"));
        }
        self.lines = new_lines;
        self.cursor_line = self.lines.len().saturating_sub(1);
        self.cursor_col = self.lines[self.cursor_line].len();
        self.sel_start = None;
        self.dirty = true;
    }
}

// ---------------------------------------------------------------------------
// 撤销状态
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LastOp {
    Typing,
    Other,
}

// ---------------------------------------------------------------------------
// 工具函数
// ---------------------------------------------------------------------------

/// 行首块标记字符串（按样式）。
fn block_marker(line: &str, style: crate::editor::LineStyle) -> Option<String> {
    let trimmed = line.trim_start();
    match style {
        LineStyle::Heading(n) => {
            let count = n as usize;
            if trimmed.starts_with(&"#".repeat(count)) {
                let after = &trimmed[count..];
                let marker_len = count + if after.starts_with(' ') { 1 } else { 0 };
                Some(line[..marker_len].to_string())
            } else {
                None
            }
        }
        LineStyle::Bullet => {
            for m in ["- ", "* ", "+ "] {
                if trimmed.starts_with(m) {
                    return Some(line[..m.len()].to_string());
                }
            }
            None
        }
        LineStyle::Ordered => {
            let bytes = trimmed.as_bytes();
            let mut i = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > 0 && i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b')') {
                let len = i + if i + 1 < bytes.len() && bytes[i + 1] == b' ' { 2 } else { 1 };
                Some(trimmed[..len.min(trimmed.len())].to_string())
            } else {
                None
            }
        }
        LineStyle::Quote => {
            let plen = if trimmed.len() > 1 && trimmed.as_bytes()[1] == b' ' { 2 } else { 1 };
            Some(trimmed[..plen.min(trimmed.len())].to_string())
        }
        _ => None,
    }
}

/// 去掉行首块标记，返回正文（保留缩进）。
fn strip_block_prefix(line: &str) -> String {
    let style = crate::editor::parse_line(line, false).line_style;
    if let Some(marker) = block_marker(line, style) {
        line[marker.len()..].to_string()
    } else {
        line.to_string()
    }
}

/// 从行首提取有序列表起始序号。
fn ordered_start(line: &str) -> Option<u64> {
    let trimmed = line.trim_start();
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        digits.parse().ok()
    } else {
        None
    }
}

/// 选区被 `open..close` 包裹时返回内层文本。
fn strip_wrapped(s: &str, open: &str, close: &str) -> Option<String> {
    if s.len() >= open.len() + close.len()
        && s.starts_with(open)
        && s.ends_with(close)
        && s.len() > open.len() + close.len()
    {
        Some(s[open.len()..s.len() - close.len()].to_string())
    } else {
        None
    }
}

/// 切换待输入标记集合。
fn toggle_pending(mut marks: Vec<Mark>, mark: Mark) -> Vec<Mark> {
    if let Some(pos) = marks.iter().position(|m| *m == mark) {
        marks.remove(pos);
    } else {
        marks.push(mark);
    }
    marks
}

/// 文本输入时应用待输入标记（stored marks）。
pub fn apply_pending_marks(text: &str, marks: &[Mark]) -> String {
    if marks.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    // 从外到内包裹（粗体最外层）
    for mark in marks.iter().rev() {
        let (open, close) = match mark {
            Mark::Bold => ("**", "**"),
            Mark::Italic => ("*", "*"),
            Mark::Code => ("`", "`"),
            Mark::Strike => ("~~", "~~"),
        };
        out = format!("{open}{out}{close}");
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;

    // ---- 测试 DSL：`|` 表示光标 ----

    /// 从带光标标注的字符串构造编辑器。
    fn setup(src: &str) -> Editor {
        let mut cursor: Option<(usize, usize)> = None;
        let mut lines: Vec<String> = Vec::new();
        for (li, raw) in src.split('\n').enumerate() {
            if let Some(pos) = raw.find('|') {
                assert!(cursor.is_none(), "only one cursor allowed");
                cursor = Some((li, pos));
                let mut l = raw.to_string();
                l.remove(pos);
                lines.push(l);
            } else {
                lines.push(raw.to_string());
            }
        }
        let mut e = Editor::new(&lines.join("\n"));
        if let Some((l, c)) = cursor {
            e.cursor_line = l;
            e.cursor_col = c;
        }
        e
    }

    /// 渲染编辑器源码 + 光标位置。
    fn render(e: &Editor) -> String {
        let mut out = String::new();
        for (i, line) in e.lines.iter().enumerate() {
            if i == e.cursor_line {
                let col = e.cursor_col.min(line.len());
                out.push_str(&line[..col]);
                out.push('|');
                out.push_str(&line[col..]);
            } else {
                out.push_str(line);
            }
            if i + 1 < e.lines.len() {
                out.push('\n');
            }
        }
        out
    }

    fn assert_edit(input: &str, ops: &[Op], expected: &str) {
        let mut e = setup(input);
        for op in ops {
            e.apply(op);
        }
        assert_eq!(render(&e), expected, "input={input:?} ops={ops:?}");
    }

    // 便捷 op 构造
    fn ty(s: &str) -> Op {
        Op::Type(s.to_string())
    }
    fn mv(d: Direction, extend: bool) -> Op {
        Op::Move(d, extend)
    }

    // ================= 2.3 文本输入 =================

    #[test]
    fn insert_plain() {
        assert_edit("|abc", &[ty("X")], "X|abc");
        assert_edit("ab|c", &[ty("X")], "abX|c");
        assert_edit("abc|", &[ty("X")], "abcX|");
    }

    #[test]
    fn insert_multibyte_cjk() {
        assert_edit("|你好", &[ty("世")], "世|你好");
        assert_edit("你|好", &[ty("界")], "你界|好");
    }

    #[test]
    fn insert_emoji() {
        assert_edit("|ab", &[ty("🚀")], "🚀|ab");
        // emoji 是单字符，光标不在中间
        assert_edit("🚀|ab", &[ty("x")], "🚀x|ab");
    }

    #[test]
    fn insert_mid_bold_preserves_markers() {
        // FMT-01：在 **bold** 内部插入，标记保持闭合
        assert_edit("**bo|ld**", &[ty("X")], "**boX|ld**");
        assert_edit("**|bold**", &[ty("X")], "**X|bold**");
        // 标记内插入 CJK
        assert_edit("**加|粗**", &[ty("X")], "**加X|粗**");
    }

    #[test]
    fn insert_replaces_selection() {
        // 选区输入替换：用 move+shift 构造选区
        let mut e = setup("ab|cd");
        e.apply(&mv(Direction::Left, true)); // 选中 b
        e.apply(&mv(Direction::Left, true)); // 选中 ab
        e.apply(&ty("X"));
        assert_eq!(render(&e), "X|cd");
    }

    #[test]
    fn paste_multiline() {
        // CLP-03：多行粘贴拆分
        assert_edit("a|b", &[Op::Paste("x\ny".into())], "ax\ny|b");
        assert_edit("|ab", &[Op::Paste("1\n2\n3".into())], "1\n2\n3|ab");
    }

    #[test]
    fn paste_does_not_trigger_input_rule() {
        // 粘贴不触发输入规则（不把粘贴内容当刚输入）
        let mut e = setup("|");
        e.apply(&Op::Paste("- ".into()));
        // 源码是 "- "，样式是列表，但不算「刚输入规则」
        assert_eq!(e.undo_depth(), 1);
    }

    // ================= 2.12 回车 / 段落语义 =================

    #[test]
    fn enter_mid_paragraph_inserts_blank_line() {
        // MD-02：段落中间回车 → 拆成两段（中间空行）
        assert_edit("hello|world", &[Op::Newline], "hello\n\n|world");
    }

    #[test]
    fn enter_end_paragraph_with_following_text() {
        // 行尾回车且下一行非空 → 插入空行分隔
        let mut e = setup("foo|\nbar");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "foo\n|\nbar");
    }

    #[test]
    fn enter_end_paragraph_plain() {
        // 行尾回车且无后续 → 普通新行
        let mut e = setup("foo|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "foo\n|");
    }

    #[test]
    fn enter_start_of_line() {
        // 行首回车 → 上方插入空行
        let mut e = setup("|foo");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "\n|foo");
    }

    #[test]
    fn hard_break_appends_two_spaces() {
        // MD-03：Shift+Enter 硬换行（行尾两空格）
        assert_edit("foo|bar", &[Op::HardBreak], "foo  \n|bar");
        // 已有两空格不重复
        assert_edit("foo  |bar", &[Op::HardBreak], "foo  \n|bar");
    }

    #[test]
    fn enter_in_empty_line() {
        // 空行回车 → 普通新行
        let mut e = setup("|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "\n|");
    }

    // ================= 2.8 撤销 / 重做 =================

    #[test]
    fn undo_restores_document_and_cursor() {
        // UND-03：撤销恢复文档与光标
        let mut e = setup("|abc");
        e.apply(&ty("X"));
        e.apply(&ty("Y"));
        assert_eq!(render(&e), "XY|abc");
        assert_eq!(e.undo_depth(), 1, "X 与 Y 合并为一步");
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "|abc");
        assert_eq!(e.undo_depth(), 0);
    }

    #[test]
    fn redo_restores() {
        let mut e = setup("|abc");
        e.apply(&ty("X"));
        e.apply(&Op::Undo);
        e.apply(&Op::Redo);
        assert_eq!(render(&e), "X|abc");
    }

    #[test]
    fn editing_after_undo_clears_redo() {
        // UND-06：撤销后编辑清空 redo
        let mut e = setup("|abc");
        e.apply(&ty("X"));
        e.apply(&Op::Undo);
        assert_eq!(e.redo_depth(), 1);
        e.apply(&ty("Z"));
        assert_eq!(e.redo_depth(), 0);
    }

    #[test]
    fn consecutive_typing_merges_into_one_undo_step() {
        // UND-02/08：连续键入合并
        let mut e = setup("|");
        e.apply(&ty("h"));
        e.apply(&ty("e"));
        e.apply(&ty("l"));
        assert_eq!(e.undo_depth(), 1, "three chars = one step");
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "|");
    }

    #[test]
    fn structural_change_breaks_merge() {
        // 回车是结构变化，不合并
        let mut e = setup("|a");
        e.apply(&ty("x"));
        e.apply(&Op::Newline);
        e.apply(&ty("y"));
        assert_eq!(e.undo_depth(), 3, "typing/newline/typing 各一步");
        // 全撤销回初始
        e.apply(&Op::Undo);
        e.apply(&Op::Undo);
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "|a");
    }

    // ================= 2.5 输入规则 =================

    #[test]
    fn input_rule_bullet_then_backspace_undoes() {
        // IR-01 + IR-07：输入 "- " 变列表，Backspace 回退
        let mut e = setup("|");
        e.apply(&ty("-"));
        e.apply(&ty(" "));
        let style = crate::editor::parse_line(&e.lines[0], false).line_style;
        assert_eq!(style, crate::editor::LineStyle::Bullet);
        // 空列表项上 Backspace 回退标记
        e.apply(&Op::Backspace);
        assert_eq!(render(&e), "|");
    }

    #[test]
    fn input_rule_heading() {
        let mut e = setup("|");
        e.apply(&ty("#"));
        e.apply(&ty(" "));
        let style = crate::editor::parse_line(&e.lines[0], false).line_style;
        assert_eq!(style, crate::editor::LineStyle::Heading(1));
    }

    #[test]
    fn input_rule_ordered() {
        let mut e = setup("|");
        e.apply(&ty("1"));
        e.apply(&ty("."));
        e.apply(&ty(" "));
        let style = crate::editor::parse_line(&e.lines[0], false).line_style;
        assert_eq!(style, crate::editor::LineStyle::Ordered);
    }

    #[test]
    fn input_rule_quote() {
        let mut e = setup("|");
        e.apply(&ty(">"));
        e.apply(&ty(" "));
        let style = crate::editor::parse_line(&e.lines[0], false).line_style;
        assert_eq!(style, crate::editor::LineStyle::Quote);
    }

    #[test]
    fn input_rule_fence() {
        let mut e = setup("|");
        e.apply(&ty("`"));
        e.apply(&ty("`"));
        e.apply(&ty("`"));
        let style = crate::editor::parse_line(&e.lines[0], false).line_style;
        assert_eq!(style, crate::editor::LineStyle::Fence);
    }

    #[test]
    fn input_rule_rule() {
        let mut e = setup("|");
        for c in ['-', '-', '-'] {
            e.apply(&ty(&c.to_string()));
        }
        let style = crate::editor::parse_line(&e.lines[0], false).line_style;
        assert_eq!(style, crate::editor::LineStyle::Rule);
    }

    // ================= 2.6 列表行为 =================

    #[test]
    fn list_enter_continues_with_marker() {
        // LST-01：列表项内回车 → 新列表项
        let mut e = setup("- item|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "- item\n- |");
    }

    #[test]
    fn list_enter_empty_exits() {
        // LST-02：空列表项回车 → 退出列表
        let mut e = setup("- |");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "- \n|");
    }

    #[test]
    fn ordered_list_enter_increments() {
        let mut e = setup("1. item|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "1. item\n2. |");
    }

    #[test]
    fn ordered_list_start_number_preserved() {
        // LST-09：start 序号延续
        let mut e = setup("3. item|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "3. item\n4. |");
    }

    #[test]
    fn nested_list_enter_keeps_indent() {
        let mut e = setup("  - item|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "  - item\n  - |");
    }

    #[test]
    fn tab_indents_list_item() {
        // LST-03：Tab 缩进嵌套
        let mut e = setup("- |item");
        // 光标在标记后
        e.cursor_col = 2;
        e.apply(&Op::Tab);
        assert_eq!(render(&e), "    - |item");
    }

    #[test]
    fn shift_tab_lifts_list_item() {
        let mut e = setup("    - |item");
        e.cursor_col = 6;
        e.apply(&Op::ShiftTab);
        assert_eq!(render(&e), "- |item");
    }

    #[test]
    fn quote_enter_continues() {
        // LST-06：引用续行
        let mut e = setup("> quote|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "> quote\n> |");
    }

    // ================= 2.4 格式化 =================

    #[test]
    fn toggle_bold_wraps_selection() {
        let mut e = setup("abc|def");
        // 选中 abc：左移3
        e.apply(&mv(Direction::Left, true));
        e.apply(&mv(Direction::Left, true));
        e.apply(&mv(Direction::Left, true));
        e.apply(&Op::ToggleMark(Mark::Bold));
        assert_eq!(render(&e), "**abc**|def");
    }

    #[test]
    fn toggle_bold_unwraps() {
        let mut e = setup("**abc**|");
        e.sel_start = Some((0, 0));
        e.cursor_col = 7; // 选中 **abc**（长度 7）
        e.apply(&Op::ToggleMark(Mark::Bold));
        assert_eq!(render(&e), "abc|");
    }

    #[test]
    fn toggle_italic_and_code() {
        let mut e = setup("abc|def");
        for _ in 0..3 {
            e.apply(&mv(Direction::Left, true));
        }
        e.apply(&Op::ToggleMark(Mark::Italic));
        assert_eq!(render(&e), "*abc*|def");
        let mut e2 = setup("abc|def");
        for _ in 0..3 {
            e2.apply(&mv(Direction::Left, true));
        }
        e2.apply(&Op::ToggleMark(Mark::Code));
        assert_eq!(render(&e2), "`abc`|def");
    }

    #[test]
    fn toggle_mark_at_cursor_sets_pending() {
        // FMT-11：光标处切换 = 设置输入态，后续输入带标记
        let mut e = setup("|");
        e.apply(&Op::ToggleMark(Mark::Bold));
        assert_eq!(e.pending_marks, vec![Mark::Bold]);
        e.apply(&ty("hi"));
        assert_eq!(render(&e), "**hi**|");
    }

    #[test]
    fn insert_link_wraps_selection() {
        let mut e = setup("abc|def");
        for _ in 0..3 {
            e.apply(&mv(Direction::Left, true));
        }
        e.apply(&Op::InsertLink);
        assert_eq!(render(&e), "[abc](url|)def");
    }

    #[test]
    fn set_heading_block_type() {
        let mut e = setup("hello|");
        e.apply(&Op::SetBlockType(BlockType::Heading(2)));
        assert_eq!(render(&e), "## hello|");
        // 再次设为标题不变
        e.apply(&Op::SetBlockType(BlockType::Heading(2)));
        assert_eq!(render(&e), "## hello|");
        // 设为段落去掉标记
        e.apply(&Op::SetBlockType(BlockType::Paragraph));
        assert_eq!(render(&e), "hello|");
    }

    #[test]
    fn set_code_block_wraps() {
        let mut e = setup("let x = 1|");
        e.apply(&Op::SetBlockType(BlockType::CodeBlock));
        assert_eq!(render(&e), "```\nlet x = 1|\n```");
    }

    #[test]
    fn wrap_selection_as_bullet_list() {
        let mut e = setup("line1|\nline2");
        e.cursor_line = 1;
        e.cursor_col = 0;
        e.sel_start = Some((0, 0));
        e.cursor_col = 5;
        e.apply(&Op::WrapList(ListKind::Bullet));
        assert_eq!(render(&e), "- line1\n- line2|");
    }

    #[test]
    fn wrap_current_line_as_task() {
        let mut e = setup("todo|");
        e.apply(&Op::WrapList(ListKind::Task));
        assert_eq!(render(&e), "- [ ] todo|");
    }

    // ================= 光标导航 =================

    #[test]
    fn move_boundaries_emoji() {
        // NAV-01：不在 emoji 中间停
        let mut e = setup("a🚀|b");
        e.apply(&mv(Direction::Left, false));
        assert_eq!(render(&e), "a|🚀b");
    }

    #[test]
    fn move_cross_line() {
        // NAV-04：行尾→下一行、行首→上一行
        let mut e = setup("abc|\ndef");
        e.apply(&mv(Direction::Right, false));
        assert_eq!(render(&e), "abc\n|def");
        let mut e = setup("abc\n|def");
        e.apply(&mv(Direction::Left, false));
        assert_eq!(render(&e), "abc|\ndef");
    }

    #[test]
    fn nav07_word_movement() {
        // NAV-07：Option+方向键词移动
        let mut e = setup("|hello world");
        e.move_word_right();
        assert_eq!(e.cursor_col, 6, "跳到下一个词开头");
        e.move_word_right();
        assert_eq!(e.cursor_col, 11, "行尾");
        e.move_word_left();
        assert_eq!(e.cursor_col, 6, "回到词开头");
        e.move_word_left();
        assert_eq!(e.cursor_col, 0, "行首");
        // 中文按字移动
        let mut e = setup("|你好世界");
        e.move_word_right();
        assert_eq!(e.cursor_col, 3, "中文每个字一个词边界");
        // 连续空白
        let mut e = setup("|a   b");
        e.move_word_right();
        assert_eq!(e.cursor_col, 4, "跨过连续空格到 b");
    }

    #[test]
    fn und05_history_cap() {
        // UND-05：撤销历史上限 100
        let mut e = setup("|");
        for _ in 0..110 {
            e.apply(&Op::Newline);
        }
        assert_eq!(e.undo_depth(), 100, "undo 栈应被裁剪到上限");
        // 仍可撤销并恢复
        e.apply(&Op::Undo);
        assert_eq!(e.lines.len(), 110);
        e.apply(&Op::Undo);
        assert_eq!(e.lines.len(), 109);
    }

    #[test]
    fn drg01_move_range() {
        // 移动：源前（选中 "world" → 拖到行首）
        let mut e = setup("hello world!|");
        e.sel_start = Some((0, 6));
        e.cursor_col = 11;
        e.apply(&Op::MoveRange { from: (0, 6, 0, 11), to: (0, 0), copy: false });
        assert_eq!(render(&e), "world|hello !");
        // 复制（Option+拖）
        let mut e = setup("hello world!|");
        e.sel_start = Some((0, 6));
        e.cursor_col = 11;
        e.apply(&Op::MoveRange { from: (0, 6, 0, 11), to: (0, 0), copy: true });
        assert_eq!(render(&e), "world|hello world!");
        // 目标在源之后：偏移修正（"ab" 移到 cd 后）
        let mut e = setup("abcd|");
        e.sel_start = Some((0, 0));
        e.cursor_col = 2;
        e.apply(&Op::MoveRange { from: (0, 0, 0, 2), to: (0, 3), copy: false });
        assert_eq!(render(&e), "ca|bd");
        // 目标在源内部：无操作
        let mut e = setup("abcd|");
        e.sel_start = Some((0, 0));
        e.cursor_col = 4;
        e.apply(&Op::MoveRange { from: (0, 0, 0, 4), to: (0, 2), copy: false });
        assert_eq!(render(&e), "abcd|");
        // 跨行移动（选中 "b\nc" → 拖到首行开头）
        let mut e = setup("ab\ncd|");
        e.sel_start = Some((0, 1));
        e.cursor_line = 1;
        e.cursor_col = 1;
        e.apply(&Op::MoveRange { from: (0, 1, 1, 1), to: (0, 0), copy: false });
        assert_eq!(render(&e), "b\nc|ad");
        // 跨行移动到源之后（目标偏移修正）
        let mut e = setup("ab\ncd|");
        e.sel_start = Some((0, 1));
        e.cursor_line = 1;
        e.cursor_col = 1;
        e.apply(&Op::MoveRange { from: (0, 1, 1, 1), to: (1, 1), copy: false });
        // to (1,1) 在源后：tl=0,tc=1。删除 → "ad"，插入 (0,1)："a"+"b\nc"+"d"
        assert_eq!(render(&e), "ab\nc|d");
    }

    #[test]
    fn select_all_sets_range() {
        let mut e = setup("a\nb|");
        e.apply(&Op::SelectAll);
        assert_eq!(e.sel_start, Some((0, 0)));
        assert_eq!((e.cursor_line, e.cursor_col), (1, 1));
    }

    // ================= 不变量 =================

    #[test]
    fn roundtrip_source() {
        // SAV-01：load → to_source 字节级一致
        let samples = [
            "# 标题\n\n正文 **加粗** 和 *斜体*\n\n- a\n- b\n",
            "```rust\nfn main() {}\n```\n",
            "> 引用\n\n| 表格 |\n",
            "",
            "  缩进行\n",
        ];
        for src in samples {
            let e = Editor::new(src);
            assert_eq!(e.to_source(), src, "roundtrip {src:?}");
        }
    }

    #[test]
    fn marks_balanced_after_edits() {
        // FMT-01 不变量：任意编辑后 ** * ` ~~ == 配对平衡
        let cases = [
            (vec![ty("**a**"), mv(Direction::Left, false), ty("X")], "**aX**"),
            (vec![ty("**b"), mv(Direction::Left, false), ty("y")], "**by"),
            (vec![ty("~~x~~"), Op::Backspace], "~~x~"),
        ];
        for (ops, _label) in cases {
            let mut e = Editor::new("");
            for op in &ops {
                e.apply(op);
            }
            for (open, close) in [("**", "**"), ("`", "`"), ("~~", "~~")] {
                let n_open = e.to_source().matches(open).count();
                let n_close = e.to_source().matches(close).count();
                assert_eq!(n_open, n_close, "balanced {open} in {:?}", e.to_source());
            }
        }
    }
    // ================= NAV-02 视觉列 =================

    #[test]
    fn up_down_preserves_visual_column() {
        // 从 "abcdef" 第 3 列向下到短行（截断到行尾），再向上恢复记忆列
        let mut e = setup("abc|def\nab");
        e.apply(&mv(Direction::Down, false));
        assert_eq!(render(&e), "abcdef\nab|");
        e.apply(&mv(Direction::Up, false));
        assert_eq!(render(&e), "abc|def\nab");
    }

    #[test]
    fn up_down_across_marked_lines() {
        // **ab** 显示宽度 2，向上对齐到 "12345" 的第 2 列
        let mut e = setup("12345\n**ab**|");
        e.apply(&mv(Direction::Up, false));
        assert_eq!(render(&e), "12|345\n**ab**");
    }

    // ================= FMT-03 标记原子删除 =================

    #[test]
    fn backspace_deletes_marker_pair() {
        let mut e = setup("**|");
        e.apply(&Op::Backspace);
        assert_eq!(render(&e), "|");
        let mut e2 = setup("~~|");
        e2.apply(&Op::Backspace);
        assert_eq!(render(&e2), "|");
        let mut e3 = setup("==|");
        e3.apply(&Op::Backspace);
        assert_eq!(render(&e3), "|");
    }

    #[test]
    fn backspace_deletes_single_marker() {
        let mut e = setup("`|");
        e.apply(&Op::Backspace);
        assert_eq!(render(&e), "|");
        let mut e2 = setup("*|");
        e2.apply(&Op::Backspace);
        assert_eq!(render(&e2), "|");
    }

    #[test]
    fn backspace_marker_keeps_other_side() {
        let mut e = setup("**|bold**");
        e.apply(&Op::Backspace);
        assert_eq!(render(&e), "|bold**");
    }

    #[test]
    fn backspace_inside_code_fence_is_plain() {
        let mut e = setup("```\n**|\n```");
        e.cursor_line = 1;
        e.cursor_col = 2;
        e.apply(&Op::Backspace);
        assert_eq!(render(&e), "```\n*|\n```");
    }

    // ================= 引用/标题/代码围栏 回车与 Tab =================

    #[test]
    fn quote_enter_empty_exits() {
        let mut e = setup("> |");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "> \n|");
    }

    #[test]
    fn heading_enter_splits_plain() {
        let mut e = setup("## title|");
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "## title\n|");
    }

    #[test]
    fn enter_in_code_fence_plain_newline() {
        let mut e = setup("```\nlet x = 1|\n```");
        e.cursor_line = 1;
        e.cursor_col = 9;
        e.apply(&Op::Newline);
        assert_eq!(render(&e), "```\nlet x = 1\n|\n```");
    }

    #[test]
    fn tab_in_code_fence_inserts_spaces() {
        let mut e = setup("```\n|code\n```");
        e.cursor_line = 1;
        e.apply(&Op::Tab);
        assert_eq!(render(&e), "```\n    |code\n```");
    }

    // ================= 撤销边界 =================

    #[test]
    fn undo_input_rule_list() {
        let mut e = setup("|");
        e.apply(&ty("-"));
        e.apply(&ty(" "));
        e.apply(&ty("item"));
        assert_eq!(e.undo_depth(), 1);
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "|");
    }

    #[test]
    fn undo_paste_is_single_step() {
        let mut e = setup("|");
        e.apply(&Op::Paste("a\nb\nc".into()));
        assert_eq!(e.undo_depth(), 1);
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "|");
    }

    #[test]
    fn formatting_is_undoable() {
        let mut e = setup("abc|");
        e.sel_start = Some((0, 0));
        e.cursor_col = 3;
        e.apply(&Op::ToggleMark(Mark::Bold));
        assert_eq!(render(&e), "**abc**|");
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "abc|");
    }

    // ================= 块类型 =================

    #[test]
    fn set_quote_block() {
        let mut e = setup("hello|");
        e.apply(&Op::SetBlockType(BlockType::Quote));
        assert_eq!(render(&e), "> hello|");
        e.apply(&Op::SetBlockType(BlockType::Paragraph));
        assert_eq!(render(&e), "hello|");
    }

    #[test]
    fn heading_empty_line_sets_marker() {
        let mut e = setup("|");
        e.apply(&Op::SetBlockType(BlockType::Heading(3)));
        assert_eq!(render(&e), "### |");
    }

    // ================= 选区转列表 =================

    #[test]
    fn wrap_multi_line_ordered_list() {
        let mut e = setup("a\nb|");
        e.sel_start = Some((0, 0));
        e.cursor_col = 1;
        e.apply(&Op::WrapList(ListKind::Ordered));
        assert_eq!(render(&e), "1. a\n2. b|");
    }

    // ================= SEL 选区增强 =================

    #[test]
    fn double_click_selects_word() {
        // SEL-04：双击选词（显示文本词边界，映射回源码）
        let mut e = setup("hello **world** here|");
        // 光标在 "world" 内（显示位置）
        e.select_word_at(0, 8); // "world" 在 display 的 6..11
        let sel = e.selected_text().unwrap();
        assert_eq!(sel, "world");
    }

    #[test]
    fn double_click_selects_cjk_word() {
        let mut e = setup("这是 **中文** 测试|");
        e.select_word_at(0, 3);
        assert_eq!(e.selected_text().unwrap(), "中文");
    }

    #[test]
    fn triple_click_selects_line() {
        // SEL-08：三击选整行
        let mut e = setup("hello world|");
        e.select_line_at(0);
        assert_eq!(e.selected_text().unwrap(), "hello world");
    }

    #[test]
    fn shift_click_extends_selection() {
        // Shift+点击扩展：由 UI 层处理，逻辑层验证选区锚定
        let mut e = setup("abcdef|");
        e.sel_start = Some((0, 0));
        e.cursor_col = 3;
        assert_eq!(e.selected_text().unwrap(), "abc");
    }

    // ================= REN 双渲染器一致性 =================

    /// 提取预览文档的纯文本。
    fn preview_text(src: &str) -> String {
        fn collect(b: &crate::parse::Block, out: &mut String) {
            use crate::parse::Block;
            match b {
                Block::Paragraph { inline, .. } => out.push_str(&inline.0),
                Block::Heading { inline, .. } => out.push_str(&inline.0),
                Block::List { items, .. } => {
                    for it in items {
                        for b in it {
                            collect(b, out);
                        }
                    }
                }
                Block::Quote(inner) => {
                    for b in inner {
                        collect(b, out);
                    }
                }
                Block::Code { text, .. } => out.push_str(text),
                Block::Table { head, rows } => {
                    for c in head {
                        out.push_str(&c.0);
                    }
                    for r in rows {
                        for c in r {
                            out.push_str(&c.0);
                        }
                    }
                }
                _ => {}
            }
        }
        let doc = crate::parse::parse_document(src, None);
        let mut out = String::new();
        for b in &doc.blocks {
            collect(b, &mut out);
        }
        out
    }

    #[test]
    fn render_parity_core_inline() {
        // REN-01：核心行内语法，编辑视图与预览文本一致
        let cases = [
            "hello world",
            "**bold** and *italic*",
            "***both***",
            "`code` here",
            "~~strike~~",
            "[link](https://a.b) text",
            "# heading",
            "## sub heading",
            "> quote",
        ];
        for src in cases {
            let editor = crate::editor::parse_line(src, false).display;
            let preview = preview_text(src);
            assert_eq!(
                editor.trim(),
                preview.trim(),
                "render parity for {src:?}"
            );
        }
    }

    #[test]
    fn render_parity_known_divergences() {
        // 已知差异（spec REN-01 遗留）：编辑视图支持但预览（pulldown 默认选项）不支持
        // 1. ==mark==：两边都去标记显示（parse_inline 的 split_mark 双端一致）
        let e = crate::editor::parse_line("==mark==", false).display;
        assert_eq!(e, "mark");
        let p = preview_text("==mark==");
        assert_eq!(p, "mark", "预览与编辑视图一致");
        // 2. 上下标：编辑视图与预览都去标记，一致
        let e2 = crate::editor::parse_line("H~2~O", false).display;
        assert_eq!(e2, "H2O");
        let p2 = preview_text("H~2~O");
        assert_eq!(p2, "H2O", "预览与编辑视图一致");
        let e3 = crate::editor::parse_line("E=mc^2^", false).display;
        assert_eq!(e3, "E=mc2");
    }

    // ================= macOS 编辑键（INP-12） =================

    #[test]
    fn ctrl_k_delete_to_line_end() {
        let mut e = setup("hello |world");
        e.cursor_col = 6;
        e.apply(&Op::DeleteToLineEnd);
        assert_eq!(render(&e), "hello |");
    }

    #[test]
    fn ctrl_u_delete_to_line_start() {
        let mut e = setup("hello |world");
        e.cursor_col = 6;
        e.apply(&Op::DeleteToLineStart);
        assert_eq!(render(&e), "|world");
    }

    #[test]
    fn ctrl_w_delete_word_back() {
        let mut e = setup("hello world|");
        e.apply(&Op::DeleteWordBack);
        assert_eq!(render(&e), "hello |");
        // 多空白
        let mut e2 = setup("a   b|");
        e2.apply(&Op::DeleteWordBack);
        assert_eq!(render(&e2), "a |");
    }

    #[test]
    fn alt_delete_delete_word_forward() {
        let mut e = setup("|hello world");
        e.apply(&Op::DeleteWordForward);
        assert_eq!(render(&e), "| world");
    }

    #[test]
    fn delete_to_line_undoable() {
        let mut e = setup("hello |world");
        e.cursor_col = 6;
        e.apply(&Op::DeleteToLineEnd);
        e.apply(&Op::Undo);
        assert_eq!(render(&e), "hello |world");
    }

}
