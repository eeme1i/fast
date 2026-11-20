use fast_editor::{HoverTarget, InlineHint, Problem, SemanticKind, SemanticMark};
use fast_lsp::{Event, Kind, Reply, Status, range, types::*, workspace::text_edits};
use rune::Tx;

use super::{Entry, Popup, Results, Waiting, completion, workspace};
use crate::{State, files};

pub(super) fn handle(state: &mut State, event: Event, tx: &mut Tx) -> bool {
    match event {
        Event::Status(status) => {
            if matches!(status, Status::Ready { .. }) {
                state.language.refresh = Some(0.0);
                if state
                    .language
                    .connection
                    .as_mut()
                    .is_some_and(|connection| std::mem::take(&mut connection.restore_pin))
                    && let Some(path) = state.language.pinned.clone()
                {
                    super::actions::pin(state, path);
                }
            } else {
                state.language.popup = None;
                state.language.compile = None;
                if let Some(connection) = &mut state.language.connection {
                    connection.pending.clear();
                }
            }
            state.language.status = Some(status);
        }
        Event::Compile(status) => {
            if status.path.as_os_str().is_empty() {
                return false;
            }
            let message = match status.status {
                fast_lsp::Compilation::Compiling => "Compiling".to_owned(),
                fast_lsp::Compilation::CompileSuccess => {
                    format!("Compiled {} pages", status.page_count)
                }
                fast_lsp::Compilation::CompileError => "Compilation failed".to_owned(),
            };
            crate::workbench::log(state, format!("{}  {message}", status.path.display()));
            state.language.compile = Some(status);
        }
        Event::Pinned(path) => {
            let document = path.as_ref().or(state.files.path.as_ref());
            if state.language.compile.as_ref().map(|status| &status.path) != document {
                state.language.compile = None;
            }
            state.language.pinned = path;
        }
        Event::Legend(legend) => state.language.legend = Some(legend),
        Event::Notice(message) => state.language.message = Some(message),
        Event::Diagnostics(params) => {
            let Some(connection) = &mut state.language.connection else {
                return false;
            };
            let active = connection
                .binding
                .as_ref()
                .is_some_and(|binding| binding.uri == params.uri);
            if let Some(open) = connection.documents.get(&params.uri)
                && params
                    .version
                    .is_some_and(|version| version != open.version)
            {
                return false;
            }
            connection
                .diagnostics
                .insert(params.uri.clone(), params.diagnostics.clone());
            state
                .language
                .problems
                .retain(|entry| entry.location.uri != params.uri);
            state
                .language
                .problems
                .extend(params.diagnostics.iter().map(|diagnostic| Entry {
                    label: format!(
                        "{}  {}",
                        if is_error(diagnostic) {
                            "Error"
                        } else {
                            "Warning"
                        },
                        diagnostic.message
                    ),
                    detail: location_label(&params.uri, diagnostic.range.start),
                    location: Location::new(params.uri.clone(), diagnostic.range),
                }));
            if active {
                show_diagnostics(state, params.diagnostics);
            }
        }
        Event::Response { id, reply } => {
            let Some(connection) = &mut state.language.connection else {
                return false;
            };
            let Some(waiting) = connection.pending.remove(&id) else {
                return false;
            };
            if waiting.kind == Kind::Export {
                let result = match reply {
                    Ok(Reply::Export(data)) => Ok(data.into_bytes()),
                    Ok(_) => Err("Unexpected Tinymist export response".into()),
                    Err(error) => Err(error),
                };
                crate::preview::text_reply(state, result);
                return true;
            }
            if waiting.kind != Kind::Execute && connection.version != waiting.version {
                return false;
            }
            match reply {
                Ok(reply) => return response(state, waiting, reply, tx),
                Err(error)
                    if matches!(
                        waiting.kind,
                        Kind::Completion
                            | Kind::Hover
                            | Kind::Signature
                            | Kind::Semantic
                            | Kind::Folding
                            | Kind::Inlay
                    ) =>
                {
                    // Automatic requests can be superseded by a newer editor version.
                    if !error.contains("cancel") && !error.contains("Content modified") {
                        state.language.message = Some(error);
                    }
                    return true;
                }
                Err(error) => state.language.message = Some(error),
            }
        }
    }
    true
}

pub(super) fn show_diagnostics(state: &mut State, diagnostics: Vec<Diagnostic>) {
    let document = state.editor.document();
    let marks: Vec<_> = diagnostics
        .iter()
        .filter_map(|diagnostic| {
            Some(Problem {
                range: range(document, diagnostic.range)?,
                error: is_error(diagnostic),
                message: diagnostic.message.clone(),
            })
        })
        .collect();
    if state.editor.problems() != marks.as_slice() {
        if matches!(
            state.language.shown_hover(),
            Some(HoverTarget::Diagnostics { .. })
        ) {
            state.language.popup = None;
            state.language.hover_dismiss = None;
            state.language.hover_card_bounds = None;
            state.language.hover_card_inside = false;
        }
        if state
            .language
            .hover
            .as_ref()
            .is_some_and(|hover| matches!(hover.target, HoverTarget::Diagnostics { .. }))
        {
            state.language.hover = None;
        }
        state.editor.set_problems(marks);
    }
    state.language.diagnostics = diagnostics;
}

fn response(state: &mut State, waiting: Waiting, reply: Reply, tx: &mut Tx) -> bool {
    let caret = state.editor.document().selection().head;
    match reply {
        Reply::Completion(items) => {
            if caret != waiting.caret || tx.focused() != Some(fast_editor::VIEWPORT) {
                return false;
            }
            let items = match items {
                Some(CompletionResponse::List(list)) => list.items,
                Some(CompletionResponse::Array(items)) => items,
                None => return false,
            };
            let document = state.editor.document();
            let items = completion::filter(document, caret, items);
            if items.is_empty() {
                return false;
            }
            state.language.popup = Some(Popup::Completion {
                items,
                selected: 0,
                version: waiting.version,
                caret,
            });
        }
        Reply::Hover(Some(hover)) => {
            if waiting.hover.is_some_and(|byte| {
                !state.language.pointer_hover_allowed()
                    || state.language.hover.as_ref().is_none_or(|hover| {
                        hover.target != HoverTarget::Symbol(byte) || !hover.sent
                    })
            }) || waiting.hover.is_none() && caret != waiting.caret
            {
                return false;
            }
            let body = match hover.contents {
                HoverContents::Scalar(value) => marked(value),
                HoverContents::Array(values) => values
                    .into_iter()
                    .map(marked)
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                HoverContents::Markup(content) => content.value,
            };
            card(state, "Hover", body, waiting.hover);
        }
        Reply::Signature(Some(signatures)) if caret == waiting.caret => {
            let index = signatures.active_signature.unwrap_or(0) as usize;
            if let Some(signature) = signatures.signatures.get(index) {
                let active = signature.active_parameter.or(signatures.active_parameter);
                let mut body = signature.label.clone();
                if let Some(parameter) =
                    active.and_then(|at| signature.parameters.as_ref()?.get(at as usize))
                {
                    let label = match &parameter.label {
                        ParameterLabel::Simple(label) => label.as_str(),
                        ParameterLabel::LabelOffsets([start, end]) => signature
                            .label
                            .get(*start as usize..*end as usize)
                            .unwrap_or_default(),
                    };
                    if !label.is_empty() {
                        body.push_str(&format!("\nActive parameter  {label}"));
                    }
                    if let Some(doc) = &parameter.documentation {
                        body.push_str(&format!("\n{}", documentation(doc)));
                    }
                }
                card(state, "Signature", body, None);
            }
        }
        Reply::Definition(Some(definition)) => {
            let locations = match definition {
                GotoDefinitionResponse::Scalar(location) => vec![location],
                GotoDefinitionResponse::Array(locations) => locations,
                GotoDefinitionResponse::Link(links) => links
                    .into_iter()
                    .map(|link| Location::new(link.target_uri, link.target_selection_range))
                    .collect(),
            };
            if let [location] = locations.as_slice() {
                go(state, location.clone(), tx);
            } else {
                show_locations(state, "Definitions", locations);
            }
        }
        Reply::Definition(None) => state.language.message = Some("No definition found".into()),
        Reply::References(locations) => {
            show_locations(state, "References", locations.unwrap_or_default())
        }
        Reply::Format(edits) => {
            if let Some(edits) = edits
                && !edits.is_empty()
            {
                let uri = state.language.binding().map(|binding| binding.uri.clone());
                if let Some(uri) = uri {
                    apply_text_edits(state, edits, &uri, tx);
                }
            } else {
                state.language.message = Some("No formatting changes".into());
            }
        }
        Reply::Rename(edit) => {
            if let Some(edit) = edit {
                workspace::apply(state, edit, None, tx);
            } else {
                state.language.message = Some("No rename available at the caret".into());
            }
        }
        Reply::Outline(symbols) => {
            let Some(uri) = state.language.binding().map(|binding| binding.uri.clone()) else {
                return false;
            };
            state.language.outline = match symbols {
                Some(DocumentSymbolResponse::Nested(symbols)) => {
                    let mut entries = Vec::new();
                    nested(&mut entries, symbols, &uri, 0);
                    entries
                }
                Some(DocumentSymbolResponse::Flat(symbols)) => symbols
                    .into_iter()
                    .map(|symbol| Entry {
                        label: symbol.name,
                        detail: location_label(&symbol.location.uri, symbol.location.range.start),
                        location: symbol.location,
                    })
                    .collect(),
                None => Vec::new(),
            };
        }
        Reply::Folding(folds) => {
            let total = state.editor.document().line_count();
            let ranges = folds
                .unwrap_or_default()
                .into_iter()
                .filter_map(|fold| {
                    let (start, end) = (fold.start_line as usize, fold.end_line as usize);
                    (start < end && end < total).then_some(start..end)
                })
                .collect();
            state.editor.set_folds(ranges);
        }
        Reply::Semantic(tokens) => {
            let Some(legend) = &state.language.legend else {
                return false;
            };
            let document = state.editor.document();
            let mut line = 0_u32;
            let mut column = 0_u32;
            let mut marks = Vec::new();
            let tokens = match tokens {
                Some(SemanticTokensResult::Tokens(tokens)) => tokens.data,
                Some(SemanticTokensResult::Partial(tokens)) => tokens.data,
                None => Vec::new(),
            };
            for token in tokens {
                let Some(next_line) = line.checked_add(token.delta_line) else {
                    break;
                };
                line = next_line;
                column = if token.delta_line != 0 {
                    token.delta_start
                } else {
                    let Some(column) = column.checked_add(token.delta_start) else {
                        break;
                    };
                    column
                };
                let Some(end) = column.checked_add(token.length) else {
                    break;
                };
                let Some(kind) = legend
                    .token_types
                    .get(token.token_type as usize)
                    .and_then(|kind| semantic_kind(kind.as_str()))
                else {
                    continue;
                };
                if let Some(range) = range(
                    document,
                    Range::new(Position::new(line, column), Position::new(line, end)),
                ) {
                    marks.push(SemanticMark { range, kind });
                }
            }
            state.editor.set_semantic_marks(marks);
        }
        Reply::Inlay(hints) => {
            let hints = hints
                .unwrap_or_default()
                .into_iter()
                .map(|hint| InlineHint {
                    line: hint.position.line as usize,
                    label: match hint.label {
                        InlayHintLabel::String(label) => label,
                        InlayHintLabel::LabelParts(parts) => {
                            parts.into_iter().map(|part| part.value).collect::<String>()
                        }
                    },
                })
                .collect();
            state.editor.set_inline_hints(hints);
        }
        Reply::Actions(actions) => {
            state.language.result = Some(Results::Actions(actions.unwrap_or_default()))
        }
        Reply::Execute(_) | Reply::Export(_) => {}
        _ => return false,
    }
    true
}

fn semantic_kind(kind: &str) -> Option<SemanticKind> {
    Some(match kind {
        "keyword" | "operator" => SemanticKind::Keyword,
        "comment" => SemanticKind::Comment,
        "string" => SemanticKind::String,
        "type" | "namespace" | "class" | "struct" | "label" => SemanticKind::Type,
        "function" | "method" | "number" | "parameter" | "variable" => SemanticKind::Symbol,
        _ => return None,
    })
}

fn nested(entries: &mut Vec<Entry>, symbols: Vec<DocumentSymbol>, uri: &Url, depth: usize) {
    for symbol in symbols {
        entries.push(Entry {
            label: format!("{}{}", "  ".repeat(depth.min(12)), symbol.name),
            detail: symbol
                .detail
                .unwrap_or_else(|| location_label(uri, symbol.selection_range.start)),
            location: Location::new(uri.clone(), symbol.selection_range),
        });
        if let Some(children) = symbol.children {
            nested(entries, children, uri, depth + 1);
        }
    }
}

fn card(state: &mut State, title: &'static str, body: String, hover: Option<usize>) {
    if !body.trim().is_empty() {
        state.language.hover_dismiss = None;
        state.language.popup = Some(Popup::Card {
            title,
            body,
            hover: hover.map(HoverTarget::Symbol),
        });
    }
}

pub(super) fn documentation(value: &Documentation) -> &str {
    match value {
        Documentation::String(text) => text,
        Documentation::MarkupContent(content) => &content.value,
    }
}

fn marked(value: MarkedString) -> String {
    match value {
        MarkedString::String(text) => text,
        MarkedString::LanguageString(code) => code.value,
    }
}

fn location_label(uri: &Url, position: Position) -> String {
    let name = uri
        .to_file_path()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| uri.to_string());
    format!("{name}  Ln {}", position.line + 1)
}

fn is_error(diagnostic: &Diagnostic) -> bool {
    diagnostic
        .severity
        .is_none_or(|severity| severity == DiagnosticSeverity::ERROR)
}

fn show_locations(state: &mut State, title: &'static str, locations: Vec<Location>) {
    let list = locations
        .into_iter()
        .map(|location| Entry {
            label: location_label(&location.uri, location.range.start),
            detail: String::new(),
            location,
        })
        .collect();
    state.language.result = Some(Results::Locations(title, list));
}

pub(super) fn go(state: &mut State, location: Location, tx: &mut Tx) {
    state.language.popup = None;
    if state
        .language
        .binding()
        .is_some_and(|binding| binding.uri == location.uri)
    {
        if let Some(range) = range(state.editor.document(), location.range) {
            state.workbench.show_editor();
            state.editor.jump(range, tx);
        }
    } else if let Ok(path) = location.uri.to_file_path() {
        files::navigate(state, path, location.range, tx);
    } else {
        state.language.message = Some("The location does not point to a local file".into());
    }
}

fn apply_text_edits(state: &mut State, edits: Vec<TextEdit>, uri: &Url, tx: &mut Tx) -> bool {
    if state
        .language
        .binding()
        .is_none_or(|binding| &binding.uri != uri)
    {
        state.language.message = Some("The edits target another file. Open it first.".into());
        return false;
    }
    let Some(edits) = text_edits(state.editor.document(), &edits) else {
        state.language.message = Some("Tinymist returned an invalid range".into());
        return false;
    };
    edits.is_empty() || state.editor.apply_language_edits(edits, tx)
}
