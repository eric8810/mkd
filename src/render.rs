//! Rendering: block tree → GPUI elements.

use gpui::{
    AnyElement, ElementId, FontWeight, Hsla, StyledText, TextStyle, div, prelude::*, px, relative,
};

use crate::parse::{Block, Inline};
use crate::theme::Theme;

pub struct RenderCtx<'a> {
    pub theme: &'a Theme,
    pub headings: &'a [(u32, String)],
}

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

fn render_quote(inner: &[Block], ctx: &RenderCtx, uid: &mut usize) -> AnyElement {
    let t = ctx.theme;
    div()
        .w_full()
        .my_1()
        .pl_3()
        .border_l_2()
        .border_color(t.quote_border)
        .text_color(t.muted)
        .children(render_blocks(inner, ctx, uid))
        .into_any()
}

fn render_list(list: &Block, ctx: &RenderCtx, uid: &mut usize) -> AnyElement {
    let t = ctx.theme;
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
            // Task list items carry the checkbox marker on their first paragraph.
            let task = item.iter().find_map(|b| match b {
                Block::Paragraph { task, .. } => *task,
                _ => None,
            });
            let marker: String = if let Some(checked) = task {
                if checked {
                    "☑".to_string()
                } else {
                    "☐".to_string()
                }
            } else if *ordered {
                format!("{}.", start + idx as u64)
            } else {
                "•".to_string()
            };
            let marker_color = if task.is_some() {
                t.info_accent
            } else {
                t.muted
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
                        .text_color(marker_color)
                        .child(marker),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(render_blocks(item, ctx, uid)),
                )
        }))
        .into_any()
}

fn render_code_block(code: &Block, t: &Theme, uid: &mut usize) -> AnyElement {
    let Block::Code {
        lang,
        text,
        highlight,
        line_numbers,
        title,
    } = code
    else {
        unreachable!()
    };
    let id = *uid;
    *uid += 1;

    let header_text = title
        .as_deref()
        .or(if lang.is_empty() { None } else { Some(lang.as_str()) });

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

    if let Some(h) = header_text {
        el = el.child(
            div()
                .mb_1()
                .px_1()
                .text_size(px(11.0))
                .text_color(t.muted)
                .font_weight(FontWeight::SEMIBOLD)
                .child(h.to_string()),
        );
    }

    for (i, line) in text.split('\n').enumerate() {
        let line_no = i + 1;
        let highlighted = highlight
            .iter()
            .any(|(a, b)| line_no >= *a && line_no <= *b);
        let mut row = div().flex().flex_row();
        if highlighted {
            row = row.bg(t.line_highlight);
        }
        if *line_numbers {
            row = row.child(
                div()
                    .flex_none()
                    .w(px(30.0))
                    .pr_1()
                    .text_right()
                    .text_size(px(11.0))
                    .text_color(t.muted)
                    .child(line_no.to_string()),
            );
        }
        row = row.child(
            div()
                .font_family("Menlo")
                .text_size(px(13.0))
                .line_height(relative(1.5))
                .text_color(t.code_fg)
                .whitespace_nowrap()
                .child(line.to_string()),
        );
        el = el.child(row);
    }
    el.into_any()
}

fn render_math(math: &str, t: &Theme) -> AnyElement {
    div()
        .w_full()
        .my_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(t.code_bg)
        .text_center()
        .font_family("Menlo")
        .text_size(px(13.0))
        .line_height(relative(1.5))
        .text_color(t.code_fg)
        .child(math.to_string())
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

fn render_container(container: &Block, ctx: &RenderCtx, uid: &mut usize) -> AnyElement {
    let t = ctx.theme;
    let Block::Container { kind, title, inner } = container else {
        unreachable!()
    };
    let (accent, bg): (Hsla, Hsla) = match kind.as_str() {
        "tip" => (t.tip_accent, t.tip_bg),
        "warning" | "warn" => (t.warn_accent, t.warn_bg),
        "danger" => (t.danger_accent, t.danger_bg),
        "info" => (t.info_accent, t.info_bg),
        "note" => (t.info_accent, t.info_bg),
        _ => (t.neutral_accent, t.neutral_bg),
    };

    let header_text = title.clone().unwrap_or_else(|| match kind.as_str() {
        "tip" => "提示".to_string(),
        "warning" | "warn" => "警告".to_string(),
        "danger" => "危险".to_string(),
        "info" => "信息".to_string(),
        "note" => "备注".to_string(),
        "details" => "详情".to_string(),
        other => other.to_uppercase(),
    });

    div()
        .w_full()
        .my_2()
        .px_3()
        .py_2()
        .rounded_md()
        .border_l(px(3.0))
        .border_color(accent)
        .bg(bg)
        .child(
            div()
                .mb_1()
                .font_weight(FontWeight::BOLD)
                .text_size(px(13.0))
                .text_color(accent)
                .child(header_text),
        )
        .children(render_blocks(inner, ctx, uid))
        .into_any()
}

fn render_definition_list(dl: &Block, ctx: &RenderCtx, uid: &mut usize) -> AnyElement {
    let t = ctx.theme;
    let Block::DefinitionList(items) = dl else {
        unreachable!()
    };
    div()
        .w_full()
        .my_1()
        .flex()
        .flex_col()
        .children(items.iter().map(|(term, defs)| {
            div()
                .mb_2()
                .child(render_inline(term, t.heading, 15.0, FontWeight::SEMIBOLD))
                .children(defs.iter().map(|def| {
                    div()
                        .pl_3()
                        .children(render_blocks(def, ctx, uid))
                }))
        }))
        .into_any()
}

fn render_toc(ctx: &RenderCtx) -> AnyElement {
    let t = ctx.theme;
    let entries: Vec<AnyElement> = ctx
        .headings
        .iter()
        .filter(|(level, _)| *level >= 2 && *level <= 3)
        .map(|(level, text)| {
            let indent = if *level == 3 { 16.0 } else { 4.0 };
            div()
                .pl(px(indent))
                .py_0p5()
                .text_size(px(14.0))
                .text_color(t.fg)
                .child(text.clone())
                .into_any()
        })
        .collect();

    if entries.is_empty() {
        return div().into_any();
    }
    div()
        .w_full()
        .my_2()
        .px_3()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(t.rule)
        .child(
            div()
                .mb_1()
                .font_weight(FontWeight::BOLD)
                .text_size(px(13.0))
                .text_color(t.heading)
                .child("目录"),
        )
        .children(entries)
        .into_any()
}

pub fn render_blocks(blocks: &[Block], ctx: &RenderCtx, uid: &mut usize) -> Vec<AnyElement> {
    blocks
        .iter()
        .map(|block| match block {
            Block::Heading { level, inline } => render_heading(*level, inline, ctx.theme),
            Block::Paragraph { inline, task } => {
                if task.is_none() && inline.0.trim() == "[[toc]]" {
                    render_toc(ctx)
                } else {
                    render_paragraph(inline, ctx.theme)
                }
            }
            Block::List { .. } => render_list(block, ctx, uid),
            Block::Code { .. } => render_code_block(block, ctx.theme, uid),
            Block::Quote(inner) => render_quote(inner, ctx, uid),
            Block::Table { .. } => render_table(block, ctx.theme, uid),
            Block::Container { .. } => render_container(block, ctx, uid),
            Block::DefinitionList(_) => render_definition_list(block, ctx, uid),
            Block::Math { text } => render_math(text, ctx.theme),
            Block::Rule => div().w_full().h_px().my_4().bg(ctx.theme.rule).into_any(),
            Block::Html(h) => div()
                .w_full()
                .mb_2()
                .text_color(ctx.theme.muted)
                .child(h.clone())
                .into_any(),
        })
        .collect()
}
