use std::ops::Range;

use fast_core::document::{Document, Selection};
use fast_core::search::{Options, Search};
use fast_theme::tok::Palette;
use rune::{El, Key, TextEdit, Tx, col, row, text, text_input};

use crate::{Editor, Host, VIEWPORT};

const QUERY: &str = "find-query";
const REPLACEMENT: &str = "find-replacement";

#[derive(Default)]
pub(crate) struct Find {
    pub open: bool,
    pub replacing: bool,
    query: TextEdit,
    replacement: TextEdit,
    options: Options,
    search: Option<Search>,
    pub matches: Vec<Range<usize>>,
    error: Option<String>,
}

impl Find {
    pub fn scan(&mut self, document: &Document) {
        self.matches.clear();
        if let Some(search) = &self.search {
            self.matches = search.matches(&document.snapshot().to_string());
        }
    }

    fn update(&mut self, document: &Document) {
        self.search = None;
        self.error = None;
        if !self.query.text.is_empty() {
            match Search::new(&self.query.text, self.options) {
                Ok(search) => self.search = Some(search),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        self.scan(document);
    }

    fn current(&self, selection: Selection) -> Option<usize> {
        self.matches
            .binary_search_by_key(&selection.range().start, |range| range.start)
            .ok()
            .filter(|&index| self.matches[index] == selection.range())
    }
}

impl Editor {
    pub fn has_find(&self) -> bool {
        self.find.open
    }

    pub fn find(&mut self, replace: bool, tx: &mut Tx) {
        self.find.open = true;
        self.find.replacing = replace;
        let selection = self.document.selection().range();
        // A single line selection is a useful starting query.
        if !selection.is_empty()
            && self.document.line_at(selection.start) == self.document.line_at(selection.end)
        {
            self.find.query.replace_text(self.document.slice(selection));
        }
        self.find.query.set_selection(0, self.find.query.text.len());
        self.find.update(&self.document);
        self.focused = false;
        tx.focus(QUERY, true);
        tx.instant();
    }

    pub fn close_find(&mut self, tx: &mut Tx) {
        self.find.open = false;
        self.find.matches.clear();
        tx.focus(VIEWPORT, false);
        self.reveal(tx);
    }

    fn step_match(&mut self, backward: bool, include_current: bool, tx: &mut Tx) {
        if self.find.matches.is_empty() {
            return;
        }
        let start = self.document.selection().range().start;
        let index = if backward {
            self.find
                .matches
                .partition_point(|range| range.start < start)
                .checked_sub(1)
                .unwrap_or(self.find.matches.len() - 1)
        } else {
            self.find.matches.partition_point(|range| {
                range.start < start || (!include_current && range.start == start)
            }) % self.find.matches.len()
        };
        let matched = &self.find.matches[index];
        self.document.select(Selection {
            anchor: matched.start,
            head: matched.end,
        });
        let line = self.document.line_at(matched.start);
        self.mark_activity(tx);
        if !self.visible.contains(&line) {
            tx.scroll_center_in_instant(VIEWPORT, crate::view::line_key(line));
        }
    }

    fn refresh_find(&mut self, tx: &mut Tx) {
        self.find.update(&self.document);
        self.step_match(false, true, tx);
        tx.instant();
    }

    fn replace_match(&mut self, all: bool, tx: &mut Tx) {
        if !all && self.find.current(self.document.selection()).is_none() {
            self.step_match(false, true, tx);
        }
        let Some(search) = &self.find.search else {
            return;
        };
        let only = if all {
            None
        } else {
            let Some(index) = self.find.current(self.document.selection()) else {
                return;
            };
            Some(self.find.matches[index].clone())
        };
        let edits = search.replacements(
            &self.document.snapshot().to_string(),
            &self.find.replacement.text,
            only,
        );
        let applied = self.document.replace_ranges(edits);
        self.changed(applied, tx);
    }
}

pub(crate) fn bar<S: Host>(editor: &Editor, palette: Palette, size: f32) -> El {
    let find = &editor.find;
    let count = if find.error.is_some() {
        "Invalid pattern".to_owned()
    } else if let Some(current) = find.current(editor.document.selection()) {
        format!("{}/{}", current + 1, find.matches.len())
    } else if find.matches.is_empty() {
        "0 matches".to_owned()
    } else {
        format!("{} matches", find.matches.len())
    };
    let mut result = text(count)
        .key("match-count")
        .font(palette.ui_text(size.min(12.0)))
        .color(if find.error.is_some() {
            palette.error_text
        } else {
            palette.muted_text
        });
    if let Some(error) = &find.error {
        result = result.help(error.clone());
    }
    let search = field(&find.query, QUERY, "Find", palette, size)
        .on_edit(|state: &mut S| &mut state.editor_mut().find.query)
        .on_input_change(|state: &mut S, tx| state.editor_mut().refresh_find(tx))
        .on_input_key(field_key::<S>)
        .on_submit(|state: &mut S, tx| state.editor_mut().step_match(false, false, tx));
    let toggle = |key, label, help, selected, action: fn(&mut Editor)| {
        button::<S>(key, label, palette, size, selected, move |editor, tx| {
            action(editor);
            editor.refresh_find(tx);
        })
        .role("switch")
        .label(help)
        .checked(selected)
        .help(help)
    };
    let mut result = col()
        .key("find")
        .w_full()
        .fill(palette.chrome)
        .padding(4.0)
        .gap(4.0)
        .child(
            row()
                .key("search")
                .w_full()
                .gap(4.0)
                .center_y()
                .child(search)
                .child(toggle(
                    "find-case",
                    "Aa",
                    "Match case",
                    find.options.case_sensitive,
                    |e| e.find.options.case_sensitive ^= true,
                ))
                .child(toggle(
                    "find-word",
                    "Word",
                    "Whole word",
                    find.options.whole_word,
                    |e| e.find.options.whole_word ^= true,
                ))
                .child(toggle(
                    "find-regex",
                    ".*",
                    "Regular expression",
                    find.options.regex,
                    |e| e.find.options.regex ^= true,
                ))
                .child(result)
                .child(
                    button::<S>("find-previous", "↑", palette, size, false, |e, tx| {
                        e.step_match(true, false, tx)
                    })
                    .label("Previous match"),
                )
                .child(
                    button::<S>("find-next", "↓", palette, size, false, |e, tx| {
                        e.step_match(false, false, tx)
                    })
                    .label("Next match"),
                )
                .child(
                    button::<S>("find-close", "×", palette, size, false, Editor::close_find)
                        .label("Close find"),
                ),
        );
    if find.replacing {
        let replacement = field(&find.replacement, REPLACEMENT, "Replace", palette, size)
            .on_edit(|state: &mut S| &mut state.editor_mut().find.replacement)
            .on_input_key(field_key::<S>)
            .on_submit(|state: &mut S, tx| state.editor_mut().replace_match(false, tx));
        result = result.child(
            row()
                .key("replace")
                .w_full()
                .gap(4.0)
                .center_y()
                .child(replacement)
                .child(button::<S>(
                    "replace-current",
                    "Replace",
                    palette,
                    size,
                    false,
                    |e, tx| e.replace_match(false, tx),
                ))
                .child(button::<S>(
                    "replace-all",
                    "Replace all",
                    palette,
                    size,
                    false,
                    |e, tx| e.replace_match(true, tx),
                )),
        );
    }
    result
}

fn field(
    edit: &TextEdit,
    key: &'static str,
    label: &'static str,
    palette: Palette,
    size: f32,
) -> El {
    text_input(edit, palette.ui_text(size.min(13.0)))
        .key(key)
        .label(label)
        .placeholder(label, palette.muted_text)
        .single_line()
        .color(palette.text)
        .fill(palette.canvas)
        .border(1.0, palette.divider)
        .focus_ring(1.0, -1.0, palette.accent)
        .caret(palette.text, palette.selected)
        .grow(1.0)
        .min_w(60.0)
        .h(24.0)
        .pt(3.0)
        .px(6.0)
}

fn field_key<S: Host>(state: &mut S, event: &rune::KeyEvent, tx: &mut Tx) -> bool {
    match event.key {
        Key::Escape => state.editor_mut().close_find(tx),
        Key::Enter if event.mods.shift => state.editor_mut().step_match(true, false, tx),
        _ => return false,
    }
    true
}

fn button<S: Host>(
    key: &'static str,
    label: &'static str,
    palette: Palette,
    size: f32,
    selected: bool,
    action: impl Fn(&mut Editor, &mut Tx) + 'static,
) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .h(24.0)
        .px(5.0)
        .center()
        .fill(if selected {
            palette.selected
        } else {
            palette.canvas
        })
        .hover(if selected {
            palette.selected_hover
        } else {
            palette.hover
        })
        .border(1.0, palette.divider)
        .focus_ring(1.0, -1.0, palette.accent)
        .on_press_tx(move |state: &mut S, tx| action(state.editor_mut(), tx))
        .child(
            text(label)
                .font(palette.control_text(size.min(13.0)))
                .color(palette.text)
                .a11y_hidden(),
        )
}
