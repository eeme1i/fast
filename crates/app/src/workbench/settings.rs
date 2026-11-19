use fast_core::config::{
    Density, EditorFont, FontFamily, PreviewDock, Sidebar, TabPlacement, Theme,
};
use fast_theme::tok::{self, Palette};
use rune::{Align, El, Side, Tx, col, row, text};

use crate::{State, view::action};

pub(crate) fn open(state: &mut State, tx: &mut Tx) {
    state.settings_menu = true;
    state.language.menu = false;
    state.language.popup = None;
    tx.focus(theme_label(state.ui.theme).0, false);
    tx.instant();
}

fn close_settings(state: &mut State, tx: &mut Tx) {
    state.settings_menu = false;
    tx.instant();
}

pub(crate) fn overlay(state: &State, palette: Palette, hairline: f32) -> El {
    col()
        .key("settings-popup")
        .role("menu")
        .label("Settings")
        .anchored_fit("settings-menu", Side::Bottom, Align::Start, 2.0)
        .focus_group()
        .modal_tx(true, close_settings)
        .w(336.0)
        .padding(4.0)
        .gap(2.0)
        .fill(palette.chrome)
        .border(hairline, palette.divider)
        .child(
            text("Theme")
                .key("theme-heading")
                .px(6.0)
                .font(palette.ui_text(11.0))
                .color(palette.muted_text),
        )
        .child(theme_option(state, palette, Theme::DarculaClassic))
        .child(theme_option(state, palette, Theme::IntelliJLightClassic))
        .child(preferences(state, palette, hairline))
}

fn theme_label(theme: Theme) -> (&'static str, &'static str) {
    match theme {
        Theme::DarculaClassic => ("darcula", "Darcula Classic"),
        Theme::IntelliJLightClassic => ("light", "IntelliJ Light Classic"),
    }
}

fn theme_option(state: &State, palette: Palette, theme: Theme) -> El {
    let (key, label) = theme_label(theme);
    let selected = state.ui.theme == theme;
    let item = row()
        .key(key)
        .role("menuitemradio")
        .label(label)
        .checked(selected)
        .focusable(true)
        .focus_selected(selected)
        .focus_highlight(if selected {
            palette.selected_hover
        } else {
            palette.hover
        })
        .h(tok::STATUS_HEIGHT)
        .px(6.0)
        .center_y()
        .justify_between()
        .on_press_tx(move |state: &mut State, tx| {
            state.ui.theme = theme;
            close_settings(state, tx);
        })
        .child(
            text(label)
                .key("label")
                .a11y_hidden()
                .font(palette.control_text(state.ui.font_size))
                .color(palette.text),
        );
    if selected {
        item.fill(palette.selected).child(
            text("✓")
                .key("theme-check")
                .font(palette.control_text(12.0))
                .color(palette.text)
                .a11y_hidden(),
        )
    } else {
        item
    }
}

fn preferences(state: &State, palette: Palette, hairline: f32) -> El {
    let ui = state.ui;
    let editor = state.editor_config;
    let family = match ui.font_family {
        FontFamily::Inter => "Inter",
        FontFamily::SystemMono => "System mono",
        FontFamily::Serif => "Serif",
    };
    let density = match ui.density {
        Density::Compact => "Compact",
        Density::Standard => "Standard",
        Density::Comfortable => "Comfortable",
    };
    let side = match ui.sidebar {
        Sidebar::Left => "Left",
        Sidebar::Right => "Right",
    };
    let preview = match ui.preview_dock {
        PreviewDock::Left => "Left",
        PreviewDock::Right => "Right",
        PreviewDock::Bottom => "Bottom",
        PreviewDock::Tab => "Tab",
    };
    let editor_family = match editor.font_family {
        EditorFont::SystemMono => "System mono",
        EditorFont::Mono => "Embedded mono",
    };
    col()
        .key("preferences")
        .w_full()
        .pt(3.0)
        .border_sides([hairline, 0.0, 0.0, 0.0], palette.divider)
        .child(
            row()
                .w_full()
                .gap(5.0)
                .child(setting(palette, "UI font", "ui-family", family, |state| {
                    state.ui.font_family = match state.ui.font_family {
                        FontFamily::Inter => FontFamily::SystemMono,
                        FontFamily::SystemMono => FontFamily::Serif,
                        FontFamily::Serif => FontFamily::Inter,
                    };
                }))
                .child(stepper(
                    palette,
                    "UI size",
                    "ui-size",
                    ui.font_size,
                    11.0,
                    16.0,
                    |state, delta| {
                        state.ui.font_size = (state.ui.font_size + delta).clamp(11.0, 16.0);
                    },
                )),
        )
        .child(
            row()
                .w_full()
                .gap(5.0)
                .child(setting(
                    palette,
                    "Density",
                    "ui-density",
                    density,
                    |state| {
                        state.ui.density = match state.ui.density {
                            Density::Standard => Density::Compact,
                            Density::Compact => Density::Comfortable,
                            Density::Comfortable => Density::Standard,
                        };
                    },
                ))
                .child(setting(palette, "Sidebar", "sidebar-side", side, |state| {
                    state.ui.sidebar = match state.ui.sidebar {
                        Sidebar::Left => Sidebar::Right,
                        Sidebar::Right => Sidebar::Left,
                    };
                    state.workbench.sidebar_changed();
                    state.workbench.save_layout();
                })),
        )
        .child(
            row()
                .w_full()
                .gap(5.0)
                .child(setting(
                    palette,
                    "Editor font",
                    "editor-family",
                    editor_family,
                    |state| {
                        state.editor_config.font_family = match state.editor_config.font_family {
                            EditorFont::SystemMono => EditorFont::Mono,
                            EditorFont::Mono => EditorFont::SystemMono,
                        };
                    },
                ))
                .child(stepper(
                    palette,
                    "Editor size",
                    "editor-size",
                    editor.font_size,
                    10.0,
                    24.0,
                    |state, delta| {
                        let editor = &mut state.editor_config;
                        editor.font_size = (editor.font_size + delta).clamp(10.0, 24.0);
                        editor.line_height = editor.line_height.max(editor.font_size + 4.0);
                    },
                )),
        )
        .child(
            row()
                .w_full()
                .gap(5.0)
                .child(setting(
                    palette,
                    "Preview",
                    "preview-placement",
                    preview,
                    |state| {
                        state.ui.preview_dock = match state.ui.preview_dock {
                            PreviewDock::Right => PreviewDock::Bottom,
                            PreviewDock::Bottom => PreviewDock::Left,
                            PreviewDock::Left => PreviewDock::Tab,
                            PreviewDock::Tab => PreviewDock::Right,
                        };
                        state
                            .workbench
                            .preview_toggled(state.preview.visible, state.ui.preview_dock);
                    },
                ))
                .child(setting(
                    palette,
                    "Ligatures",
                    "editor-ligatures",
                    if editor.ligatures { "On" } else { "Off" },
                    |state| {
                        state.editor_config.ligatures = !state.editor_config.ligatures;
                    },
                )),
        )
        .child(
            row()
                .w_full()
                .gap(5.0)
                .child(stepper(
                    palette,
                    "Line height",
                    "editor-line-height",
                    editor.line_height,
                    editor.font_size + 4.0,
                    42.0,
                    |state, delta| {
                        let editor = &mut state.editor_config;
                        editor.line_height =
                            (editor.line_height + delta).clamp(editor.font_size + 4.0, 42.0);
                    },
                ))
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .center_y()
                        .gap(2.0)
                        .child(setting(
                            palette,
                            "Tabs",
                            "tabs-placement",
                            if ui.tabs == TabPlacement::Top {
                                "Top"
                            } else {
                                "Bottom"
                            },
                            |state| {
                                state.ui.tabs = match state.ui.tabs {
                                    TabPlacement::Top => TabPlacement::Bottom,
                                    TabPlacement::Bottom => TabPlacement::Top,
                                };
                            },
                        ))
                        .child(
                            action("settings-done", "Done", palette, 12.0, true, close_settings)
                                .h(22.0),
                        ),
                ),
        )
}

fn setting(
    palette: Palette,
    label: &str,
    key: &'static str,
    value: &str,
    change: impl Fn(&mut State) + 'static,
) -> El {
    let control = super::item(palette, key, value, 22.0, false, move |state, tx| {
        change(state);
        tx.instant();
    })
    .label(format!("{label}: {value}"))
    .help(format!("Change {label}"));
    setting_row(palette, label).child(control)
}

fn stepper(
    palette: Palette,
    label: &str,
    key: &str,
    size: f32,
    min: f32,
    max: f32,
    change: fn(&mut State, f32),
) -> El {
    let button = |sign, suffix, delta| {
        super::item(
            palette,
            format!("{key}-{suffix}"),
            sign,
            22.0,
            false,
            move |state, tx| {
                change(state, delta);
                tx.instant();
            },
        )
        .w(18.0)
        .px(0.0)
        .justify_center()
        .label(format!("{suffix} {label}"))
        .disabled(if delta < 0.0 {
            size <= min
        } else {
            size >= max
        })
    };
    setting_row(palette, label)
        .child(button("−", "decrease", -1.0))
        .child(
            text(format!("{size:.0}"))
                .key(key)
                .font(palette.ui_text(11.0))
                .color(palette.text)
                .w(19.0)
                .align(Align::Center),
        )
        .child(button("+", "increase", 1.0))
}

fn setting_row(palette: Palette, label: &str) -> El {
    row().h(23.0).grow(1.0).min_w(0.0).center_y().px(3.0).child(
        text(label)
            .font(palette.ui_text(11.0))
            .color(palette.muted_text)
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
    )
}
