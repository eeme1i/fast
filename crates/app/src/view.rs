use fast_theme::tok::{self, Palette};
use rune::{Anim, Color, Cx, El, HelpLook, LiveRegion, Spring, Transition, Tx, col, row, text};

use crate::{State, files, language, preview, workbench};

pub fn window(state: &State, cx: &mut Cx<State>) -> El {
    let palette = tok::palette(state.ui.theme).with_font(state.ui.font_family);
    let hairline = 1.0 / cx.env().scale;
    cx.help_look(HelpLook::system(
        state.ui.theme == fast_core::config::Theme::DarculaClassic,
    ));
    let mut root = col()
        .key("window")
        .role("window")
        .w_full()
        .h_full()
        .fill(palette.canvas)
        .instant_changes()
        .transition(Transition::none())
        .motion(Spring::NONE)
        .hover_motion_within(Anim::INSTANT, Anim::INSTANT)
        .on_paste(fast_editor::paste::<State>)
        .child(document_bar(state, palette, hairline));
    root = root.child(workbench::view(state, palette, cx.env().scale));
    if let Some(results) = language::view::results(state, palette) {
        root = root.child(results);
    }
    root = root.child(status_bar(state, palette, hairline));
    for overlay in language::view::overlays(state, palette) {
        root = root.child(overlay);
    }
    if state.settings_menu {
        root = root.child(workbench::settings::overlay(state, palette, hairline));
    }
    if let Some(overlay) = language::workspace::overlay(state, palette) {
        root = root.child(overlay);
    }
    for overlay in workbench::overlays(state, palette, cx.env().scale) {
        root = root.child(overlay);
    }
    if let Some(id) = state.files.confirm_close {
        root = root.child(confirm_close(state, palette, id));
    }
    if let Some(overlay) = preview::export_overlay(state, palette) {
        root = root.child(overlay);
    }
    root
}

fn document_bar(state: &State, palette: Palette, hairline: f32) -> El {
    let compact = state.workbench.width() < 540.0;
    let size = if compact && preview::available(state) {
        state.ui.font_size.min(14.0)
    } else {
        state.ui.font_size
    };
    let enabled = !state.files.busy();
    let button = |key, label, enabled, on_press: fn(&mut State, &mut Tx)| {
        action(key, label, palette, size, enabled, on_press).px(4.0)
    };
    let settings = button("settings-menu", "Settings", true, workbench::settings::open)
        .expanded(state.settings_menu);
    let mut bar = row()
        .key("document-bar")
        .w_full()
        .h(tok::row_height(state.ui.density) + 4.0)
        .px(6.0)
        .center_y()
        .gap(2.0)
        .scroll_xy()
        .clip_rounded(0.0)
        .fill(palette.chrome)
        .border_sides([0.0, 0.0, hairline, 0.0], palette.divider)
        .child(
            button(
                "new-file",
                if compact { "+" } else { "New" },
                enabled,
                files::create,
            )
            .label("New file")
            .help("New file (Command/Ctrl+N)"),
        )
        .child(button("open-file", "Open", enabled, files::open))
        .child(button("save-file", "Save", enabled, |state, tx| {
            files::save(state, false, tx)
        }))
        .child(button("find-file", "Find", true, |state, tx| {
            state.editor.find(false, tx)
        }))
        .child(button("replace-file", "Replace", true, |state, tx| {
            state.editor.find(true, tx)
        }))
        .child(if state.settings_menu {
            settings
                .fill(palette.selected)
                .hover(palette.selected_hover)
        } else {
            settings
        });
    if state.language.status.is_some() && preview::available(state) {
        bar = bar.child(
            button(
                "language-menu",
                if compact { "LSP" } else { "Language" },
                state.language.ready(),
                |state, tx| {
                    state.language.menu = !state.language.menu;
                    state.language.popup = None;
                    tx.instant();
                },
            )
            .label("Language tools")
            .help("Language tools"),
        );
    }
    if preview::available(state) {
        bar = bar
            .child(button("toggle-preview", "Preview", true, preview::toggle))
            .child(button(
                "export-document",
                "Export",
                true,
                preview::export_dialog,
            ));
    }
    bar.child(
        button(
            "command-palette",
            if compact { "⌘" } else { "Commands" },
            true,
            workbench::commands,
        )
        .label("Commands")
        .help("Command Palette (Ctrl+Shift+P)"),
    )
}

pub(crate) fn action(
    key: &'static str,
    label: &'static str,
    palette: Palette,
    size: f32,
    enabled: bool,
    on_press: impl Fn(&mut State, &mut Tx) + 'static,
) -> El {
    let button = row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(enabled)
        .disabled(!enabled)
        .h(24.0)
        .px(6.0)
        .center()
        .focus_ring(1.0, -1.0, palette.accent)
        .child(
            text(label)
                .font(palette.control_text(size))
                .color(if enabled {
                    palette.text
                } else {
                    palette.muted_text
                })
                .a11y_hidden(),
        );
    if enabled {
        button.hover(palette.hover).on_press_tx(on_press)
    } else {
        button
    }
}

fn confirm_close(state: &State, palette: Palette, id: u64) -> El {
    let dismiss = |state: &mut State, tx: &mut Tx| {
        state.files.confirm_close = None;
        tx.focus(fast_editor::VIEWPORT, false);
        tx.instant();
    };
    let size = state.ui.font_size;
    col()
        .key("discard-confirmation")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .center()
        .fill(Color::BLACK.alpha(0.42))
        .role("dialog")
        .label("Unsaved changes")
        .modal_tx(false, dismiss)
        .child(
            col()
                .w(350.0)
                .padding(12.0)
                .gap(10.0)
                .fill(palette.chrome)
                .border(1.0, palette.divider)
                .child(
                    text("Discard unsaved changes?")
                        .font(palette.control_text(size))
                        .color(palette.text),
                )
                .child(
                    row()
                        .gap(4.0)
                        .child(action(
                            "keep-editing",
                            "Keep editing",
                            palette,
                            size,
                            true,
                            dismiss,
                        ))
                        .child(action(
                            "discard-tab",
                            "Discard and close",
                            palette,
                            size,
                            true,
                            move |state, tx| files::discard(state, id, tx),
                        )),
                ),
        )
}

fn status_bar(state: &State, palette: Palette, hairline: f32) -> El {
    let error = state
        .config_error
        .as_deref()
        .or(state.watcher_error.as_deref());
    let label = if state.config_error.is_some() {
        Some("Configuration error")
    } else if state.watcher_error.is_some() {
        Some("Configuration watch unavailable")
    } else if state.config_loaded {
        Some("Configuration loaded")
    } else {
        None
    };
    let language = language::view::status(state, palette, state.ui.font_size);
    let export = preview::export_status(state);
    let mut bar = row()
        .key("status-bar")
        .h(tok::row_height(state.ui.density))
        .w_full()
        .px(8.0)
        .center_y()
        .gap(10.0)
        .fill(palette.chrome)
        .border_sides([hairline, 0.0, 0.0, 0.0], palette.divider);
    bar = if label.is_some() || language.is_some() || export.is_some() {
        bar.justify_between()
    } else {
        bar.justify_end()
    };
    let font = palette.ui_text((state.ui.font_size - 1.0).max(11.0));
    if let Some(label) = label {
        let path = state
            .config_path
            .as_ref()
            .map(|path| path.display().to_string());
        let detail = match (error, path) {
            (Some(error), Some(path)) => format!("{path}\n{error}"),
            (Some(error), None) => error.to_owned(),
            (None, Some(path)) => path,
            (None, None) => label.to_owned(),
        };
        bar = bar.child(
            text(label)
                .key("config-status")
                .live_region(LiveRegion::Polite)
                .font(font)
                .color(if error.is_some() {
                    palette.error_text
                } else {
                    palette.muted_text
                })
                .help(detail),
        );
    }
    if let Some(language) = language {
        bar = bar.child(language);
    }
    if let Some(message) = export {
        bar = bar.child(
            text(message)
                .key("export-status")
                .font(font)
                .color(palette.muted_text)
                .truncate()
                .shrink()
                .help(message),
        );
    }
    if let Some(message) = state
        .language
        .message
        .as_deref()
        .or(state.workbench.layout_error.as_deref())
        .or_else(|| state.files.status())
    {
        bar = bar.child(
            text(message)
                .key("file-status")
                .font(font)
                .color(palette.muted_text)
                .truncate()
                .shrink()
                .help(message),
        );
    } else {
        let document = state.editor.document();
        let head = document.selection().head;
        let line = document.line_at(head);
        let column = document
            .slice(document.line_range(line).start..head)
            .chars()
            .count();
        let ending = if document.ending() == "\r\n" {
            "CRLF"
        } else {
            "LF"
        };
        bar = bar.child(
            text(format!(
                "Ln {}, Col {}   UTF-8   {ending}",
                line + 1,
                column + 1
            ))
            .key("position")
            .font(font)
            .color(palette.muted_text)
            .min_w(0.0)
            .shrink()
            .truncate(),
        );
    }
    bar
}
