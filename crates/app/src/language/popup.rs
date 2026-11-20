use fast_editor::HoverTarget;
use fast_lsp::types::*;
use fast_theme::tok::Palette;
use rune::{Align, El, ScrollSpec, Side, col, row, text};

use super::{completion, response};
use crate::State;

pub(super) fn completion(palette: Palette, items: &[CompletionItem], selected: usize) -> El {
    let mut rows = col().key("completion-items").w_full();
    for (i, item) in items.iter().enumerate() {
        let kind = kind_label(item.kind);
        let detail = item
            .detail
            .as_deref()
            .and_then(|text| text.lines().next())
            .unwrap_or_default();
        let mut entry = row()
            .key(format!("completion-{i}"))
            .role("option")
            .label(&item.label)
            .checked(i == selected)
            .w_full()
            .h(super::ROW_HEIGHT)
            .px(6.0)
            .gap(6.0)
            .center_y()
            .preserve_focus_on_press(true)
            .on_press_tx(move |state: &mut State, tx| completion::accept(state, i, tx))
            .child(
                text(kind)
                    .font(palette.ui_text(10.0))
                    .w(41.0)
                    .color(palette.accent),
            )
            .child(
                text(&item.label)
                    .key("completion-label")
                    .font(palette.control_text(12.0))
                    .color(palette.text)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            );
        if !detail.is_empty() {
            entry = entry.child(
                text(detail)
                    .font(palette.ui_text(11.0))
                    .color(palette.muted_text)
                    .w(110.0)
                    .truncate(),
            );
        }
        rows = rows.child(if i == selected {
            entry.fill(palette.selected)
        } else {
            entry.hover(palette.hover)
        });
    }
    let documentation = items
        .get(selected)
        .and_then(|item| item.documentation.as_ref())
        .map(response::documentation);
    let mut popup = col()
        .key("completion-popup")
        .anchored_fit("caret-0", Side::Bottom, Align::Start, 4.0)
        .w(355.0)
        .min_h(0.0)
        .padding(2.0)
        .fill(palette.chrome)
        .border(1.0, palette.divider)
        .child(
            col()
                .key("completion-list")
                .role("listbox")
                .label("Typst completions")
                .h(items.len().min(7) as f32 * super::ROW_HEIGHT)
                .min_h(0.0)
                .shrink()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(rows),
        );
    if let Some(docs) = documentation {
        let summary = docs
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("```"))
            .unwrap_or_default();
        if !summary.is_empty() {
            popup = popup.child(
                text(summary)
                    .key("completion-docs")
                    .w_full()
                    .truncate()
                    .font(palette.ui_text(11.0))
                    .color(palette.muted_text)
                    .px(6.0)
                    .py(3.0),
            );
        }
    }
    popup
}

fn kind_label(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::FUNCTION | CompletionItemKind::METHOD) => "fn",
        Some(CompletionItemKind::VARIABLE | CompletionItemKind::CONSTANT) => "value",
        Some(CompletionItemKind::KEYWORD) => "kw",
        Some(CompletionItemKind::FILE | CompletionItemKind::FOLDER) => "path",
        Some(CompletionItemKind::SNIPPET) => "snippet",
        Some(CompletionItemKind::FIELD | CompletionItemKind::PROPERTY) => "field",
        _ => "symbol",
    }
}

pub(super) fn card(palette: Palette, title: &str, body: &str, hover: Option<&HoverTarget>) -> El {
    let anchor = if hover.is_some() {
        fast_editor::HOVER_ANCHOR
    } else {
        "caret-0"
    };
    let diagnostic = matches!(hover, Some(HoverTarget::Diagnostics { .. }));
    let mut parts = col().key("card-content").gap(5.0).w_full().min_w(0.0);
    if diagnostic {
        parts = parts.child(
            text(body)
                .font(palette.ui_text(12.0))
                .w_full()
                .wrap_anywhere()
                .color(palette.text),
        );
    } else {
        for (index, part) in body.split("```").enumerate().take(20) {
            let code = index % 2 != 0;
            let part = if code {
                part.split_once('\n').map_or(part, |(_, rest)| rest)
            } else {
                part
            };
            if part.trim().is_empty() {
                continue;
            }
            let font = if code {
                palette.ui_text(12.0).family(rune::tok::Family::SystemMono)
            } else {
                palette.ui_text(12.0)
            };
            let text: String = part.trim().chars().take(6000).collect();
            parts = parts.child(
                rune::text(text)
                    .font(font)
                    .w_full()
                    .wrap_anywhere()
                    .color(palette.text),
            );
        }
    }
    let mut card = col()
        .key("language-card")
        .anchored_fit(anchor, Side::Bottom, Align::Start, 4.0)
        .w(385.0)
        .min_h(0.0)
        .padding(8.0)
        .gap(5.0)
        .fill(palette.chrome)
        .border(1.0, palette.divider);
    if !diagnostic {
        card = card.h(if title == "Signature" { 96.0 } else { 146.0 });
    }
    if title == "Hover" || diagnostic {
        card = card.on_hover_change(super::keep_hover).on_drawn_layout_tx(
            |state: &mut State, bounds, tx| {
                state.language.hover_card_bounds = Some(bounds);
                tx.live_only();
            },
        );
    }
    if title != "Hover" {
        card = card.child(
            text(title)
                .key("card-title")
                .font(palette.control_text(12.0))
                .color(if title == "Error" {
                    palette.error_text
                } else {
                    palette.accent
                }),
        );
    }
    let viewport = col()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(parts);
    card.child(if diagnostic {
        viewport.max_h(120.0)
    } else {
        viewport.grow(1.0)
    })
}
