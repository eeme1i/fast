use std::{collections::HashMap, ops::Range};

use fast_core::document::{Document, Edit, Position as CorePosition};
use lsp_types::*;
use ropey::Rope;
use serde_json::Value;

use crate::{Command, protocol};

pub fn position(document: &Document, byte: usize) -> Option<Position> {
    let p = document.byte_to_utf16(byte)?;
    Some(Position::new(
        p.line.try_into().ok()?,
        p.column.try_into().ok()?,
    ))
}

pub fn byte(document: &Document, p: Position) -> Option<usize> {
    document.utf16_to_byte(CorePosition {
        line: p.line as usize,
        column: p.character as usize,
    })
}

pub fn range(document: &Document, r: lsp_types::Range) -> Option<Range<usize>> {
    let from = byte(document, r.start)?;
    let to = byte(document, r.end)?;
    (from <= to).then_some(from..to)
}

#[derive(Default)]
pub(crate) struct Replica {
    documents: HashMap<Url, Open>,
    pub pinned: Option<String>,
}

struct Open {
    text: Rope,
    version: i32,
    changes: Vec<TextDocumentContentChangeEvent>,
}

impl Replica {
    pub fn reopen(&mut self) -> impl Iterator<Item = Value> + '_ {
        self.documents.iter_mut().map(|(uri, file)| {
            file.changes.clear();
            open_message(uri.clone(), file)
        })
    }

    pub fn notification(&mut self, command: Command, online: bool) -> Option<Value> {
        match command {
            Command::Open { uri, version, text } => {
                let file = Open {
                    text,
                    version,
                    changes: Vec::new(),
                };
                let message = online.then(|| open_message(uri.clone(), &file));
                self.documents.insert(uri, file);
                message
            }
            Command::Change {
                uri,
                version,
                edits,
                text,
            } => {
                let file = self.documents.get_mut(&uri)?;
                if version <= file.version {
                    return None;
                }
                if online {
                    match incremental(&file.text, &edits) {
                        Some(changes)
                            if !changes.is_empty() && file.changes.len() + changes.len() < 2048 =>
                        {
                            file.changes.extend(changes)
                        }
                        _ => file.changes = vec![full(&text)],
                    }
                }
                file.text = text;
                file.version = version;
                None
            }
            Command::Save(uri) if online && self.documents.contains_key(&uri) => {
                Some(protocol::notice(
                    "textDocument/didSave",
                    DidSaveTextDocumentParams {
                        text_document: TextDocumentIdentifier::new(uri),
                        text: None,
                    },
                ))
            }
            Command::Close(uri) => {
                let existed = self.documents.remove(&uri).is_some();
                (online && existed).then(|| {
                    protocol::notice(
                        "textDocument/didClose",
                        DidCloseTextDocumentParams {
                            text_document: TextDocumentIdentifier::new(uri),
                        },
                    )
                })
            }
            Command::FilesChanged(changes) if online => Some(protocol::notice(
                "workspace/didChangeWatchedFiles",
                DidChangeWatchedFilesParams { changes },
            )),
            _ => None,
        }
    }

    pub fn drain(&mut self) -> impl Iterator<Item = Value> + '_ {
        self.documents
            .iter_mut()
            .filter(|(_, file)| !file.changes.is_empty())
            .map(|(uri, file)| {
                protocol::notice(
                    "textDocument/didChange",
                    DidChangeTextDocumentParams {
                        text_document: VersionedTextDocumentIdentifier::new(
                            uri.clone(),
                            file.version,
                        ),
                        content_changes: std::mem::take(&mut file.changes),
                    },
                )
            })
    }
}

fn open_message(uri: Url, file: &Open) -> Value {
    protocol::notice(
        "textDocument/didOpen",
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem::new(
                uri,
                "typst".to_owned(),
                file.version,
                file.text.to_string(),
            ),
        },
    )
}

fn full(text: &Rope) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: text.to_string(),
    }
}

fn incremental(before: &Rope, edits: &[Edit]) -> Option<Vec<TextDocumentContentChangeEvent>> {
    let mut text = before.clone();
    let mut result = Vec::with_capacity(edits.len());
    for edit in edits {
        let document = Document::from_rope(text.clone());
        let start = position(&document, edit.range.start)?;
        let end = position(&document, edit.range.end)?;
        if edit.range.start > edit.range.end {
            return None;
        }
        let from = text.byte_to_char(edit.range.start);
        let to = text.byte_to_char(edit.range.end);
        text.remove(from..to);
        text.insert(from, &edit.text);
        result.push(TextDocumentContentChangeEvent {
            range: Some(lsp_types::Range::new(start, end)),
            range_length: None,
            text: edit.text.clone(),
        });
    }
    Some(result)
}
