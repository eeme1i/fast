use std::path::PathBuf;

use fast_theme::tok::Palette;
use rune::{Color, El, Key, Latest, ScrollSpec, TextEdit, Tx, Waker, col, row, text};

use super::{Tool, dock, item, project, search_field};
use crate::{State, files, language, preview};

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Commands,
    Files,
}

#[derive(Default)]
pub(super) struct Picker {
    pub mode: Option<Mode>,
    field: TextEdit,
    selected: usize,
    file_choices: Vec<Choice>,
    requested: Option<(u64, String)>,
    job: Latest<Vec<Choice>>,
}

#[derive(Clone, Copy)]
struct Action {
    name: &'static str,
    shortcut: &'static str,
    run: fn(&mut State, &mut Tx),
    available: fn(&State) -> bool,
}

const ALWAYS: fn(&State) -> bool = |_| true;
const LANGUAGE: fn(&State) -> bool = |state| state.language.ready() && preview::available(state);

const ACTIONS: &[Action] = &[
    Action {
        name: "New file",
        shortcut: "Ctrl+N",
        run: files::create,
        available: |state| !state.files.busy(),
    },
    Action {
        name: "Open file",
        shortcut: "Ctrl+O",
        run: files::open,
        available: ALWAYS,
    },
    Action {
        name: "Open project folder",
        shortcut: "",
        run: project::choose_root,
        available: ALWAYS,
    },
    Action {
        name: "Quick open file",
        shortcut: "Ctrl+P",
        run: super::quick_open,
        available: ALWAYS,
    },
    Action {
        name: "Find in Files",
        shortcut: "Ctrl+Shift+F",
        run: project::open_search,
        available: ALWAYS,
    },
    Action {
        name: "Find in document",
        shortcut: "Ctrl+F",
        run: |s, tx| s.editor.find(false, tx),
        available: ALWAYS,
    },
    Action {
        name: "Replace in document",
        shortcut: "Ctrl+H",
        run: |s, tx| s.editor.find(true, tx),
        available: ALWAYS,
    },
    Action {
        name: "Save file",
        shortcut: "Ctrl+S",
        run: |s, tx| files::save(s, false, tx),
        available: ALWAYS,
    },
    Action {
        name: "Save as",
        shortcut: "Ctrl+Shift+S",
        run: |s, tx| files::save(s, true, tx),
        available: ALWAYS,
    },
    Action {
        name: "Close tab",
        shortcut: "Ctrl+W",
        run: |s, tx| files::close(s, s.files.active, tx),
        available: ALWAYS,
    },
    Action {
        name: "Toggle Project files",
        shortcut: "Alt+1",
        run: |s, tx| dock::toggle(s, Tool::Project, tx),
        available: ALWAYS,
    },
    Action {
        name: "Toggle Outline",
        shortcut: "Alt+2",
        run: |s, tx| dock::toggle(s, Tool::Outline, tx),
        available: ALWAYS,
    },
    Action {
        name: "Toggle Problems",
        shortcut: "Alt+6",
        run: |s, tx| dock::toggle(s, Tool::Problems, tx),
        available: ALWAYS,
    },
    Action {
        name: "Toggle Compiler output",
        shortcut: "",
        run: |s, tx| dock::toggle(s, Tool::Compiler, tx),
        available: ALWAYS,
    },
    Action {
        name: "Toggle Preview",
        shortcut: "Alt+P",
        run: preview::toggle,
        available: preview::available,
    },
    Action {
        name: "Export document",
        shortcut: "",
        run: preview::export_dialog,
        available: preview::available,
    },
    Action {
        name: "Quick PDF export",
        shortcut: "Ctrl+Shift+E",
        run: preview::quick_export,
        available: preview::available,
    },
    Action {
        name: "Format document",
        shortcut: "",
        run: |s, tx| language::invoke(s, language::Command::Format, tx),
        available: LANGUAGE,
    },
    Action {
        name: "Go to definition",
        shortcut: "F12",
        run: |s, tx| language::invoke(s, language::Command::Definition, tx),
        available: LANGUAGE,
    },
    Action {
        name: "Find references",
        shortcut: "Shift+F12",
        run: |s, tx| language::invoke(s, language::Command::References, tx),
        available: LANGUAGE,
    },
    Action {
        name: "Rename symbol",
        shortcut: "F2",
        run: |s, tx| language::invoke(s, language::Command::Rename, tx),
        available: LANGUAGE,
    },
    Action {
        name: "Pin main file",
        shortcut: "",
        run: language::pin_dialog,
        available: LANGUAGE,
    },
    Action {
        name: "Unpin main file",
        shortcut: "",
        run: |s, tx| language::invoke(s, language::Command::Unpin, tx),
        available: |s| s.language.pinned.is_some(),
    },
    Action {
        name: "Settings",
        shortcut: "",
        run: super::settings::open,
        available: ALWAYS,
    },
];

#[derive(Clone)]
enum Choice {
    Command(Action),
    File(PathBuf, String),
}

impl Choice {
    fn label(&self) -> String {
        match self {
            Self::Command(action) if !action.shortcut.is_empty() => {
                format!("{}   {}", action.name, action.shortcut)
            }
            Self::Command(action) => action.name.into(),
            Self::File(_, display) => display.clone(),
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    class: u8,
    gap: usize,
    length: usize,
}

fn rank(query: &str, label: &str) -> Option<Rank> {
    if query.is_empty() {
        return Some(Rank {
            class: 0,
            gap: 0,
            length: 0,
        });
    }
    let label = label.to_lowercase();
    if let Some(at) = label.find(query) {
        return Some(Rank {
            class: if at == 0 { 0 } else { 1 },
            gap: at,
            length: label.len(),
        });
    }
    let mut chars = label.chars();
    let mut gap = 0;
    for ch in query.chars() {
        gap += chars.position(|candidate| candidate == ch)?;
    }
    Some(Rank {
        class: 2,
        gap,
        length: label.len(),
    })
}

fn choices(state: &State, mode: Mode) -> Vec<Choice> {
    match mode {
        Mode::Files => state.workbench.picker.file_choices.clone(),
        Mode::Commands => {
            let query = state.workbench.picker.field.text.trim().to_lowercase();
            ranked(
                ACTIONS
                    .iter()
                    .filter(|action| (action.available)(state))
                    .filter_map(|action| {
                        Some((rank(&query, action.name)?, Choice::Command(*action)))
                    }),
            )
        }
    }
}

fn ranked(choices: impl Iterator<Item = (Rank, Choice)>) -> Vec<Choice> {
    let mut matches: Vec<_> = choices.collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    matches
        .into_iter()
        .take(40)
        .map(|(_, choice)| choice)
        .collect()
}

impl Picker {
    pub(super) fn setup(&self, waker: Waker) {
        self.job.set_waker(waker);
    }
}

pub(super) fn poll(state: &mut State) -> bool {
    let project = &state.workbench.project;
    let picker = &mut state.workbench.picker;
    if !matches!(picker.mode, Some(Mode::Files)) {
        return false;
    }
    let (Some(root), Some(index)) = (&project.root, &project.index) else {
        picker.file_choices.clear();
        return false;
    };
    let query = picker.field.text.trim().to_lowercase();
    if picker
        .requested
        .as_ref()
        .is_none_or(|(version, requested)| *version != project.version || requested != &query)
    {
        picker.requested = Some((project.version, query.clone()));
        picker.file_choices.clear();
        picker.job.request(
            (root.clone(), index.clone(), query),
            |(root, index, query)| {
                ranked(index.files().filter_map(|path| {
                    let relative = path.to_string_lossy();
                    let name = path.file_name()?.to_string_lossy();
                    let rating = rank(&query, &name).or_else(|| {
                        let mut rating = rank(&query, &relative)?;
                        rating.class += 3;
                        Some(rating)
                    })?;
                    Some((rating, Choice::File(root.join(path), relative.into_owned())))
                }))
            },
        );
        return true;
    }
    if let Some(choices) = picker.job.take() {
        picker.file_choices = choices;
        picker.selected = picker
            .selected
            .min(picker.file_choices.len().saturating_sub(1));
        return true;
    }
    false
}

pub(super) fn show(state: &mut State, mode: Mode, tx: &mut Tx) {
    state.language.menu = false;
    state.language.popup = None;
    let picker = &mut state.workbench.picker;
    picker.mode = Some(mode);
    picker.field = TextEdit::default();
    picker.selected = 0;
    picker.file_choices.clear();
    picker.requested = None;
    picker.job.cancel();
    tx.focus("picker-input", true);
    tx.instant();
}

fn close(state: &mut State, tx: &mut Tx) {
    state.workbench.picker.mode = None;
    tx.focus(fast_editor::VIEWPORT, false);
    tx.instant();
}

fn choose(state: &mut State, index: usize, tx: &mut Tx) {
    let Some(mode) = state.workbench.picker.mode else {
        return;
    };
    let Some(choice) = choices(state, mode).into_iter().nth(index) else {
        return;
    };
    close(state, tx);
    match choice {
        Choice::Command(action) => (action.run)(state, tx),
        Choice::File(path, _) => files::open_path(state, path, tx),
    }
}

pub(super) fn overlay(state: &State, palette: Palette, scale: f32) -> Option<El> {
    let mode = state.workbench.picker.mode?;
    let title = match mode {
        Mode::Commands => "Command Palette",
        Mode::Files => "Quick Open",
    };
    let placeholder = match mode {
        Mode::Commands => "Type a command",
        Mode::Files => "Type a file name",
    };
    let options = choices(state, mode);
    let selected = state
        .workbench
        .picker
        .selected
        .min(options.len().saturating_sub(1));
    let height = (options.len().max(1) as f32 * 24.0).min(144.0);
    let query = search_field(
        palette,
        &state.workbench.picker.field,
        "picker-input",
        placeholder,
    )
    .on_edit(|state: &mut State| &mut state.workbench.picker.field)
    .on_input_change(|state: &mut State, tx| {
        state.workbench.picker.selected = 0;
        state.workbench.picker.file_choices.clear();
        tx.scroll_into_view_in("picker-scroll", "picker-result-0");
        tx.instant();
    })
    .on_input_key(move |state: &mut State, event, tx| match event.key {
        Key::Escape => {
            close(state, tx);
            true
        }
        Key::Up | Key::Down => {
            let count = choices(state, mode).len();
            if count > 0 {
                let current = state.workbench.picker.selected.min(count - 1);
                state.workbench.picker.selected = if event.key == Key::Up {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                };
                tx.scroll_into_view_in(
                    "picker-scroll",
                    format!("picker-result-{}", state.workbench.picker.selected),
                );
                tx.instant();
            }
            true
        }
        _ => false,
    })
    .on_submit(move |state: &mut State, tx| choose(state, selected, tx));
    let mut rows = col().w_full();
    if options.is_empty() {
        let message = match mode {
            Mode::Files if state.workbench.project.root.is_none() => {
                "Open a file or choose a project folder."
            }
            Mode::Files if state.workbench.project.index.is_none() => state
                .workbench
                .project
                .error
                .as_deref()
                .unwrap_or("Reading project files…"),
            Mode::Files if state.workbench.picker.job.is_pending() => "Matching files…",
            _ => "No matches",
        };
        rows = rows.child(project::message(palette, message));
    }
    for (i, choice) in options.iter().enumerate() {
        rows = rows.child(
            item(
                palette,
                format!("picker-result-{i}"),
                &choice.label(),
                24.0,
                i == selected,
                move |state, tx| choose(state, i, tx),
            )
            .w_full()
            .role("option")
            .checked(i == selected),
        );
    }
    Some(
        col()
            .key("picker-overlay")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .pt(30.0)
            .items_center()
            .fill(Color::BLACK.alpha(0.34))
            .role("dialog")
            .label(title)
            .modal_tx(true, close)
            .child(
                col()
                    .key("picker")
                    .w(410.0)
                    .padding(6.0)
                    .gap(4.0)
                    .fill(palette.chrome)
                    .border(scale.recip(), palette.divider)
                    .child(
                        text(title)
                            .font(palette.control_text(13.0))
                            .color(palette.text)
                            .px(2.0),
                    )
                    .child(row().child(query))
                    .child(
                        col()
                            .key("picker-scroll")
                            .w_full()
                            .h(height)
                            .min_h(0.0)
                            .scroll(ScrollSpec {
                                stick_bottom: false,
                            })
                            .child(rows),
                    ),
            ),
    )
}
