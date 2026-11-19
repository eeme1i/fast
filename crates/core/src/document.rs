mod history;
mod position;
mod selection;

use std::ops::Range;
use std::time::Instant;

use ropey::Rope;

use history::{History, Transaction};
pub use position::Position;
pub use selection::Selection;
use selection::normalize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

pub struct Document {
    text: Rope,
    selections: Vec<Selection>,
    history: History,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("")
    }
}

impl Document {
    pub fn new(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text))
    }

    pub fn from_rope(text: Rope) -> Self {
        Self {
            text,
            selections: vec![Selection::caret(0)],
            history: History::default(),
        }
    }

    pub fn snapshot(&self) -> Rope {
        self.text.clone()
    }

    pub fn len(&self) -> usize {
        self.text.len_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn selection(&self) -> Selection {
        self.selections[self.selections.len() - 1]
    }

    /// The last selection is primary. Ranges are disjoint and lie on UTF-8 boundaries.
    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    pub fn select(&mut self, selection: Selection) {
        self.set_selections(vec![selection]);
    }

    pub fn set_selections(&mut self, selections: Vec<Selection>) {
        let selections = normalize(selections.into_iter().map(|selection| Selection {
            anchor: self.floor_boundary(selection.anchor),
            head: self.floor_boundary(selection.head),
        }));
        if selections != self.selections {
            self.history.end_typing();
            self.selections = selections;
        }
    }

    pub fn revision(&self) -> u64 {
        self.history.revision()
    }

    pub fn end_typing(&mut self) {
        self.history.end_typing();
    }

    pub fn insert(&mut self, text: &str, typing: bool) -> Vec<Edit> {
        self.replace_selections(
            |_, selection| (selection.range(), text.to_owned(), text.len()),
            typing,
        )
    }

    pub fn delete(&mut self, backward: bool) -> Vec<Edit> {
        // Adjacent carets can expand over the same grapheme or selected text.
        let ranges = normalize(self.selections.iter().map(|&selection| {
            if !selection.is_empty() {
                return selection;
            }
            let head = selection.head;
            let anchor = if backward {
                self.previous_grapheme(head)
            } else {
                self.next_grapheme(head)
            };
            Selection { anchor, head }
        }));
        self.replace_ranges(
            ranges
                .into_iter()
                .map(|selection| Edit {
                    range: selection.range(),
                    text: String::new(),
                })
                .collect(),
        )
    }

    /// Each replacement supplies its range and the new caret's byte offset within its text.
    /// The callback sees the document before any of the replacements.
    pub fn replace_selections(
        &mut self,
        mut replace: impl FnMut(&Self, Selection) -> (Range<usize>, String, usize),
        typing: bool,
    ) -> Vec<Edit> {
        let edits = self
            .selections
            .iter()
            .map(|&selection| {
                let (range, text, caret) = replace(self, selection);
                let caret = caret.min(text.len());
                (Edit { range, text }, caret)
            })
            .collect();
        self.apply_replacements(edits, typing)
    }

    /// Apply non-overlapping replacements in one undo step. The last supplied range is primary.
    pub fn replace_ranges(&mut self, edits: Vec<Edit>) -> Vec<Edit> {
        let edits = edits
            .into_iter()
            .map(|edit| {
                let caret = edit.text.len();
                (edit, caret)
            })
            .collect();
        self.apply_replacements(edits, false)
    }

    fn apply_replacements(&mut self, edits: Vec<(Edit, usize)>, typing: bool) -> Vec<Edit> {
        let mut edits: Vec<_> = edits.into_iter().enumerate().collect();
        edits.sort_by_key(|(_, (edit, _))| edit.range.start);
        let mut selections = vec![Selection::caret(0); edits.len()];
        let mut shift = 0;
        for (index, (edit, caret)) in &edits {
            selections[*index] =
                Selection::caret(edit.range.start.saturating_add_signed(shift) + caret);
            shift += edit.text.len() as isize - edit.range.len() as isize;
        }
        self.transact(
            edits.into_iter().map(|(_, (edit, _))| edit).collect(),
            selections,
            typing,
        )
    }

    fn transact(
        &mut self,
        mut edits: Vec<Edit>,
        selections: Vec<Selection>,
        typing: bool,
    ) -> Vec<Edit> {
        if edits.is_empty()
            || edits.iter().any(|e| !self.valid_range(&e.range))
            || edits
                .windows(2)
                .any(|pair| pair[0].range.end > pair[1].range.start)
        {
            return Vec::new();
        }
        edits.retain(|edit| !edit.range.is_empty() || !edit.text.is_empty());
        if edits.is_empty() {
            self.set_selections(selections);
            return edits;
        }
        edits.reverse();
        let before = self.selections.clone();
        let backward = self.apply(&edits);
        self.selections = normalize(
            selections
                .into_iter()
                .map(|selection| Selection::caret(self.floor_boundary(selection.head))),
        );
        self.history.record(
            Transaction {
                before,
                after: self.selections.clone(),
                forward: edits.clone(),
                backward,
            },
            typing.then(Instant::now),
        );
        edits
    }

    /// The returned edits use the offsets at the moment each edit is applied.
    fn apply(&mut self, edits: &[Edit]) -> Vec<Edit> {
        let mut reversed = Vec::with_capacity(edits.len());
        for edit in edits {
            let start = self.text.byte_to_char(edit.range.start);
            let end = self.text.byte_to_char(edit.range.end);
            let previous = self.text.slice(start..end).to_string();
            self.text.remove(start..end);
            self.text.insert(start, &edit.text);
            reversed.push(Edit {
                range: edit.range.start..edit.range.start + edit.text.len(),
                text: previous,
            });
        }
        reversed
    }

    pub fn undo(&mut self) -> Vec<Edit> {
        let Some(transaction) = self.history.undo() else {
            return Vec::new();
        };
        let edits: Vec<_> = transaction.backward.iter().rev().cloned().collect();
        self.selections.clone_from(&transaction.before);
        self.apply(&edits);
        edits
    }

    pub fn redo(&mut self) -> Vec<Edit> {
        self.redo_on_branch(false)
    }

    pub fn redo_alternate(&mut self) -> Vec<Edit> {
        self.redo_on_branch(true)
    }

    fn redo_on_branch(&mut self, alternate: bool) -> Vec<Edit> {
        let Some(transaction) = self.history.redo(alternate) else {
            return Vec::new();
        };
        let edits = transaction.forward.clone();
        self.selections.clone_from(&transaction.after);
        self.apply(&edits);
        edits
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        self.text.byte_slice(range).to_string()
    }
}
