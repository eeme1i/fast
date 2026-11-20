use fast_lsp::{Compilation, Status, types::*};
use fast_theme::tok::Palette;
use rune::{Align, El, Key, ScrollSpec, Side, col, row, text, text_input};

use super::{Command, Entry, Popup, Results, actions, invoke, pin_dialog, popup, response};
use crate::{State, view::action};

pub(crate) fn status(state: &State, palette: Palette, size: f32) -> Option<El> {
    let status = state.language.status.as_ref()?;
    let (label, help) = match status {
        Status::Starting => ("Tinymist starting", None),
        Status::Unavailable(message) => ("Tinymist unavailable", Some(message.as_str())),
        Status::Restarting(message) => ("Tinymist restarting", Some(message.as_str())),
        Status::Ready { .. } => (
            match state.language.compile.as_ref().map(|status| status.status) {
                Some(Compilation::Compiling) => "Compiling",
                Some(Compilation::CompileSuccess) => "Tinymist ready",
                Some(Compilation::CompileError) => "Compile error",
                None => "Tinymist ready",
            },
            None,
        ),
    };
    let danger = matches!(status, Status::Unavailable(_) | Status::Restarting(_))
        || state
            .language
            .compile
            .as_ref()
            .is_some_and(|compile| compile.status == Compilation::CompileError);
    let mut indicator = text(label)
        .key("tinymist-status")
        .font(palette.ui_text(size.min(12.0)))
        .color(if danger {
            palette.error_text
        } else {
            palette.muted_text
        });
    if let Some(help) = help {
        indicator = indicator.help(help);
    }
    let mut status = row()
        .key("language-status")
        .gap(6.0)
        .center_y()
        .child(indicator);
    if let Some(path) = &state.language.pinned {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        status = status.child(
            row()
                .key("pinned-main")
                .role("button")
                .label(format!("Pinned main file {name}"))
                .focusable(state.language.ready())
                .on_press_tx(pin_dialog)
                .px(4.0)
                .help(path.display().to_string())
                .hover(palette.hover)
                .child(
                    text(format!("Main  {name}"))
                        .w(88.0)
                        .truncate()
                        .font(palette.ui_text(size.min(12.0)))
                        .color(palette.accent),
                ),
        );
    }
    Some(status)
}

pub(crate) fn overlays(state: &State, palette: Palette) -> Vec<El> {
    let mut overlays = Vec::new();
    if state.language.menu {
        overlays.push(menu(state, palette));
    }
    if let Some(popup) = &state.language.popup {
        overlays.push(match popup {
            Popup::Completion {
                items, selected, ..
            } => popup::completion(palette, items, *selected),
            Popup::Card { title, body, hover } => popup::card(palette, title, body, hover.as_ref()),
        });
    }
    if let Some(rename) = &state.language.rename {
        let field = text_input(rename, palette.ui_text(13.0))
            .key("rename-symbol")
            .label("New symbol name")
            .single_line()
            .placeholder("New name", palette.muted_text)
            .fill(palette.canvas)
            .border(1.0, palette.divider)
            .color(palette.text)
            .caret(palette.text, palette.selected)
            .h(24.0)
            .px(6.0)
            .pt(3.0)
            .grow(1.0)
            .min_w(0.0)
            .on_edit(|state: &mut State| state.language.rename.get_or_insert_with(Default::default))
            .on_submit(actions::rename)
            .on_input_key(|state: &mut State, event, tx| {
                if event.key == Key::Escape {
                    state.language.rename = None;
                    tx.focus(fast_editor::VIEWPORT, false);
                    tx.instant();
                    return true;
                }
                false
            });
        overlays.push(
            row()
                .key("rename-card")
                .anchored("caret-0", Side::Bottom, Align::Start, 4.0)
                .w(285.0)
                .padding(5.0)
                .gap(6.0)
                .fill(palette.chrome)
                .border(1.0, palette.divider)
                .child(field)
                .child(action(
                    "apply-rename",
                    "Rename",
                    palette,
                    13.0,
                    true,
                    actions::rename,
                )),
        );
    }
    overlays
}

fn menu(state: &State, palette: Palette) -> El {
    let items = [
        ("language-complete", "Complete", Command::Complete),
        ("language-hover", "Hover documentation", Command::Hover),
        ("language-signature", "Signature help", Command::Signature),
        (
            "language-definition",
            "Go to definition",
            Command::Definition,
        ),
        (
            "language-references",
            "Find references",
            Command::References,
        ),
        ("language-format", "Format document", Command::Format),
        ("language-rename", "Rename symbol", Command::Rename),
        ("language-outline", "Outline", Command::Outline),
        ("language-problems", "Problems", Command::Problems),
        ("language-actions", "Code actions", Command::Actions),
        ("language-fold", "Toggle fold at caret", Command::Fold),
    ];
    let mut commands = col().key("language-commands").gap(1.0);
    for (key, label, command) in items {
        commands = commands.child(menu_item(palette, key, label, move |state, tx| {
            tx.focus(fast_editor::VIEWPORT, false);
            invoke(state, command, tx);
        }));
    }
    commands = commands.child(menu_item(
        palette,
        "language-pin",
        "Pin main file…",
        pin_dialog,
    ));
    if state.language.pinned.is_some() {
        commands = commands.child(menu_item(
            palette,
            "language-unpin",
            "Unpin main file",
            |state, tx| {
                invoke(state, Command::Unpin, tx);
                tx.focus(fast_editor::VIEWPORT, false);
            },
        ));
    }
    col()
        .key("language-popup")
        .anchored_fit("language-menu", Side::Bottom, Align::Start, 2.0)
        .w(215.0)
        .h(if state.language.pinned.is_some() {
            315.0
        } else {
            291.0
        })
        .min_h(0.0)
        .padding(2.0)
        .fill(palette.chrome)
        .border(1.0, palette.divider)
        .child(
            col()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .focus_group()
                .child(commands),
        )
}

fn menu_item(
    palette: Palette,
    key: &'static str,
    label: &'static str,
    f: impl Fn(&mut State, &mut rune::Tx) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .h(super::ROW_HEIGHT)
        .w_full()
        .px(8.0)
        .center_y()
        .hover(palette.hover)
        .focus_ring(1.0, -1.0, palette.accent)
        .on_press_tx(f)
        .child(
            text(label)
                .font(palette.ui_text(12.0))
                .color(palette.text)
                .a11y_hidden(),
        )
}

pub(crate) fn results(state: &State, palette: Palette) -> Option<El> {
    let result = state.language.result.as_ref()?;
    let mut content = col().key("language-results-content").w_full();
    let (title, empty) = match result {
        Results::Locations(title, items) => {
            content = entries(content, items, palette);
            (*title, "No locations found")
        }
        Results::Actions(actions) => {
            for (index, action) in actions.iter().enumerate() {
                let (title, disabled) = match action {
                    CodeActionOrCommand::Command(command) => (&command.title, None),
                    CodeActionOrCommand::CodeAction(action) => {
                        (&action.title, action.disabled.as_ref())
                    }
                };
                let mut item = result_row(palette, format!("code-action-{index}"), title, "");
                if let Some(reason) = disabled {
                    item = item.disabled(true).help(&reason.reason);
                } else {
                    item = item
                        .focusable(true)
                        .on_press_tx(move |state: &mut State, tx| {
                            actions::apply_action(state, index, tx)
                        });
                }
                content = content.child(item);
            }
            ("Code actions", "No code actions at the caret")
        }
    };
    let is_empty = match result {
        Results::Locations(_, items) => items.is_empty(),
        Results::Actions(actions) => actions.is_empty(),
    };
    if is_empty {
        content = content.child(
            text(empty)
                .font(palette.ui_text(12.0))
                .color(palette.muted_text)
                .px(8.0)
                .py(5.0),
        );
    }
    Some(
        col()
            .key("language-results")
            .h(142.0)
            .w_full()
            .min_h(0.0)
            .fill(palette.chrome)
            .border_sides([1.0, 0.0, 0.0, 0.0], palette.divider)
            .child(
                row()
                    .key("results-header")
                    .h(26.0)
                    .px(8.0)
                    .w_full()
                    .center_y()
                    .justify_between()
                    .child(
                        text(title)
                            .key("results-title")
                            .font(palette.control_text(12.0))
                            .color(palette.text),
                    )
                    .child(action(
                        "close-language-results",
                        "Close",
                        palette,
                        12.0,
                        true,
                        |state, tx| {
                            state.language.result = None;
                            tx.focus(fast_editor::VIEWPORT, false);
                            tx.instant();
                        },
                    )),
            )
            .child(
                col()
                    .key("results-scroll")
                    .grow(1.0)
                    .min_h(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .fill(palette.canvas)
                    .child(content),
            ),
    )
}

pub(crate) fn tool(state: &State, tool: crate::workbench::Tool, palette: Palette) -> El {
    let (items, empty) = if tool == crate::workbench::Tool::Outline {
        (&state.language.outline, "No document symbols")
    } else {
        (&state.language.problems, "No problems reported")
    };
    let content = col().w_full();
    let content = if items.is_empty() {
        content.child(
            text(empty)
                .font(palette.ui_text(12.0))
                .color(palette.muted_text)
                .w_full()
                .px(8.0)
                .py(5.0)
                .truncate(),
        )
    } else {
        entries(content, items, palette)
    };
    col()
        .key("tool-language-scroll")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

fn entries(mut content: El, entries: &[Entry], palette: Palette) -> El {
    for (i, entry) in entries.iter().enumerate() {
        let location = entry.location.clone();
        content = content.child(
            result_row(
                palette,
                format!("language-result-{i}"),
                &entry.label,
                &entry.detail,
            )
            .focusable(true)
            .on_press_tx(move |state: &mut State, tx| response::go(state, location.clone(), tx)),
        );
    }
    content
}

fn result_row(palette: Palette, key: String, label: &str, detail: &str) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .help(label)
        .h(22.0)
        .w_full()
        .px(8.0)
        .gap(8.0)
        .center_y()
        .hover(palette.hover)
        .focus_ring(1.0, -1.0, palette.accent)
        .child(
            text(label)
                .font(palette.ui_text(12.0))
                .color(palette.text)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            text(detail)
                .font(palette.ui_text(11.0))
                .color(palette.muted_text)
                .max_w(110.0)
                .shrink()
                .truncate(),
        )
}
