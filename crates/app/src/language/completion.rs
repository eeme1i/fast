use std::ops::Range;

use fast_core::document::{Document, Edit};
use fast_lsp::{range, snippet, types::*, workspace::text_edits};
use rune::Tx;

use super::{Popup, sync};
use crate::State;

pub(super) fn filter(
    document: &Document,
    caret: usize,
    items: Vec<CompletionItem>,
) -> Vec<CompletionItem> {
    let word = word_prefix(document, caret).to_lowercase();
    let quoted = quoted_content(document, caret)
        .map(|(content, _)| document.slice(content.start..caret).to_lowercase());
    let mut ranked: Vec<_> = items
        .into_iter()
        .filter_map(|item| {
            let filter_text = item
                .filter_text
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or(&item.label);
            let (query, text) = if let Some(query) = &quoted
                && unquote(&item.label).is_some()
            {
                let text = unquote(filter_text).unwrap_or(filter_text).to_lowercase();
                if !text.contains(query) {
                    return None;
                }
                (query, text)
            } else {
                (&word, filter_text.to_lowercase())
            };
            Some((!text.starts_with(query), item))
        })
        .collect();
    ranked.sort_by(|(a_rank, a), (b_rank, b)| {
        a_rank.cmp(b_rank).then_with(|| {
            a.sort_text
                .as_ref()
                .unwrap_or(&a.label)
                .cmp(b.sort_text.as_ref().unwrap_or(&b.label))
        })
    });
    ranked.into_iter().take(80).map(|(_, item)| item).collect()
}

pub(super) fn accept(state: &mut State, index: usize, tx: &mut Tx) {
    sync(state);
    let Some(Popup::Completion {
        mut items,
        version,
        caret,
        ..
    }) = state.language.popup.take()
    else {
        return;
    };
    if items.len() <= index
        || state
            .language
            .connection
            .as_ref()
            .is_none_or(|connection| connection.version != version)
        || state.editor.document().selection().head != caret
    {
        return;
    }
    let item = items.swap_remove(index);
    let document = state.editor.document();
    let selection = document.selection().range();
    let (range, text) = match &item.text_edit {
        Some(CompletionTextEdit::Edit(edit)) => {
            let Some(range) = range(document, edit.range) else {
                return;
            };
            (range, edit.new_text.as_str())
        }
        Some(CompletionTextEdit::InsertAndReplace(edit)) => {
            let Some(range) = range(document, edit.replace) else {
                return;
            };
            (range, edit.new_text.as_str())
        }
        None => {
            let word = word_prefix(document, caret).len();
            let range = if selection.is_empty() {
                caret - word..caret
            } else {
                selection
            };
            (range, item.insert_text.as_deref().unwrap_or(&item.label))
        }
    };
    let (range, text, stops) = if item.text_edit.is_some()
        && range == (caret..caret)
        && text.starts_with('"')
        && let Some(value) = unquote(&item.label)
        && let Some((content, true)) = quoted_content(document, caret)
        && !content.is_empty()
    {
        // Tinymist can return a zero-width insertion inside a partial string.
        (content, value.to_owned(), Vec::new())
    } else if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
        let expanded = snippet(text);
        (range, expanded.text, expanded.stops)
    } else {
        (range, text.to_owned(), Vec::new())
    };
    let Some(mut edits) = text_edits(document, &item.additional_text_edits.unwrap_or_default())
    else {
        state.language.message = Some("Tinymist supplied an invalid additional edit".into());
        return;
    };
    let shift = edits
        .iter()
        .filter(|edit| edit.range.end <= range.start)
        .map(|edit| edit.text.len() as isize - edit.range.len() as isize)
        .sum::<isize>();
    let inserted = range.start.saturating_add_signed(shift);
    let stops = stops
        .into_iter()
        .map(|stop| inserted + stop.start..inserted + stop.end)
        .collect();
    edits.push(Edit { range, text });
    if !state.editor.apply_completion(edits, stops, tx) {
        state.language.message = Some("Completion edits overlap or are no longer valid".into());
    }
}

pub(super) fn quoted_content(document: &Document, caret: usize) -> Option<(Range<usize>, bool)> {
    let line = document.line_range(document.line_at(caret));
    let text = document.slice(line.clone());
    let mut start = None;
    let mut escaped = false;
    for (offset, ch) in text.char_indices() {
        let offset = line.start + offset;
        if escaped {
            escaped = false;
        } else if ch == '\\' && start.is_some() {
            escaped = true;
        } else if ch == '"' {
            if let Some(start) = start.take() {
                if offset >= caret {
                    return Some((start..offset, true));
                }
            } else if offset < caret {
                start = Some(offset + 1);
            } else {
                break;
            }
        }
    }
    start.map(|start| (start..line.end, false))
}

fn unquote(text: &str) -> Option<&str> {
    text.strip_prefix('"')?.strip_suffix('"')
}

fn word_prefix(document: &Document, caret: usize) -> String {
    let preceding = document.slice(document.line_range(document.line_at(caret)).start..caret);
    let bytes = preceding
        .chars()
        .rev()
        .take_while(|ch| ch.is_alphanumeric() || "_-".contains(*ch))
        .map(char::len_utf8)
        .sum::<usize>();
    preceding[preceding.len() - bytes..].to_owned()
}
