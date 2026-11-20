mod dock;
mod layout_store;
mod picker;
pub(crate) mod project;
pub(crate) mod settings;

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use fast_core::config::{PreviewDock, TabPlacement};
use fast_theme::tok::{Palette, row_height};
use rune::{El, ScrollSpec, TextEdit, Tx, Waker, col, row, text, text_input};

use crate::{State, files, preview};
pub(crate) use dock::{Tool, toggle};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub(crate) struct Workbench {
    layout: dock::Layout,
    layout_store: layout_store::Store,
    pub layout_error: Option<String>,
    pub project: project::Project,
    picker: picker::Picker,
    preview_tab: bool,
    pin_menu: Option<(String, PathBuf)>,
    compiler: VecDeque<String>,
    last_preview: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkbenchSession {
    geometry: layout_store::Preferences,
    tools: Vec<Tool>,
    preview_visible: bool,
    preview_tab: bool,
}

impl Workbench {
    pub fn session(&self, preview: &preview::Preview) -> WorkbenchSession {
        WorkbenchSession {
            geometry: layout_store::Preferences::capture(&self.layout),
            tools: Tool::ALL
                .into_iter()
                .filter(|tool| self.is_open(*tool))
                .collect(),
            preview_visible: preview.visible,
            preview_tab: self.preview_tab,
        }
    }

    pub fn restore_session(&mut self, session: WorkbenchSession, preview: &mut preview::Preview) {
        self.layout_store.supersede();
        session.geometry.restore(&mut self.layout);
        for tool in Tool::ALL {
            self.layout.open(tool, session.tools.contains(&tool));
        }
        preview.visible = session.preview_visible;
        self.preview_tab = session.preview_tab;
    }

    pub(crate) fn width(&self) -> f32 {
        self.layout.width()
    }

    pub(crate) fn setup(&mut self, config: Option<&std::path::Path>, waker: Waker) {
        self.project.setup(waker.clone());
        self.picker.setup(waker.clone());
        self.layout_store.setup(config, waker);
    }

    pub(crate) fn show_editor(&mut self) {
        self.preview_tab = false;
    }

    pub(crate) fn preview_toggled(&mut self, visible: bool, placement: PreviewDock) {
        self.preview_tab = visible && placement == PreviewDock::Tab;
    }

    pub(crate) fn sidebar_changed(&mut self) {
        self.layout.reset_sidebar();
    }

    fn save_layout(&mut self) {
        self.layout_store.save(&self.layout);
    }
}

pub(crate) fn poll(state: &mut State, tx: &mut Tx) -> bool {
    let mut changed = project::poll(state, tx);
    if picker::poll(state) {
        changed = true;
        tx.instant();
    }
    let workbench = &mut state.workbench;
    if workbench
        .layout_store
        .poll(&mut workbench.layout, &mut workbench.layout_error)
    {
        changed = true;
        tx.instant();
    }
    let status = preview::report(&state.preview);
    if state.workbench.last_preview.as_deref() != status {
        state.workbench.last_preview = status.map(str::to_owned);
        if let Some(status) = status {
            log(state, format!("Preview  {status}"));
        }
        changed = true;
    }
    changed
}

pub(crate) fn log(state: &mut State, message: String) {
    let log = &mut state.workbench.compiler;
    if log.back() == Some(&message) {
        return;
    }
    if log.len() == 100 {
        log.pop_front();
    }
    log.push_back(message);
}

fn compiler(state: &State, palette: Palette) -> El {
    let mut content = col().w_full();
    if state.workbench.compiler.is_empty() {
        content = content.child(project::message(palette, "No compiler output"));
    }
    for message in &state.workbench.compiler {
        content = content.child(
            text(message)
                .font(palette.ui_text(12.0).mono())
                .color(palette.text)
                .w_full()
                .px(8.0)
                .py(3.0)
                .wrap_anywhere(),
        );
    }
    col()
        .key("compiler-output")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .child(content)
}

pub(crate) fn view(state: &State, palette: Palette, scale: f32) -> El {
    let tabbed = state.ui.preview_dock == PreviewDock::Tab
        && state.preview.visible
        && preview::available(state);
    let preview_active = tabbed && state.workbench.preview_tab;
    let center = col()
        .key("editor-area")
        .grow(1.0)
        .h_full()
        .min_w(0.0)
        .min_h(0.0)
        .focus_within_ring(scale.recip(), -scale.recip(), palette.accent);
    let tabs = tabs(state, palette, scale, tabbed, preview_active);
    let editor = if preview_active {
        preview::view::pane(state, palette, scale.recip(), scale)
    } else {
        fast_editor::view(state, state.ui, state.editor_config, scale)
    };
    let center = match state.ui.tabs {
        TabPlacement::Top => center.child(tabs).child(editor),
        TabPlacement::Bottom => center.child(editor).child(tabs),
    };
    dock::body(state, center, palette, scale)
}

fn tabs(state: &State, palette: Palette, scale: f32, preview: bool, preview_active: bool) -> El {
    let height = row_height(state.ui.density);
    let mut tabs = row().key("tab-items").h(height).gap(1.0).center_y();
    for tab in state.files.tabs(&state.editor) {
        let id = tab.id;
        let name = tab
            .path
            .and_then(std::path::Path::file_name)
            .map_or_else(|| "Untitled".into(), |name| name.to_string_lossy());
        let active = state.files.active == id && !preview_active;
        let pinned = tab.path.is_some() && tab.path == state.language.pinned.as_deref();
        let mut adornment = String::new();
        if tab.dirty {
            adornment.push_str(" •");
        }
        if pinned {
            adornment.push_str(" [Main]");
        }
        let title = format!("{name}{adornment}");
        let badge_width = if pinned { 35.0 } else { 0.0 } + if tab.dirty { 12.0 } else { 0.0 };
        let width = (name.chars().count().min(26) as f32 * (state.ui.font_size * 0.61) + 16.0)
            .max(68.0)
            + badge_width;
        let help = tab
            .path
            .map_or_else(|| title.clone(), |path| path.display().to_string());
        let tab_key = format!("tab-{id}");
        let mut button = item(
            palette,
            &tab_key,
            &name,
            height,
            active,
            move |state, tx| {
                files::activate(state, id, tx);
            },
        )
        .w(width)
        .gap(4.0)
        .role("tab")
        .label(title)
        .checked(active)
        .help(help);
        if tab.dirty {
            button = button.child(
                text("•")
                    .font(palette.control_text(11.0))
                    .color(palette.text)
                    .w(8.0)
                    .a11y_hidden(),
            );
        }
        if pinned {
            button = button.child(
                text("Main")
                    .font(palette.control_text(10.0))
                    .color(palette.text)
                    .w(29.0)
                    .a11y_hidden(),
            );
        }
        let button = with_pin_menu(state, tab_key, tab.path, button);
        tabs = tabs.child(
            row()
                .key(format!("tab-group-{id}"))
                .h(height)
                .center_y()
                .fill(if active {
                    palette.selected
                } else {
                    palette.chrome
                })
                .border_sides([0.0, scale.recip(), 0.0, 0.0], palette.divider)
                .child(button)
                .child(
                    item(
                        palette,
                        format!("close-tab-{id}"),
                        "×",
                        height - 2.0,
                        active,
                        move |state, tx| {
                            files::close(state, id, tx);
                        },
                    )
                    .w(22.0)
                    .px(0.0)
                    .justify_center()
                    .label(format!("Close {name}")),
                ),
        );
    }
    if preview {
        tabs = tabs.child(
            item(
                palette,
                "preview-tab",
                "Preview",
                height,
                preview_active,
                |state, tx| {
                    state.workbench.preview_tab = true;
                    tx.focus("preview-tab", false);
                    tx.instant();
                },
            )
            .w(82.0)
            .role("tab")
            .checked(preview_active),
        );
    }
    row()
        .key("tab-bar")
        .role("tablist")
        .label("Open documents")
        .w_full()
        .h(height)
        .min_h(height)
        .min_w(0.0)
        .clip_rounded(0.0)
        .scroll_xy()
        .fill(palette.chrome)
        .border_sides(
            match state.ui.tabs {
                TabPlacement::Top => [0.0, 0.0, scale.recip(), 0.0],
                TabPlacement::Bottom => [scale.recip(), 0.0, 0.0, 0.0],
            },
            palette.divider,
        )
        .child(tabs)
}

pub(crate) fn overlays(state: &State, palette: Palette, scale: f32) -> impl Iterator<Item = El> {
    [
        dock::overlay(state, palette, scale),
        picker::overlay(state, palette, scale),
        pin_overlay(state, palette, scale),
    ]
    .into_iter()
    .flatten()
}

fn with_pin_menu(state: &State, anchor: String, path: Option<&Path>, button: El) -> El {
    let Some(path) = path.filter(|path| {
        state.language.ready()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("typ"))
    }) else {
        return button;
    };
    let path = path.to_owned();
    button.on_secondary_press_local(move |state: &mut State, _, tx| {
        state.workbench.pin_menu = Some((anchor.clone(), path.clone()));
        tx.focus("pin-tab-action", false);
        tx.instant();
    })
}

fn pin_overlay(state: &State, palette: Palette, scale: f32) -> Option<El> {
    let (anchor, path) = state.workbench.pin_menu.as_ref()?;
    let pinned = state.language.pinned.as_ref() == Some(path);
    let label = if pinned {
        "Unpin main file"
    } else {
        "Pin as main file"
    };
    let path = path.clone();
    Some(
        col()
            .key("pin-tab-menu")
            .w(152.0)
            .padding(3.0)
            .fill(palette.chrome)
            .border(scale.recip(), palette.divider)
            .role("menu")
            .anchored_fit(anchor.as_str(), rune::Side::Bottom, rune::Align::Start, 1.0)
            .modal_tx(true, |state: &mut State, tx| {
                state.workbench.pin_menu = None;
                tx.instant();
            })
            .child(item(
                palette,
                "pin-tab-action",
                label,
                24.0,
                false,
                move |state, tx| {
                    if pinned {
                        crate::language::invoke(state, crate::language::Command::Unpin, tx);
                    } else {
                        crate::language::pin(state, path.clone());
                    }
                    state.workbench.pin_menu = None;
                    tx.instant();
                },
            )),
    )
}

pub(crate) fn commands(state: &mut State, tx: &mut Tx) {
    picker::show(state, picker::Mode::Commands, tx);
}

pub(crate) fn quick_open(state: &mut State, tx: &mut Tx) {
    picker::show(state, picker::Mode::Files, tx);
}

pub(crate) fn shortcuts_available(state: &State) -> bool {
    state.files.confirm_close.is_none()
        && !state.files.comparing()
        && !state.language.workspace.showing()
        && !state.settings_menu
        && state.workbench.picker.mode.is_none()
}

fn item(
    palette: Palette,
    key: impl Into<String>,
    label: &str,
    height: f32,
    selected: bool,
    press: impl Fn(&mut State, &mut Tx) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .h(height)
        .min_h(height)
        .min_w(0.0)
        .center_y()
        .px(7.0)
        .fill(if selected {
            palette.selected
        } else {
            rune::Color::TRANSPARENT
        })
        .hover(if selected {
            palette.selected_hover
        } else {
            palette.hover
        })
        .focus_ring(1.0, -1.0, palette.accent)
        .on_press_tx(press)
        .child(
            text(label)
                .font(palette.ui_text((height - 10.0).clamp(11.0, 13.0)))
                .color(palette.text)
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
}

fn search_field(palette: Palette, edit: &TextEdit, key: &str, label: &str) -> El {
    text_input(edit, palette.ui_text(13.0))
        .key(key)
        .label(label)
        .placeholder(label, palette.muted_text)
        .single_line()
        .caret(palette.text, palette.selected)
        .color(palette.text)
        .fill(palette.canvas)
        .border(1.0, palette.divider)
        .focus_ring(1.0, -1.0, palette.accent)
        .h(27.0)
        .pt(4.0)
        .px(6.0)
        .grow(1.0)
        .min_w(0.0)
}
