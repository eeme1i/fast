//! Rune editor view.

mod find;
mod input;
mod language;
mod selection;
mod syntax;
mod view;

use std::{ops::Range, time::Duration};

use fast_core::document::{Document, Edit, Selection};
use rune::{Live, PointerPress, Tx, Waker};

use find::Find;
pub use input::EditCommand;
pub use language::{HoverTarget, InlineHint, Problem, SemanticKind, SemanticMark};
use syntax::Syntax;
pub use view::view;

pub const VIEWPORT: &str = "editor";
pub const HOVER_ANCHOR: &str = "hover-anchor";
const BLINK: Duration = Duration::from_millis(550);

pub trait Host: 'static {
    fn editor(&self) -> &Editor;
    fn editor_mut(&mut self) -> &mut Editor;
    fn editor_key(&mut self, _: &rune::KeyEvent, _: &mut Tx) -> bool {
        false
    }
    fn editor_hover(&mut self, _: Option<HoverTarget>, _: Option<PointerPress>, _: &mut Tx) {}
    fn editor_hover_anchor(&self) -> Option<usize> {
        None
    }
}

pub struct Editor {
    document: Document,
    syntax: Syntax,
    find: Find,
    focused: bool,
    blink_on: bool,
    next_blink: Option<f64>,
    preferred_column: Option<usize>,
    drag_origin: Option<selection::Gesture>,
    scroll: Live<[f32; 2]>,
    hover_line: Option<usize>,
    preedit: String,
    visible: Range<usize>,
    mounted: bool,
    language: language::Language,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(Document::default())
    }
}

impl Editor {
    pub fn new(document: Document) -> Self {
        let syntax = Syntax::new(&document);
        Self {
            document,
            syntax,
            find: Find::default(),
            focused: false,
            blink_on: true,
            next_blink: None,
            preferred_column: None,
            drag_origin: None,
            scroll: Live::new([0.0; 2]),
            hover_line: None,
            preedit: String::new(),
            visible: 0..0,
            mounted: false,
            language: language::Language::default(),
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn reveal_tab(&mut self, tx: &mut Tx) {
        self.preedit.clear();
        self.drag_origin = None;
        self.hover_line = None;
        self.mark_activity(tx);
        let [x, y] = self.scroll();
        tx.scroll_to_instant(VIEWPORT, x, y);
    }

    pub fn center_caret(&self, tx: &mut Tx) {
        let line = self.document.line_at(self.document.selection().head);
        tx.scroll_center_in_instant(VIEWPORT, view::line_key(line));
    }

    pub fn scroll(&self) -> [f32; 2] {
        self.scroll.get()
    }

    pub fn restore_view(&mut self, selections: Vec<Selection>, scroll: [f32; 2]) {
        self.document.set_selections(selections);
        self.scroll.set(scroll.map(|offset| {
            if offset.is_finite() {
                offset.max(0.0)
            } else {
                0.0
            }
        }));
    }

    pub fn reload(&mut self, disk: &Document, tx: &mut Tx) {
        let selections = self.document.selections().to_vec();
        let scroll = self.scroll();
        let edits = self.document.replace_ranges(vec![Edit {
            range: 0..self.document.len(),
            text: disk.snapshot().to_string(),
        }]);
        self.restore_view(selections, scroll);
        self.changed(edits, tx);
        self.reveal_tab(tx);
    }

    pub fn setup(&self, waker: Waker) {
        self.syntax.set_waker(waker);
    }

    pub fn save_snapshot(&mut self) -> (u64, Document) {
        self.document.end_typing();
        (
            self.document.revision(),
            Document::from_rope(self.document.snapshot()),
        )
    }

    fn changed(&mut self, edits: Vec<Edit>, tx: &mut Tx) {
        if !edits.is_empty() {
            if self.language.track {
                self.language.edits.extend(edits.iter().cloned());
                tx.wake_after(Duration::ZERO);
            }
            for edit in &edits {
                self.language
                    .problems
                    .retain_mut(|problem| language::adjust(&mut problem.range, edit));
                self.language
                    .snippets
                    .retain_mut(|stop| language::adjust(stop, edit));
            }
            self.language.semantic.clear();
            self.language.hints.clear();
            self.language.folds.clear();
            if !self.language.collapsed.is_empty() {
                self.language.collapsed.clear();
                self.language.fold_version += 1;
            }
            self.syntax.update(&self.document, edits);
            if self.find.open {
                self.find.scan(&self.document);
            }
        }
        self.reveal(tx);
    }

    fn reveal(&mut self, tx: &mut Tx) {
        self.mark_activity(tx);
        let line = self.document.line_at(self.document.selection().head);
        if !self.visible.contains(&line) {
            tx.scroll_into_view_in(VIEWPORT, view::line_key(line));
        }
    }

    fn mark_activity(&mut self, tx: &mut Tx) {
        self.blink_on = true;
        self.next_blink = None;
        tx.wake_after(BLINK);
        tx.instant();
    }

    pub fn ime(&mut self, event: &rune::Ime, tx: &mut Tx) {
        match event {
            rune::Ime::Preedit(text, ..) => {
                self.preedit.clone_from(text);
                if !text.is_empty() {
                    self.reveal(tx);
                }
            }
            rune::Ime::Commit(text) => {
                self.preedit.clear();
                self.insert_plain(text, tx);
            }
            rune::Ime::Select(range) => {
                self.document.select(Selection {
                    anchor: range.start,
                    head: range.end,
                });
                self.reveal(tx);
            }
            // Scrolling the virtual caret row offscreen can disable the input method.
            rune::Ime::Disabled => self.preedit.clear(),
            rune::Ime::Enabled => {}
        }
    }

    pub fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        let parsed = self.syntax.poll();
        if parsed {
            tx.instant();
        }
        let focused = tx.focused() == Some(VIEWPORT);
        let changed = focused != self.focused;
        self.focused = focused;
        if !focused {
            self.next_blink = None;
            self.preedit.clear();
            return changed || parsed;
        }
        let due = self.next_blink.unwrap_or(now + BLINK.as_secs_f64());
        let toggled = now >= due;
        if self.next_blink.is_none() || toggled {
            self.blink_on = !toggled || !self.blink_on;
            self.next_blink = Some(now + BLINK.as_secs_f64());
            tx.wake_after(BLINK);
        }
        if changed || toggled {
            tx.instant();
        }
        changed || toggled || parsed
    }
}
