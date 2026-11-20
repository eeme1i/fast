use fast_core::document::{Document, Selection};
use rune::{Drag, Phase, Tx};

use crate::{
    Editor, Host, VIEWPORT,
    input::{select_word, word},
    view::{Style, hit},
};

pub(super) enum Gesture {
    Linear(Selection),
    Column {
        row: usize,
        x: f32,
        previous: Vec<Selection>,
        dragged: bool,
    },
}

impl Editor {
    pub fn select_next_occurrence(&mut self, tx: &mut Tx) {
        let document = &mut self.document;
        let primary = document.selection();
        let mut selections = document.selections().to_vec();
        if primary.is_empty() {
            let word = word_at_caret(document, primary.head);
            if let Some(last) = selections.last_mut() {
                *last = word;
            }
        } else {
            let range = primary.range();
            let needle = document.slice(range.clone());
            let text = document.snapshot().to_string();
            let next = [range.end..text.len(), 0..range.start]
                .into_iter()
                .flat_map(|segment| {
                    let start = segment.start;
                    text[segment]
                        .match_indices(&needle)
                        .map(move |(offset, text)| start + offset..start + offset + text.len())
                })
                .find(|candidate| {
                    !selections.iter().any(|selection| {
                        let range = selection.range();
                        if range.is_empty() {
                            candidate.start <= range.start && range.start <= candidate.end
                        } else {
                            candidate.start < range.end && range.start < candidate.end
                        }
                    })
                });
            if let Some(range) = next {
                selections.push(Selection {
                    anchor: range.start,
                    head: range.end,
                });
            }
        }
        document.set_selections(selections);
        self.reveal(tx);
    }

    pub(crate) fn single_caret(&mut self, tx: &mut Tx) -> bool {
        if self.document.selections().len() < 2 {
            return false;
        }
        self.document.select(self.document.selection());
        self.reveal(tx);
        true
    }
}

fn word_at_caret(document: &Document, at: usize) -> Selection {
    let line = document.line_range(document.line_at(at));
    if at < line.end && word(&document.slice(at..document.next_grapheme(at))) {
        return select_word(document, at);
    }
    if at > line.start {
        let previous = document.previous_grapheme(at);
        if word(&document.slice(previous..at)) {
            return select_word(document, previous);
        }
    }
    Selection::caret(at)
}

pub(super) fn pointer<S: Host>(
    state: &mut S,
    start_row: usize,
    style: Style,
    drag: &Drag,
    tx: &mut Tx,
) {
    if matches!(drag.phase, Phase::Up) {
        let editor = state.editor_mut();
        if let Some(Gesture::Column { previous, .. }) = editor.drag_origin.take()
            && drag.cancelled()
        {
            editor.document.set_selections(previous);
            editor.reveal(tx);
        } else {
            tx.live_only();
        }
        return;
    }
    let down = matches!(drag.phase, Phase::Down);
    if down {
        state.editor_hover(None, None, tx);
    }
    let offset = (drag.y / style.line_height).floor() as isize;
    let row = start_row
        .saturating_add_signed(offset)
        .min(state.editor().displayed_count() - 1);
    let line = state.editor().displayed_line(row);
    let (byte, glyph) = hit(state, line, style, drag.x);
    if down && tx.mods().alt {
        let editor = state.editor_mut();
        let previous = editor.document.selections().to_vec();
        let mut selections = previous.clone();
        let caret = Selection::caret(byte);
        if selections.len() > 1 && selections.contains(&caret) {
            selections.retain(|selection| *selection != caret);
        } else {
            selections.push(caret);
        }
        editor.document.set_selections(selections);
        editor.drag_origin = Some(Gesture::Column {
            row,
            x: drag.x,
            previous,
            dragged: false,
        });
    } else if !down
        && let Some(Gesture::Column {
            row: origin,
            x,
            dragged,
            ..
        }) = state.editor().drag_origin
    {
        if !dragged && drag.dx.abs().max(drag.dy.abs()) < 3.0 && row == origin {
            tx.live_only();
            return;
        }
        let selections = (row.min(origin)..=row.max(origin)).map(|display| {
            let line = state.editor().displayed_line(display);
            Selection {
                anchor: hit(state, line, style, x).0,
                head: hit(state, line, style, drag.x).0,
            }
        });
        let selections = if row < origin {
            selections.rev().collect()
        } else {
            selections.collect()
        };
        let editor = state.editor_mut();
        if let Some(Gesture::Column { dragged, .. }) = &mut editor.drag_origin {
            *dragged = true;
        }
        editor.document.set_selections(selections);
    } else {
        let editor = state.editor_mut();
        let document = &mut editor.document;
        let unit = match drag.clicks {
            2 => glyph.map_or_else(|| Selection::caret(byte), |at| select_word(document, at)),
            3.. => {
                let range = document.line_range(line);
                Selection {
                    anchor: range.start,
                    head: range.end,
                }
            }
            _ => Selection::caret(byte),
        };
        let selection = if down && tx.mods().shift {
            let anchor = document.selection().anchor;
            editor.drag_origin = Some(Gesture::Linear(Selection::caret(anchor)));
            Selection { anchor, head: byte }
        } else if down {
            editor.drag_origin = Some(Gesture::Linear(unit));
            unit
        } else if let Some(Gesture::Linear(origin)) = editor.drag_origin {
            let unit = if origin.is_empty() {
                Selection::caret(byte)
            } else {
                unit
            };
            if unit.head <= origin.anchor {
                Selection {
                    anchor: origin.head,
                    head: unit.anchor,
                }
            } else {
                Selection {
                    anchor: origin.anchor,
                    head: origin.head.max(unit.head),
                }
            }
        } else {
            tx.live_only();
            return;
        };
        document.select(selection);
    }
    let editor = state.editor_mut();
    editor.preferred_column = None;
    editor.focused = true;
    tx.focus(VIEWPORT, false);
    editor.reveal(tx);
}
