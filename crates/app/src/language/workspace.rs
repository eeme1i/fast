use std::path::PathBuf;

use fast_core::{document::Document, file};
use fast_lsp::{
    Query,
    types::{Command, ExecuteCommandParams, FileChangeType, FileEvent, Url, WorkspaceEdit},
    workspace::{Applied, WorkspacePlan, text_edits},
};
use fast_theme::tok::Palette;
use rune::{Color, El, Latest, ScrollSpec, Tx, Waker, col, row, text};

use super::{actions, request};
use crate::{State, files, view::action};

#[derive(Default)]
pub(crate) struct Workspace {
    pending: Option<Proposal>,
    in_flight: bool,
    after: Option<Command>,
    job: Latest<Result<Outcome, String>>,
}

struct Outcome {
    applied: Applied,
    tabs: Vec<Result<(PathBuf, PathBuf, fast_editor::Editor), String>>,
}

struct Proposal {
    plan: WorkspacePlan,
    root: PathBuf,
    path: PathBuf,
    revision: u64,
    session: u64,
    after: Option<Command>,
}

impl Workspace {
    pub fn setup(&self, waker: Waker) {
        self.job.set_waker(waker);
    }

    pub fn showing(&self) -> bool {
        self.pending.is_some() || self.in_flight
    }
}

pub(super) fn apply(state: &mut State, edit: WorkspaceEdit, after: Option<Command>, tx: &mut Tx) {
    let Some(conn) = &state.language.connection else {
        return;
    };
    let Some(binding) = &conn.binding else { return };
    let Some(path) = &state.files.path else {
        return;
    };
    let plan = match WorkspacePlan::from_edit(edit, &binding.uri, conn.version) {
        Ok(plan) => plan,
        Err(error) => {
            state.language.message = Some(error);
            return;
        }
    };
    if let Some(edits) = plan.local_edits(path) {
        match text_edits(state.editor.document(), edits) {
            Some(edits) if edits.is_empty() => execute(state, after),
            Some(edits) => {
                if state.editor.apply_language_edits(edits, tx) {
                    execute(state, after);
                }
            }
            None => {
                state.language.message =
                    Some("Tinymist supplied overlapping or invalid edits".into())
            }
        }
        return;
    }
    if state.files.dirty(&state.editor) || state.files.busy() {
        state.language.message =
            Some("Save the current file before applying workspace edits".into());
        return;
    }
    let paths = plan.paths();
    if let Some((path, _)) = state
        .files
        .dirty_buffers()
        .find(|(path, _)| paths.contains(path))
    {
        state.language.message = Some(format!(
            "Save {} before applying workspace edits",
            path.display()
        ));
        return;
    }
    state.language.workspace.pending = Some(Proposal {
        plan,
        root: conn.options.root.clone(),
        path: path.clone(),
        revision: state.editor.document().revision(),
        session: state.files.session,
        after,
    });
    state.language.popup = None;
    tx.instant();
}

fn confirm(state: &mut State, tx: &mut Tx) {
    let Some(proposal) = state.language.workspace.pending.take() else {
        return;
    };
    if proposal.session != state.files.session
        || proposal.revision != state.editor.document().revision()
    {
        state.language.message =
            Some("The document changed. Run the language action again.".into());
        return;
    }
    state.language.workspace.in_flight = true;
    state.language.workspace.after = proposal.after;
    let paths = proposal.plan.paths();
    let tabs = state
        .files
        .other_buffers()
        .filter(|(path, _)| paths.contains(path))
        .map(|(path, document)| (path.to_owned(), document.snapshot()))
        .collect::<Vec<_>>();
    state.language.workspace.job.request(
        (
            proposal.plan,
            proposal.root,
            proposal.path,
            Document::from_rope(state.editor.document().snapshot()),
            tabs,
        ),
        |(plan, root, path, doc, tabs)| {
            for (path, expected) in &tabs {
                let disk = file::read_text(path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                if &disk != expected {
                    return Err(format!(
                        "{} changed on disk. Review it before applying workspace edits.",
                        path.display()
                    ));
                }
            }
            let applied = plan.apply(&root, &path, &doc)?;
            let tabs = tabs
                .into_iter()
                .map(|(previous, _)| {
                    let path = applied
                        .moved
                        .as_ref()
                        .filter(|(from, _)| *from == previous)
                        .map_or_else(|| previous.clone(), |(_, to)| to.clone());
                    files::load(path).map(|(path, editor)| (previous, path, editor))
                })
                .collect();
            Ok(Outcome { applied, tabs })
        },
    );
    tx.instant();
}

fn dismiss(state: &mut State, tx: &mut Tx) {
    if !state.language.workspace.in_flight {
        state.language.workspace.pending = None;
        tx.focus(fast_editor::VIEWPORT, false);
        tx.instant();
    }
}

pub(crate) fn poll(state: &mut State, tx: &mut Tx) -> bool {
    let Some(result) = state.language.workspace.job.take() else {
        return false;
    };
    state.language.workspace.in_flight = false;
    let command = state.language.workspace.after.take();
    match result {
        Ok(Outcome { applied, tabs }) => {
            if !applied.edits.is_empty() {
                state.editor.apply_language_edits(applied.edits, tx);
            }
            state.files.workspace_saved(applied.path, &state.editor);
            let mut refreshed = Vec::new();
            for tab in tabs {
                match tab {
                    Ok(tab) => {
                        if let Some(conn) = &mut state.language.connection
                            && let Ok(uri) = Url::from_file_path(&tab.0)
                            && conn.documents.remove(&uri).is_some()
                        {
                            conn.client.close(uri.clone());
                            conn.diagnostics.remove(&uri);
                            state
                                .language
                                .problems
                                .retain(|entry| entry.location.uri != uri);
                        }
                        refreshed.push(tab)
                    }
                    Err(error) => state.files.message = Some(error),
                }
            }
            state.files.refresh_tabs(refreshed, state.waker.as_ref());
            state.workbench.project.refresh();
            let change = |path: PathBuf, typ| {
                Url::from_file_path(path)
                    .ok()
                    .map(|uri| FileEvent::new(uri, typ))
            };
            let mut events: Vec<_> = applied
                .updated
                .into_iter()
                .filter_map(|path| change(path, FileChangeType::CHANGED))
                .collect();
            if let Some((from, to)) = applied.moved {
                events.extend(change(from.clone(), FileChangeType::DELETED));
                events.extend(change(to.clone(), FileChangeType::CREATED));
                if state.language.pinned.as_ref() == Some(&from) {
                    actions::pin(state, to);
                }
            }
            if let Some(conn) = &state.language.connection {
                conn.client.files_changed(events);
            }
            state.language.saved();
            execute(state, command);
        }
        Err(error) => state.language.message = Some(error),
    }
    tx.focus(fast_editor::VIEWPORT, false);
    tx.instant();
    true
}

pub(super) fn execute(state: &mut State, command: Option<Command>) {
    if let Some(command) = command {
        super::sync(state);
        request(
            state,
            Query::Execute(ExecuteCommandParams {
                command: command.command,
                arguments: command.arguments.unwrap_or_default(),
                work_done_progress_params: Default::default(),
            }),
            None,
        );
    }
}

pub(crate) fn overlay(state: &State, palette: Palette) -> Option<El> {
    if !state.language.workspace.showing() {
        return None;
    }
    let mut card = col()
        .w(380.0)
        .padding(10.0)
        .gap(6.0)
        .fill(palette.chrome)
        .border(1.0, palette.divider)
        .child(
            text("Workspace edits")
                .font(palette.control_text(13.0))
                .color(palette.text),
        );
    if let Some(proposal) = &state.language.workspace.pending {
        let paths = proposal.plan.paths();
        let mut items = col().w_full();
        for path in &paths {
            let display = path
                .strip_prefix(&proposal.root)
                .unwrap_or(path)
                .display()
                .to_string();
            items = items.child(
                text(&display)
                    .help(&display)
                    .font(palette.ui_text(12.0))
                    .color(palette.muted_text)
                    .w_full()
                    .truncate(),
            );
        }
        card = card
            .child(
                text("Apply and save changes to these paths?")
                    .font(palette.ui_text(12.0))
                    .color(palette.text),
            )
            .child(
                col()
                    .h(paths.len().min(4) as f32 * 17.0)
                    .w_full()
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(items),
            )
            .child(
                row()
                    .gap(5.0)
                    .child(
                        action(
                            "cancel-workspace-edit",
                            "Cancel",
                            palette,
                            12.0,
                            true,
                            dismiss,
                        )
                        .autofocus(),
                    )
                    .child(action(
                        "confirm-workspace-edit",
                        "Apply and save",
                        palette,
                        12.0,
                        true,
                        confirm,
                    )),
            );
    } else {
        card = card.child(
            text("Saving workspace edits…")
                .font(palette.ui_text(12.0))
                .color(palette.text),
        );
    }
    Some(
        col()
            .key("workspace-edit")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .role("dialog")
            .label("Workspace edits")
            .fill(Color::BLACK.alpha(0.4))
            .modal_tx(false, dismiss)
            .child(card),
    )
}
