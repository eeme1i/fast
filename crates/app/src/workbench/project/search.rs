mod scan;

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use fast_theme::tok::{Palette, row_height};
use rune::{El, Key, Latest, Span, TextEdit, Tx, Waker, col, rich, row, text};

use super::message;
use crate::{
    State, files,
    workbench::{Tool, item, search_field},
};

use scan::{Found, MAX_FILE_BYTES, Results, Source, find};

pub(super) struct Search {
    field: TextEdit,
    running: bool,
    revision: Option<(u64, u64, u64)>,
    results: Results,
    selected: usize,
    error: Option<String>,
    cancelled: Option<Arc<AtomicBool>>,
    job: Latest<Found>,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            field: TextEdit::default(),
            running: false,
            revision: None,
            results: Results::default(),
            selected: 0,
            error: None,
            cancelled: None,
            job: Latest::default().debounce(Duration::from_millis(140)),
        }
    }
}

impl Search {
    pub(super) fn setup(&self, waker: Waker) {
        self.job.set_waker(waker);
    }

    pub(super) fn cancel(&mut self) {
        if let Some(cancelled) = self.cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.job.cancel();
        self.running = false;
    }

    fn has_query(&self) -> bool {
        !self.field.text.is_empty()
    }
}

pub(super) fn poll(state: &mut State) -> bool {
    let mut changed = false;
    let search = &state.workbench.project.search;
    let revision = search_revision(state);
    if search.has_query()
        && search.revision != Some(revision)
        && state.workbench.is_open(Tool::Search)
    {
        rerun_search(state);
        changed = true;
    }
    let search = &mut state.workbench.project.search;
    if let Some(result) = search.job.take() {
        search.cancelled = None;
        search.running = false;
        match result {
            Ok(results) => {
                search.results = results;
                search.selected = search
                    .selected
                    .min(search.results.matches.len().saturating_sub(1));
            }
            Err(error) => search.error = Some(error),
        }
        changed = true;
    }
    changed
}

pub(crate) fn open_search(state: &mut State, tx: &mut Tx) {
    state.workbench.open(Tool::Search);
    if !state.workbench.project.search.field.text.is_empty() {
        rerun_search(state);
    }
    tx.focus("workspace-search-input", true);
    tx.instant();
}

pub(super) fn rerun_search(state: &mut State) {
    let revision = search_revision(state);
    let open = state.workbench.is_open(Tool::Search);
    let project = &mut state.workbench.project;
    let search = &mut project.search;
    search.cancel();
    search.error = None;
    search.results = Results::default();
    search.selected = 0;
    search.revision = Some(revision);
    if !open || !search.has_query() {
        return;
    }
    let (Some(root), Some(index)) = (project.root.clone(), project.index.clone()) else {
        return;
    };
    let mut buffers: BTreeMap<_, _> = state
        .files
        .dirty_buffers()
        .map(|(path, doc)| (path.to_owned(), doc.snapshot()))
        .collect();
    if let Some(path) = &state.files.path {
        buffers.insert(path.clone(), state.editor.document().snapshot());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    search.cancelled = Some(cancelled.clone());
    search.running = true;
    search.job.request(
        (root, index, search.field.text.clone(), buffers, cancelled),
        |(root, index, query, buffers, cancelled)| {
            let sources = index.files().map(|path| root.join(path));
            find(
                sources,
                &query,
                |path| match buffers.get(path) {
                    Some(text) if text.len_bytes() > MAX_FILE_BYTES => Source::Large,
                    Some(text) => Source::Buffer(text.to_string()),
                    None => Source::Disk,
                },
                &cancelled,
            )
        },
    );
}

fn search_revision(state: &State) -> (u64, u64, u64) {
    (
        state.files.session,
        state.editor.document().revision(),
        state.files.buffer_epoch,
    )
}

pub(in crate::workbench) fn search(state: &State, palette: Palette) -> El {
    let project = &state.workbench.project;
    let search = &project.search;
    let field = search_field(
        palette,
        &search.field,
        "workspace-search-input",
        "Find text in project",
    )
    .on_edit(|state: &mut State| &mut state.workbench.project.search.field)
    .on_input_change(|state: &mut State, tx| {
        rerun_search(state);
        tx.instant();
    })
    .on_input_key(|state: &mut State, event, tx| match event.key {
        Key::Up | Key::Down => {
            let search = &mut state.workbench.project.search;
            if !search.results.matches.is_empty() {
                let delta = if event.key == Key::Up { -1 } else { 1 };
                search.selected = (search.selected as isize + delta)
                    .rem_euclid(search.results.matches.len() as isize)
                    as usize;
                tx.scroll_into_view_in(
                    "workspace-results-scroll",
                    format!("workspace-match-{}", search.selected),
                );
                tx.instant();
            }
            true
        }
        Key::Escape => {
            state.workbench.close(Tool::Search, tx);
            true
        }
        _ => false,
    })
    .on_submit(open_selected);
    let mut controls = row().padding(4.0).center_y().gap(6.0).child(field);
    if let Some(skipped) = skipped_files(&search.results) {
        let count = search.results.skipped_binary + search.results.skipped_large;
        controls = controls.child(
            text(format!("{count} skipped"))
                .key("workspace-search-skipped")
                .font(palette.ui_text(11.0))
                .color(palette.muted_text)
                .label(&skipped)
                .help(skipped),
        );
    }
    let mut body = col()
        .key("workspace-search")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(controls);
    if let Some(message_text) = search.error.as_deref().or(project.error.as_deref()) {
        return body.child(message(palette, message_text));
    }
    if search.running || (search.has_query() && project.indexing) {
        return body.child(message(palette, "Searching…"));
    }
    if project.root.is_none() {
        return body.child(message(palette, "Open a file or choose a project folder."));
    }
    if search.results.matches.is_empty() {
        return if search.has_query() {
            body.child(message(palette, "No matches"))
        } else {
            body
        };
    }

    let mut rows = col().w_full();
    let height = row_height(state.ui.density);
    let mut last = None;
    for (i, found) in search.results.matches.iter().enumerate() {
        if last != Some(&found.path) {
            let display = project
                .root
                .as_ref()
                .and_then(|root| found.path.strip_prefix(root).ok())
                .unwrap_or(&found.path);
            rows = rows.child(message(palette, &display.to_string_lossy()));
            last = Some(&found.path);
        }
        rows = rows.child(
            item(
                palette,
                format!("workspace-match-{i}"),
                &format!("{}  {}", found.line, found.snippet),
                height,
                i == search.selected,
                move |state, tx| {
                    state.workbench.project.search.selected = i;
                    open_selected(state, tx);
                },
            )
            .on_hover_change(move |state: &mut State, hovered, tx| {
                if hovered && state.workbench.project.search.selected != i {
                    state.workbench.project.search.selected = i;
                    tx.instant();
                } else {
                    tx.live_only();
                }
            })
            .pl(14.0),
        );
    }
    body = body.child(
        col()
            .key("workspace-results-scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(rows),
    );
    if let Some(found) = search.results.matches.get(search.selected) {
        body = body.child(
            rich(
                &found.snippet,
                vec![Span::new(found.highlight.start, found.highlight.end).color(palette.accent)],
            )
            .key("workspace-match-preview")
            .font(palette.ui_text(12.0).mono())
            .color(palette.text)
            .fill(palette.chrome)
            .w_full()
            .px(8.0)
            .py(4.0)
            .truncate()
            .help(format!("{}:{}", found.path.display(), found.line)),
        );
    }
    body
}

fn open_selected(state: &mut State, tx: &mut Tx) {
    let search = &state.workbench.project.search;
    if let Some(found) = search.results.matches.get(search.selected) {
        files::jump(state, found.path.clone(), found.range.clone(), tx);
    }
}

fn skipped_files(results: &Results) -> Option<String> {
    let mut reasons = Vec::new();
    if results.skipped_binary > 0 {
        reasons.push(format!("{} binary or media", results.skipped_binary));
    }
    if results.skipped_large > 0 {
        reasons.push(format!(
            "{} over {} MiB",
            results.skipped_large,
            MAX_FILE_BYTES / (1024 * 1024)
        ));
    }
    (!reasons.is_empty()).then(|| format!("Files skipped · {}", reasons.join(", ")))
}
