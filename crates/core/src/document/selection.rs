use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
}

pub(super) fn normalize(selections: impl IntoIterator<Item = Selection>) -> Vec<Selection> {
    let mut ordered: Vec<_> = selections.into_iter().enumerate().collect();
    ordered.sort_by_key(|(_, selection)| selection.range().start);
    let mut merged: Vec<(usize, Selection)> = Vec::with_capacity(ordered.len());
    for (index, selection) in ordered {
        if let Some((previous_index, previous)) = merged.last_mut() {
            let a = previous.range();
            let b = selection.range();
            if b.start <= a.end {
                let backward = if index > *previous_index {
                    selection.anchor > selection.head
                } else {
                    previous.anchor > previous.head
                };
                let end = a.end.max(b.end);
                *previous = if backward {
                    Selection {
                        anchor: end,
                        head: a.start,
                    }
                } else {
                    Selection {
                        anchor: a.start,
                        head: end,
                    }
                };
                *previous_index = (*previous_index).max(index);
                continue;
            }
        }
        merged.push((index, selection));
    }
    merged.sort_by_key(|(index, _)| *index);
    if merged.is_empty() {
        return vec![Selection::caret(0)];
    }
    merged.into_iter().map(|(_, selection)| selection).collect()
}
