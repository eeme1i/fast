use std::path::PathBuf;

use fast_lsp::{Query, position, types::*};
use rune::{FileDialogOptions, Key, KeyEvent, TextEdit, Tx};

use super::{Popup, Results, at, completion, identifier, request, sync, workspace};
use crate::State;

#[derive(Clone, Copy)]
pub(crate) enum Command {
    Complete,
    Hover,
    Signature,
    Definition,
    References,
    Format,
    Rename,
    Outline,
    Problems,
    Actions,
    Fold,
    Unpin,
}

pub(crate) fn invoke(state: &mut State, command: Command, tx: &mut Tx) {
    state.language.menu = false;
    if matches!(command, Command::Outline | Command::Problems) {
        let tool = if matches!(command, Command::Outline) {
            crate::workbench::Tool::Outline
        } else {
            crate::workbench::Tool::Problems
        };
        state.workbench.open(tool);
        if tool == crate::workbench::Tool::Outline {
            request_outline(state);
        }
        tx.instant();
        return;
    }
    if !state.language.ready() {
        return;
    }
    sync(state);
    state.language.message = None;
    if matches!(command, Command::Unpin) {
        request(state, Query::pin(None), None);
        tx.instant();
        return;
    }
    let head = state.editor.document().selection().head;
    let Some(params) = at(state, head) else {
        return;
    };
    let document = params.text_document.clone();
    let query = match command {
        Command::Complete => {
            complete(state, None);
            return;
        }
        Command::Hover => {
            hover(state, head, false);
            return;
        }
        Command::Signature => Query::Signature(SignatureHelpParams {
            text_document_position_params: params,
            work_done_progress_params: Default::default(),
            context: None,
        }),
        Command::Definition => Query::Definition(GotoDefinitionParams {
            text_document_position_params: params,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }),
        Command::References => Query::References(ReferenceParams {
            text_document_position: params,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: ReferenceContext {
                include_declaration: true,
            },
        }),
        Command::Format => Query::Format(DocumentFormattingParams {
            text_document: document,
            options: FormattingOptions {
                tab_size: 2,
                insert_spaces: true,
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
        }),
        Command::Rename => {
            state.language.rename = Some(TextEdit::default());
            tx.focus("rename-symbol", true);
            tx.instant();
            return;
        }
        Command::Outline | Command::Problems => return,
        Command::Actions => {
            let selection = state.editor.document().selection().range();
            let start =
                position(state.editor.document(), selection.start).unwrap_or(params.position);
            let end = position(state.editor.document(), selection.end).unwrap_or(params.position);
            let diagnostics = state
                .language
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.range.start <= end && diagnostic.range.end >= start)
                .cloned()
                .collect();
            Query::Actions(CodeActionParams {
                text_document: document,
                range: Range { start, end },
                context: CodeActionContext {
                    diagnostics,
                    only: None,
                    trigger_kind: Some(CodeActionTriggerKind::INVOKED),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
        }
        Command::Fold => {
            if !state
                .editor
                .toggle_fold(state.editor.document().line_at(head), tx)
            {
                state.language.message = Some("No folding range starts on this line".into());
            }
            return;
        }
        Command::Unpin => return,
    };
    request(state, query, None);
    tx.instant();
}

pub(crate) fn rename(state: &mut State, tx: &mut Tx) {
    let Some(field) = state.language.rename.take() else {
        return;
    };
    let new_name = field.text.trim();
    if !new_name.is_empty() {
        sync(state);
        if let Some(position) = at(state, state.editor.document().selection().head) {
            request(
                state,
                Query::Rename(RenameParams {
                    text_document_position: position,
                    new_name: new_name.to_owned(),
                    work_done_progress_params: Default::default(),
                }),
                None,
            );
        }
    }
    tx.focus(fast_editor::VIEWPORT, false);
    tx.instant();
}

pub(crate) fn complete(state: &mut State, trigger: Option<String>) {
    let Some(params) = at(state, state.editor.document().selection().head) else {
        return;
    };
    let trigger_character = trigger
        .filter(|value| ["#", "(", "<", ",", ".", ":", "/", "\"", "@"].contains(&value.as_str()));
    let context = CompletionContext {
        trigger_kind: if trigger_character.is_some() {
            CompletionTriggerKind::TRIGGER_CHARACTER
        } else {
            CompletionTriggerKind::INVOKED
        },
        trigger_character,
    };
    request(
        state,
        Query::Completion(CompletionParams {
            text_document_position: params,
            context: Some(context),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }),
        None,
    );
}

pub(crate) fn hover(state: &mut State, byte: usize, pointer: bool) {
    let Some(position) = at(state, byte) else {
        return;
    };
    request(
        state,
        Query::Hover(HoverParams {
            text_document_position_params: position,
            work_done_progress_params: Default::default(),
        }),
        pointer.then_some(byte),
    );
}

pub(crate) fn refresh(state: &mut State) {
    if !state.language.ready() {
        return;
    }
    let Some(document) = identifier(state) else {
        return;
    };
    if state.language.legend.is_some() {
        request(
            state,
            Query::Semantic(SemanticTokensParams {
                text_document: document.clone(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            }),
            None,
        );
    }
    request(
        state,
        Query::Folding(FoldingRangeParams {
            text_document: document.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }),
        None,
    );
    let editor = state.editor.document();
    let visible = state.editor.visible_lines();
    let start = editor.line_range(visible.start.saturating_sub(80)).start;
    let end = editor.line_range(visible.end.saturating_add(80)).end;
    if let (Some(start), Some(end)) = (position(editor, start), position(editor, end)) {
        request(
            state,
            Query::Inlay(InlayHintParams {
                text_document: document.clone(),
                range: Range::new(start, end),
                work_done_progress_params: Default::default(),
            }),
            None,
        );
    }
    if state.workbench.is_open(crate::workbench::Tool::Outline) {
        outline(state, document);
    }
}

pub(crate) fn request_outline(state: &mut State) {
    if let Some(document) = identifier(state) {
        outline(state, document);
    }
}

fn outline(state: &mut State, document: TextDocumentIdentifier) {
    request(
        state,
        Query::Outline(DocumentSymbolParams {
            text_document: document,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }),
        None,
    );
}

pub(crate) fn pin_dialog(state: &mut State, tx: &mut Tx) {
    state.language.menu = false;
    if !state.language.ready() {
        return;
    }
    let path = state.files.path.as_deref();
    let options = FileDialogOptions {
        title: "Choose the main Typst file".into(),
        directory: state
            .language
            .connection
            .as_ref()
            .map(|conn| conn.options.root.clone()),
        file_name: path
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        ..Default::default()
    }
    .filter("Typst documents", &["typ"]);
    tx.open_file(options, |state: &mut State, result, tx| {
        match result {
            Ok(Some(path)) => pin(state, path),
            Ok(None) => {}
            Err(error) => state.language.message = Some(error.to_string()),
        }
        tx.focus(fast_editor::VIEWPORT, false);
        tx.instant();
    });
}

pub(crate) fn pin(state: &mut State, path: PathBuf) {
    if let Some(path) = path.to_str() {
        sync(state);
        request(state, Query::pin(Some(path.to_owned())), None);
    } else {
        state.language.message = Some("The main file path is not valid UTF-8".into());
    }
}

pub(crate) fn apply_action(state: &mut State, index: usize, tx: &mut Tx) {
    sync(state);
    let Some(Results::Actions(actions)) = &state.language.result else {
        return;
    };
    let Some(action) = actions.get(index).cloned() else {
        return;
    };
    match action {
        CodeActionOrCommand::Command(command) => workspace::execute(state, Some(command)),
        CodeActionOrCommand::CodeAction(action) => {
            if action.disabled.is_some() {
                return;
            }
            if let Some(edit) = action.edit {
                workspace::apply(state, edit, action.command, tx);
            } else {
                workspace::execute(state, action.command);
            }
        }
    }
    state.language.result = None;
    tx.focus(fast_editor::VIEWPORT, false);
    tx.instant();
}

pub(crate) fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if state.language.workspace.showing() {
        return false;
    }
    if let Some(Popup::Completion {
        items, selected, ..
    }) = &mut state.language.popup
    {
        match event.key {
            Key::Up | Key::Down => {
                *selected = if event.key == Key::Up {
                    (*selected + items.len() - 1) % items.len()
                } else {
                    (*selected + 1) % items.len()
                };
                tx.scroll_into_view_in("completion-list", format!("completion-{selected}"));
                tx.instant();
                return true;
            }
            Key::Enter | Key::Tab => {
                let index = *selected;
                completion::accept(state, index, tx);
                return true;
            }
            Key::Escape => {
                state.language.popup = None;
                tx.instant();
                return true;
            }
            _ => state.language.popup = None,
        }
    }
    if !state.language.ready() {
        return false;
    }
    let command = match &event.key {
        Key::Command(' ') => Command::Complete,
        Key::Text(value) if value == " " && event.mods.ctrl => Command::Complete,
        Key::Function(1) => Command::Hover,
        Key::Function(2) => Command::Rename,
        Key::Function(12) if event.mods.shift => Command::References,
        Key::Function(12) => Command::Definition,
        Key::Command('.') => Command::Actions,
        Key::Escape if state.language.popup.is_some() || state.language.menu => {
            state.language.popup = None;
            state.language.menu = false;
            state.language.hover = None;
            tx.instant();
            return true;
        }
        _ => return false,
    };
    invoke(state, command, tx);
    true
}
