use fast_core::document::{Document, Selection};
use rune::{Key, KeyEvent, Tx};

use crate::{Editor, Host};

pub(crate) fn key<S: Host>(state: &mut S, event: &KeyEvent, tx: &mut Tx) -> bool {
    if state.editor_key(event, tx) {
        return true;
    }
    let editor = state.editor_mut();
    if !matches!(event.key, Key::Up | Key::Down) {
        editor.preferred_column = None;
    }
    let edits = match &event.key {
        Key::Text(text) if !event.is_command_chord() => editor.type_text(text),
        Key::Enter => editor.insert_newline(),
        Key::Tab if !event.mods.shift && editor.next_snippet_stop(tx) => return true,
        Key::Tab if !event.mods.shift => editor.document.insert("  ", false),
        Key::Backspace => editor.backspace(),
        Key::Delete => editor.document.delete(false),
        Key::Left | Key::Right | Key::Home | Key::End | Key::Up | Key::Down => {
            editor.navigate(event);
            Vec::new()
        }
        Key::Escape if editor.find.open => {
            editor.close_find(tx);
            return true;
        }
        Key::Escape => return editor.single_caret(tx),
        _ => return false,
    };
    editor.focused = true;
    editor.changed(edits, tx);
    true
}

#[derive(Clone, Copy)]
pub enum EditCommand {
    Undo,
    Redo,
    RedoAlternate,
    SelectAll,
    Copy,
    Cut,
    Paste,
}

impl Editor {
    pub fn run(&mut self, command: EditCommand, tx: &mut Tx) {
        let edits = match command {
            EditCommand::Undo => self.document.undo(),
            EditCommand::Redo => self.document.redo(),
            EditCommand::RedoAlternate => self.document.redo_alternate(),
            EditCommand::SelectAll => {
                self.document.select(Selection {
                    anchor: 0,
                    head: self.document.len(),
                });
                Vec::new()
            }
            EditCommand::Copy | EditCommand::Cut => {
                let mut ranges: Vec<_> = self
                    .document
                    .selections()
                    .iter()
                    .filter(|selection| !selection.is_empty())
                    .map(|selection| selection.range())
                    .collect();
                if ranges.is_empty() {
                    return;
                }
                ranges.sort_by_key(|range| range.start);
                let selected = ranges
                    .into_iter()
                    .map(|range| self.document.slice(range))
                    .collect::<Vec<_>>();
                tx.copy(selected.join(self.document.ending()));
                if matches!(command, EditCommand::Cut) {
                    self.document.insert("", false)
                } else {
                    return;
                }
            }
            EditCommand::Paste => {
                tx.request_paste();
                return;
            }
        };
        self.preferred_column = None;
        self.changed(edits, tx);
    }

    pub fn insert_plain(&mut self, text: &str, tx: &mut Tx) {
        let edits = self.document.insert(text, false);
        self.changed(edits, tx);
    }

    fn insert_newline(&mut self) -> Vec<fast_core::document::Edit> {
        let ending = self.document.ending();
        self.document.replace_selections(
            |document, selection| {
                let range = selection.range();
                let line = document.line_range(document.line_at(range.start));
                let before = document.slice(line.start..range.start);
                let indent = before.len() - before.trim_start_matches([' ', '\t']).len();
                let text = format!("{ending}{}", &before[..indent]);
                let caret = text.len();
                (range, text, caret)
            },
            false,
        )
    }

    fn type_text(&mut self, text: &str) -> Vec<fast_core::document::Edit> {
        let closing = match text {
            "(" => Some(")"),
            "[" => Some("]"),
            "{" => Some("}"),
            "\"" | "$" => Some(text),
            _ => None,
        };
        if matches!(text, ")" | "]" | "}" | "\"" | "$")
            && self.document.selections().iter().all(|selection| {
                selection.is_empty()
                    && selection.head < self.document.len()
                    && self
                        .document
                        .slice(selection.head..self.document.next_grapheme(selection.head))
                        == text
            })
        {
            let selections = self
                .document
                .selections()
                .iter()
                .map(|selection| Selection::caret(selection.head + text.len()))
                .collect();
            self.document.set_selections(selections);
            return Vec::new();
        }
        match closing {
            Some(closing) => self.document.replace_selections(
                |document, selection| {
                    let range = selection.range();
                    let next = document.slice(range.end..document.next_grapheme(range.end));
                    if range.is_empty()
                        && !next
                            .chars()
                            .next()
                            .is_none_or(|ch| ch.is_whitespace() || ")]},;".contains(ch))
                    {
                        return (range, text.to_owned(), text.len());
                    }
                    let inside = document.slice(range.clone());
                    let offset = text.len() + inside.len();
                    (range, format!("{text}{inside}{closing}"), offset)
                },
                false,
            ),
            None => self.document.insert(text, true),
        }
    }

    fn backspace(&mut self) -> Vec<fast_core::document::Edit> {
        let selection = self.document.selection();
        if self.document.selections().len() == 1 && selection.is_empty() {
            let at = selection.head;
            let start = self.document.previous_grapheme(at);
            let end = self.document.next_grapheme(at);
            if matches!(
                self.document.slice(start..end).as_str(),
                "()" | "[]" | "{}" | "\"\"" | "$$"
            ) {
                return self
                    .document
                    .replace_ranges(vec![fast_core::document::Edit {
                        range: start..end,
                        text: String::new(),
                    }]);
            }
        }
        self.document.delete(true)
    }

    fn navigate(&mut self, event: &KeyEvent) {
        let document = &self.document;
        let primary = document.selection().head;
        let primary_column = primary - document.line_range(document.line_at(primary)).start;
        let column = *self.preferred_column.get_or_insert(primary_column);
        let selections = document
            .selections()
            .iter()
            .map(|&selection| {
                let line = document.line_at(selection.head);
                let span = document.line_range(line);
                let head = match &event.key {
                    Key::Left if !event.mods.shift && !selection.is_empty() => {
                        selection.range().start
                    }
                    Key::Right if !event.mods.shift && !selection.is_empty() => {
                        selection.range().end
                    }
                    Key::Left if event.mods.cmd => span.start,
                    Key::Right if event.mods.cmd => span.end,
                    Key::Left => document.previous_grapheme(selection.head),
                    Key::Right => document.next_grapheme(selection.head),
                    Key::Home if event.mods.cmd => 0,
                    Key::End if event.mods.cmd => document.len(),
                    Key::Home => span.start,
                    Key::End => span.end,
                    Key::Up if event.mods.cmd => 0,
                    Key::Down if event.mods.cmd => document.len(),
                    Key::Up | Key::Down => {
                        let target = if matches!(event.key, Key::Up) {
                            line.saturating_sub(1)
                        } else {
                            (line + 1).min(document.line_count() - 1)
                        };
                        let range = document.line_range(target);
                        let preferred = if selection.head == primary {
                            column
                        } else {
                            selection.head - span.start
                        };
                        document.floor_boundary((range.start + preferred).min(range.end))
                    }
                    _ => selection.head,
                };
                if event.mods.shift {
                    Selection {
                        anchor: selection.anchor,
                        head,
                    }
                } else {
                    Selection::caret(head)
                }
            })
            .collect();
        self.document.set_selections(selections);
        if !matches!(event.key, Key::Up | Key::Down) {
            self.preferred_column = None;
        }
    }
}

pub(crate) fn select_word(document: &Document, at: usize) -> Selection {
    let range = document.line_range(document.line_at(at));
    if at == range.end {
        return Selection::caret(at);
    }
    let mut end = document.next_grapheme(at);
    if !word(&document.slice(at..end)) {
        return Selection {
            anchor: at,
            head: end,
        };
    }
    let mut start = at;
    while start > range.start {
        let previous = document.previous_grapheme(start);
        if !word(&document.slice(previous..start)) {
            break;
        }
        start = previous;
    }
    while end < range.end {
        let next = document.next_grapheme(end);
        if !word(&document.slice(end..next)) {
            break;
        }
        end = next;
    }
    Selection {
        anchor: start,
        head: end,
    }
}

pub(super) fn word(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}
