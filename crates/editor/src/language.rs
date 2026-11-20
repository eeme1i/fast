use std::ops::Range;

use fast_core::document::{Edit, Selection};
use rune::Tx;

use crate::{Editor, VIEWPORT, view};

#[derive(Clone, PartialEq, Eq)]
pub enum HoverTarget {
    Symbol(usize),
    Diagnostics { anchor: usize, indices: Vec<usize> },
}

impl HoverTarget {
    pub fn anchor(&self) -> usize {
        match self {
            Self::Symbol(byte) => *byte,
            Self::Diagnostics { anchor, .. } => *anchor,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Problem {
    pub range: Range<usize>,
    pub error: bool,
    pub message: String,
}

impl Problem {
    pub fn label(&self) -> &'static str {
        if self.error { "Error" } else { "Warning" }
    }
}

#[derive(Clone, Copy)]
pub enum SemanticKind {
    Keyword,
    String,
    Comment,
    Type,
    Symbol,
}

pub struct SemanticMark {
    pub range: Range<usize>,
    pub kind: SemanticKind,
}

pub struct InlineHint {
    pub line: usize,
    pub label: String,
}

#[derive(Default)]
pub(crate) struct Language {
    pub track: bool,
    pub edits: Vec<Edit>,
    pub problems: Vec<Problem>,
    pub semantic: Vec<SemanticMark>,
    pub hints: Vec<InlineHint>,
    pub folds: Vec<Range<usize>>,
    pub collapsed: Vec<Range<usize>>,
    pub fold_version: u64,
    pub snippets: Vec<Range<usize>>,
}

impl Editor {
    pub fn track_edits(&mut self, track: bool) {
        self.language.track = track;
        self.language.edits.clear();
    }

    pub fn take_edits(&mut self) -> Vec<Edit> {
        std::mem::take(&mut self.language.edits)
    }

    pub fn visible_lines(&self) -> Range<usize> {
        self.visible.clone()
    }

    pub fn problems(&self) -> &[Problem] {
        &self.language.problems
    }

    pub fn set_problems(&mut self, problems: Vec<Problem>) {
        self.language.problems = problems;
    }

    pub fn set_semantic_marks(&mut self, mut marks: Vec<SemanticMark>) {
        marks.sort_by_key(|mark| mark.range.start);
        self.language.semantic = marks;
    }

    pub fn set_inline_hints(&mut self, mut hints: Vec<InlineHint>) {
        hints.sort_by_key(|hint| hint.line);
        self.language.hints = hints;
    }

    pub fn set_folds(&mut self, folds: Vec<Range<usize>>) {
        self.language.folds = folds;
    }

    pub fn jump(&mut self, range: Range<usize>, tx: &mut Tx) {
        let line = self.select_and_unfold(range);
        self.mark_activity(tx);
        tx.focus(VIEWPORT, false);
        tx.scroll_center_in_instant(VIEWPORT, view::line_key(line));
    }

    fn select_and_unfold(&mut self, range: Range<usize>) -> usize {
        self.document.select(Selection {
            anchor: range.start,
            head: range.end,
        });
        let line = self.document.line_at(range.end);
        let before = self.language.collapsed.len();
        self.language
            .collapsed
            .retain(|fold| line <= fold.start || line > fold.end);
        if before != self.language.collapsed.len() {
            self.language.fold_version += 1;
        }
        line
    }

    pub fn apply_language_edits(&mut self, edits: Vec<Edit>, tx: &mut Tx) -> bool {
        self.apply_language_replacements(edits, Vec::new(), tx)
    }

    /// Apply a completion with its primary edit last and snippet stops in the resulting document.
    pub fn apply_completion(
        &mut self,
        edits: Vec<Edit>,
        stops: Vec<Range<usize>>,
        tx: &mut Tx,
    ) -> bool {
        if !self.apply_language_replacements(edits, stops, tx) {
            return false;
        }
        tx.focus(VIEWPORT, false);
        true
    }

    fn apply_language_replacements(
        &mut self,
        edits: Vec<Edit>,
        mut stops: Vec<Range<usize>>,
        tx: &mut Tx,
    ) -> bool {
        let edits = self.document.replace_ranges(edits);
        if edits.is_empty() {
            return false;
        }

        stops.reverse();
        let selection = stops
            .pop()
            .map_or(self.document.selection(), |range| Selection {
                anchor: range.start,
                head: range.end,
            });
        self.document.select(selection);
        self.language.snippets.clear();
        self.changed(edits, tx);
        self.language.snippets = stops;
        true
    }

    pub(crate) fn next_snippet_stop(&mut self, tx: &mut Tx) -> bool {
        let Some(range) = self.language.snippets.pop() else {
            return false;
        };
        self.select_and_unfold(range);
        self.reveal(tx);
        true
    }

    pub fn toggle_fold(&mut self, line: usize, tx: &mut Tx) -> bool {
        let language = &mut self.language;
        if let Some(index) = language
            .collapsed
            .iter()
            .position(|fold| fold.start == line)
        {
            language.collapsed.remove(index);
        } else {
            let Some(fold) = language
                .folds
                .iter()
                .filter(|fold| fold.start == line)
                .max_by_key(|fold| fold.end)
                .cloned()
            else {
                return false;
            };
            language
                .collapsed
                .retain(|old| old.end <= fold.start || old.start > fold.end);
            let at = language
                .collapsed
                .partition_point(|old| old.start < fold.start);
            language.collapsed.insert(at, fold.clone());
            let caret = self.document.line_at(self.document.selection().head);
            if caret > fold.start && caret <= fold.end {
                self.document
                    .select(Selection::caret(self.document.line_range(fold.start).end));
            }
        }
        language.fold_version += 1;
        tx.instant();
        true
    }

    pub(crate) fn displayed_count(&self) -> usize {
        self.document.line_count()
            - self
                .language
                .collapsed
                .iter()
                .map(Range::len)
                .sum::<usize>()
    }

    pub(crate) fn displayed_line(&self, mut index: usize) -> usize {
        for fold in &self.language.collapsed {
            if index <= fold.start {
                break;
            }
            index += fold.len();
        }
        index.min(self.document.line_count() - 1)
    }
}

pub(crate) fn adjust(range: &mut Range<usize>, edit: &Edit) -> bool {
    if range.end <= edit.range.start && range.start < edit.range.start {
        return true;
    }
    if range.start < edit.range.end {
        return false;
    }
    let delta = edit.text.len() as isize - edit.range.len() as isize;
    range.start = range.start.saturating_add_signed(delta);
    range.end = range.end.saturating_add_signed(delta);
    true
}
