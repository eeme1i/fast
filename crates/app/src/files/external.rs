use std::{collections::BTreeMap, io, path::PathBuf, sync::Arc, time::Duration};

use fast_core::{document::Document, file};
use fast_editor::VIEWPORT;
use fast_theme::tok::Palette;
use rune::{El, GrowthSpec, Latest, Tx, Waker, col, row, text};

use crate::{State, view::action};

const INTERVAL: Duration = Duration::from_secs(1);
type Watched = (u64, PathBuf, Option<Arc<Document>>);
type Conflicts = BTreeMap<u64, Conflict>;

#[derive(Default)]
pub(super) struct Watch {
    generation: u64,
    next: f64,
    scan: Latest<(u64, Conflicts)>,
    conflicts: Conflicts,
    comparing: bool,
}

#[derive(Clone)]
enum Conflict {
    Changed(Arc<Document>),
    Missing,
    Unreadable(String),
}

impl Watch {
    pub fn setup(&self, waker: Waker) {
        self.scan.set_waker(waker);
    }

    pub fn invalidate(&mut self, tab: u64) {
        self.generation += 1;
        self.next = 0.0;
        self.conflicts.remove(&tab);
        self.comparing = false;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn rescan(&mut self) {
        self.next = 0.0;
    }

    pub fn comparing(&self) -> bool {
        self.comparing
    }
}

fn scan(files: Vec<Watched>, previous: Conflicts) -> Conflicts {
    files
        .into_iter()
        .filter_map(|(tab, path, expected)| {
            let change = match file::read_text(&path) {
                Ok(text) if expected.as_ref().is_some_and(|doc| doc.snapshot() == text) => {
                    return None;
                }
                Ok(text) => {
                    if let Some(Conflict::Changed(old)) = previous.get(&tab)
                        && old.snapshot() == text
                    {
                        Conflict::Changed(Arc::clone(old))
                    } else {
                        Conflict::Changed(Arc::new(Document::from_rope(text)))
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    expected?;
                    Conflict::Missing
                }
                Err(error) => Conflict::Unreadable(error.to_string()),
            };
            Some((tab, change))
        })
        .collect()
}

pub(super) fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let watch = &mut state.files.external;
    let mut changed = false;
    if let Some((generation, conflicts)) = watch.scan.take()
        && generation == watch.generation
    {
        changed = !same_conflicts(&watch.conflicts, &conflicts);
        watch.conflicts = conflicts;
        watch.comparing &= matches!(
            watch.conflicts.get(&state.files.active),
            Some(Conflict::Changed(_))
        );
    }
    if now >= watch.next && !watch.scan.is_pending() {
        let files = state
            .files
            .path
            .iter()
            .map(|path| (state.files.active, path.clone(), state.files.disk.clone()))
            .chain(
                state
                    .files
                    .parked
                    .iter()
                    .filter_map(|(&id, tab)| Some((id, tab.path.clone()?, tab.disk.clone()))),
            )
            .collect::<Vec<_>>();
        if !files.is_empty() {
            watch.scan.request(
                (watch.generation, files, watch.conflicts.clone()),
                |(generation, files, previous)| (generation, scan(files, previous)),
            );
            watch.next = now + INTERVAL.as_secs_f64();
            tx.wake_after(INTERVAL);
        }
    } else if now < watch.next && changed {
        tx.wake_after(Duration::from_secs_f64(watch.next - now));
    }
    if changed {
        tx.instant();
    }
    changed
}

fn same_conflicts(a: &Conflicts, b: &Conflicts) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(tab, conflict)| match (conflict, b.get(tab)) {
                (Conflict::Missing, Some(Conflict::Missing)) => true,
                (Conflict::Unreadable(a), Some(Conflict::Unreadable(b))) => a == b,
                (Conflict::Changed(a), Some(Conflict::Changed(b))) => Arc::ptr_eq(a, b),
                _ => false,
            })
}

fn resolve(state: &mut State, reload: bool, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    let id = state.files.active;
    let disk = match state.files.external.conflicts.get(&id) {
        Some(Conflict::Changed(disk)) => Some(disk.clone()),
        Some(Conflict::Missing) if !reload => None,
        _ => return,
    };
    if reload && let Some(disk) = &disk {
        state.editor.reload(disk, tx);
    }
    // Keep the editor contents as edits against the newly acknowledged disk version.
    state.files.saved_revision = (reload
        || disk
            .as_ref()
            .is_some_and(|disk| disk.snapshot() == state.editor.document().snapshot()))
    .then(|| state.editor.document().revision());
    state.files.disk = disk;
    state.files.external.invalidate(id);
    state.files.message = Some(
        if reload {
            "Reloaded from disk"
        } else {
            "Kept editor version"
        }
        .into(),
    );
    state.workbench.project.refresh();
    tx.focus(VIEWPORT, false);
    tx.instant();
}

pub(crate) fn banner(state: &State, palette: Palette, hairline: f32) -> Option<El> {
    let change = state.files.external.conflicts.get(&state.files.active)?;
    let (label, detail) = match change {
        Conflict::Changed(_) if state.files.dirty(&state.editor) => {
            ("File changed on disk. This tab has unsaved edits.", None)
        }
        Conflict::Changed(_) => ("File changed on disk.", None),
        Conflict::Missing => ("File was removed from disk.", None),
        Conflict::Unreadable(error) => ("Couldn’t read the file on disk.", Some(error.as_str())),
    };
    let size = state.ui.font_size;
    let enabled = !state.files.busy() && !state.language.workspace.showing();
    let mut message = text(label)
        .font(palette.ui_text(size))
        .color(palette.text)
        .shrink()
        .truncate();
    if let Some(detail) = detail {
        message = message.help(detail);
    }
    let mut bar = row()
        .key("external-change")
        .w_full()
        .h(29.0)
        .min_h(29.0)
        .center_y()
        .px(6.0)
        .gap(5.0)
        .fill(palette.chrome)
        .border_sides([0.0, 0.0, hairline, 0.0], palette.divider)
        .child(message);
    if matches!(change, Conflict::Changed(_)) {
        bar = bar
            .child(action(
                "compare-external",
                "Compare",
                palette,
                size,
                enabled,
                |state, tx| {
                    state.files.external.comparing = true;
                    tx.focus("close-comparison", false);
                    tx.instant();
                },
            ))
            .child(action(
                "reload-external",
                "Reload",
                palette,
                size,
                enabled,
                |state, tx| resolve(state, true, tx),
            ));
    }
    if !matches!(change, Conflict::Unreadable(_)) {
        bar = bar.child(action(
            "keep-external",
            "Keep editor",
            palette,
            size,
            enabled,
            |state, tx| resolve(state, false, tx),
        ));
    }
    Some(bar)
}

pub(crate) fn overlay(state: &State, palette: Palette) -> Option<El> {
    if !state.files.external.comparing {
        return None;
    }
    let Conflict::Changed(disk) = state.files.external.conflicts.get(&state.files.active)? else {
        return None;
    };
    let dismiss = |state: &mut State, tx: &mut Tx| {
        state.files.external.comparing = false;
        tx.focus(VIEWPORT, false);
        tx.instant();
    };
    let size = state.ui.font_size;
    let columns = row()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(6.0)
        .child(source(
            palette,
            "Editor",
            false,
            state.editor.document().line_count(),
        ))
        .child(source(palette, "On disk", true, disk.line_count()));
    Some(
        col()
            .key("external-comparison")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(rune::Color::BLACK.alpha(0.42))
            .role("dialog")
            .label("Compare external change")
            .modal_tx(false, dismiss)
            .child(
                col()
                    .w((state.workbench.width() - 36.0).clamp(400.0, 920.0))
                    .h_full()
                    .max_h(274.0)
                    .padding(8.0)
                    .gap(7.0)
                    .fill(palette.chrome)
                    .border(1.0, palette.divider)
                    .child(columns)
                    .child(
                        row()
                            .gap(5.0)
                            .child(action(
                                "close-comparison",
                                "Close",
                                palette,
                                size,
                                true,
                                dismiss,
                            ))
                            .child(action(
                                "reload-compared",
                                "Reload",
                                palette,
                                size,
                                true,
                                |state, tx| resolve(state, true, tx),
                            ))
                            .child(action(
                                "keep-compared",
                                "Keep editor",
                                palette,
                                size,
                                true,
                                |state, tx| resolve(state, false, tx),
                            )),
                    ),
            ),
    )
}

fn source(palette: Palette, title: &str, disk: bool, lines: usize) -> El {
    col()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .child(text(title).font(palette.ui_text(13.0)).color(palette.text))
        .child(
            col()
                .key(if disk {
                    "disk-version"
                } else {
                    "editor-version"
                })
                .grow(1.0)
                .min_h(0.0)
                .min_w(0.0)
                .w_full()
                .scroll_xy()
                .fill(palette.canvas)
                .border(1.0, palette.divider)
                .child(col().growth(GrowthSpec::default()).virtual_rows(
                    lines,
                    19.0,
                    |_: &State, i| format!("comparison-line-{i}").into(),
                    move |state: &State, i| {
                        let doc = if disk {
                            match state.files.external.conflicts.get(&state.files.active) {
                                Some(Conflict::Changed(doc)) => doc,
                                _ => return row().h(19.0),
                            }
                        } else {
                            state.editor.document()
                        };
                        let code = doc.slice(doc.line_range(i));
                        row()
                            .h(19.0)
                            .center_y()
                            .gap(6.0)
                            .px(5.0)
                            .child(
                                text(format!("{}", i + 1))
                                    .font(palette.ui_text(11.0).mono())
                                    .color(palette.muted_text),
                            )
                            .child(
                                text(code)
                                    .font(palette.ui_text(12.0).mono())
                                    .color(palette.text),
                            )
                    },
                )),
        )
}
