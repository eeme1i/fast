pub(crate) mod assets;
pub(crate) mod external;
pub(crate) mod session;

use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use fast_core::{document::Document, file};
use fast_editor::{Editor, VIEWPORT};
use rune::{FileDialogOptions, Latest, Tx, Waker};

use crate::State;

type Loaded = Result<(PathBuf, Editor), String>;
type Saved = Result<(PathBuf, Document), String>;

pub(crate) struct Files {
    pub path: Option<PathBuf>,
    pub session: u64,
    pub buffer_epoch: u64,
    pub active: u64,
    next_tab: u64,
    saved_revision: Option<u64>,
    disk: Option<Arc<Document>>,
    parked: BTreeMap<u64, Tab>,
    opening: Option<Target>,
    external: external::Watch,
    assets: assets::Paste,
    saving: Option<u64>,
    open_job: Latest<Loaded>,
    save_job: Latest<Saved>,
    pub confirm_close: Option<u64>,
    pub message: Option<String>,
}

struct Tab {
    path: Option<PathBuf>,
    editor: Editor,
    saved_revision: Option<u64>,
    disk: Option<Arc<Document>>,
}

pub(crate) struct TabInfo<'a> {
    pub id: u64,
    pub path: Option<&'a Path>,
    pub dirty: bool,
}

enum Target {
    File,
    Language(fast_lsp::types::Range),
    Bytes(Range<usize>),
}

impl Default for Files {
    fn default() -> Self {
        Self {
            path: None,
            session: 0,
            buffer_epoch: 0,
            active: 0,
            next_tab: 0,
            saved_revision: Some(0),
            disk: None,
            parked: BTreeMap::new(),
            opening: None,
            external: external::Watch::default(),
            assets: assets::Paste::default(),
            saving: None,
            open_job: Latest::default(),
            save_job: Latest::default(),
            confirm_close: None,
            message: None,
        }
    }
}

impl Files {
    pub fn setup(&self, waker: Waker) {
        self.open_job.set_waker(waker.clone());
        self.save_job.set_waker(waker.clone());
        self.external.setup(waker.clone());
        self.assets.setup(waker);
    }

    pub fn dirty(&self, editor: &Editor) -> bool {
        self.saved_revision != Some(editor.document().revision())
    }

    pub fn comparing(&self) -> bool {
        self.external.comparing()
    }

    pub fn busy(&self) -> bool {
        self.opening.is_some() || self.saving.is_some()
    }

    pub fn tabs<'a>(&'a self, editor: &'a Editor) -> Vec<TabInfo<'a>> {
        let mut tabs = self
            .parked
            .iter()
            .map(|(&id, tab)| TabInfo {
                id,
                path: tab.path.as_deref(),
                dirty: tab.saved_revision != Some(tab.editor.document().revision()),
            })
            .collect::<Vec<_>>();
        tabs.push(TabInfo {
            id: self.active,
            path: self.path.as_deref(),
            dirty: self.dirty(editor),
        });
        tabs.sort_by_key(|tab| tab.id);
        tabs
    }

    fn parked_at(&self, path: &Path) -> Option<u64> {
        self.parked
            .iter()
            .find_map(|(&id, tab)| (tab.path.as_deref() == Some(path)).then_some(id))
    }

    pub fn other_buffers(&self) -> impl Iterator<Item = (&Path, &Document)> {
        self.parked
            .values()
            .filter_map(|tab| Some((tab.path.as_deref()?, tab.editor.document())))
    }

    pub fn parked_documents(&self) -> impl Iterator<Item = (u64, &Path, &Document)> {
        self.parked
            .iter()
            .filter_map(|(&id, tab)| Some((id, tab.path.as_deref()?, tab.editor.document())))
    }

    pub fn dirty_buffers(&self) -> impl Iterator<Item = (&Path, &Document)> {
        self.parked.values().filter_map(|tab| {
            (tab.saved_revision != Some(tab.editor.document().revision()))
                .then_some((tab.path.as_deref()?, tab.editor.document()))
        })
    }

    pub fn status(&self) -> Option<&str> {
        if self.opening.is_some() {
            Some("Opening…")
        } else if self.saving.is_some() {
            Some("Saving…")
        } else {
            self.message.as_deref()
        }
    }

    pub fn workspace_saved(&mut self, path: PathBuf, editor: &Editor) {
        self.path = Some(path);
        self.saved_revision = Some(editor.document().revision());
        self.disk = Some(disk_snapshot(editor));
        self.external.invalidate(self.active);
        self.message = Some("Workspace edits saved".into());
    }

    pub fn refresh_tabs(
        &mut self,
        refreshed: Vec<(PathBuf, PathBuf, Editor)>,
        waker: Option<&Waker>,
    ) {
        for (previous, path, editor) in refreshed {
            if let Some(id) = self.parked_at(&previous) {
                if let Some(waker) = waker {
                    editor.setup(waker.clone());
                }
                self.parked.insert(
                    id,
                    Tab {
                        path: Some(path),
                        saved_revision: Some(editor.document().revision()),
                        disk: Some(disk_snapshot(&editor)),
                        editor,
                    },
                );
                self.buffer_epoch += 1;
                self.external.invalidate(id);
            }
        }
    }
}

pub(crate) fn load(path: PathBuf) -> Loaded {
    let file =
        file::open(&path).map_err(|error| format!("Couldn't open {}. {error}", path.display()))?;
    Ok((file.path, Editor::new(Document::from_rope(file.text))))
}

fn disk_snapshot(editor: &Editor) -> Arc<Document> {
    Arc::new(Document::from_rope(editor.document().snapshot()))
}

fn new_tab(state: &mut State, path: Option<PathBuf>, editor: Editor) -> Tab {
    if let Some(waker) = &state.waker {
        editor.setup(waker.clone());
    }
    Tab {
        disk: path.as_ref().map(|_| disk_snapshot(&editor)),
        path,
        saved_revision: Some(editor.document().revision()),
        editor,
    }
}

fn swap(state: &mut State, id: u64, tab: Tab, keep: bool) {
    let previous_id = std::mem::replace(&mut state.files.active, id);
    state.editor.track_edits(false);
    let previous = Tab {
        path: std::mem::replace(&mut state.files.path, tab.path),
        saved_revision: std::mem::replace(&mut state.files.saved_revision, tab.saved_revision),
        editor: std::mem::replace(&mut state.editor, tab.editor),
        disk: std::mem::replace(&mut state.files.disk, tab.disk),
    };
    if keep {
        state.files.parked.insert(previous_id, previous);
    } else {
        state.files.external.invalidate(previous_id);
    }
    state.files.external.rescan();
    state.files.session += 1;
    state.files.message = None;
}

fn allocate(files: &mut Files) -> u64 {
    files.next_tab += 1;
    files.next_tab
}

fn focus(state: &mut State, tx: &mut Tx) {
    state.workbench.show_editor();
    tx.focus(VIEWPORT, false);
    tx.instant();
}

fn reveal(state: &mut State, tx: &mut Tx) {
    focus(state, tx);
    state.editor.reveal_tab(tx);
}

pub(crate) fn create(state: &mut State, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    let id = allocate(&mut state.files);
    let tab = new_tab(state, None, Editor::default());
    swap(state, id, tab, true);
    reveal(state, tx);
    tx.scroll_into_view_in("tab-bar", format!("tab-{id}"));
}

pub(crate) fn activate(state: &mut State, id: u64, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    if let Some(tab) = state.files.parked.remove(&id) {
        swap(state, id, tab, true);
        reveal(state, tx);
    } else if state.files.active == id {
        reveal(state, tx);
    }
}

pub(crate) fn close(state: &mut State, id: u64, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    if state
        .files
        .tabs(&state.editor)
        .iter()
        .any(|tab| tab.id == id && tab.dirty)
    {
        state.files.confirm_close = Some(id);
        tx.instant();
    } else {
        discard(state, id, tx);
    }
}

pub(crate) fn discard(state: &mut State, id: u64, tx: &mut Tx) {
    state.files.confirm_close = None;
    if id == state.files.active {
        let next = state
            .files
            .parked
            .range(..id)
            .next_back()
            .or_else(|| state.files.parked.first_key_value())
            .map(|(&id, _)| id);
        if let Some(next) = next
            && let Some(tab) = state.files.parked.remove(&next)
        {
            swap(state, next, tab, false);
        } else {
            let next = allocate(&mut state.files);
            let blank = new_tab(state, None, Editor::default());
            swap(state, next, blank, false);
        }
        reveal(state, tx);
    } else {
        if state.files.parked.remove(&id).is_some() {
            state.files.buffer_epoch += 1;
            state.files.external.invalidate(id);
        }
        focus(state, tx);
    }
}

pub(crate) fn initial_open(state: &mut State, path: PathBuf) {
    match load(path) {
        Ok((path, editor)) => {
            let tab = new_tab(state, Some(path), editor);
            swap(state, 0, tab, false);
        }
        Err(error) => state.files.message = Some(error),
    }
}

pub(crate) fn open(state: &mut State, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    let options = FileDialogOptions {
        title: "Open Typst file".to_owned(),
        directory: state
            .files
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_owned),
        ..Default::default()
    }
    .filter("Typst documents", &["typ"]);
    tx.open_file(options, |state: &mut State, result, tx| {
        match result {
            Ok(Some(path)) => open_path(state, path, tx),
            Ok(None) => focus(state, tx),
            Err(error) => state.files.message = Some(error.to_string()),
        }
        tx.instant();
    });
}

pub(crate) fn open_path(state: &mut State, path: PathBuf, tx: &mut Tx) {
    navigate_to(state, path, Target::File, tx);
}

pub(crate) fn navigate(
    state: &mut State,
    path: PathBuf,
    target: fast_lsp::types::Range,
    tx: &mut Tx,
) {
    navigate_to(state, path, Target::Language(target), tx);
}

pub(crate) fn jump(state: &mut State, path: PathBuf, range: Range<usize>, tx: &mut Tx) {
    navigate_to(state, path, Target::Bytes(range), tx);
}

fn navigate_to(state: &mut State, path: PathBuf, target: Target, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    if state.files.path.as_ref() == Some(&path) {
        select_target(state, target, tx);
    } else if let Some(id) = state.files.parked_at(&path) {
        activate(state, id, tx);
        select_target(state, target, tx);
    } else {
        state.files.opening = Some(target);
        state.files.message = None;
        state.files.open_job.request(path, load);
        tx.instant();
    }
}

fn select_target(state: &mut State, target: Target, tx: &mut Tx) {
    let target = match target {
        Target::Language(range) => fast_lsp::range(state.editor.document(), range),
        Target::Bytes(range) => Some(range),
        Target::File => None,
    };
    if let Some(target) = target {
        state.editor.jump(target, tx);
        state.editor.center_caret(tx);
    }
    focus(state, tx);
}

pub(crate) fn save(state: &mut State, save_as: bool, tx: &mut Tx) {
    if state.files.busy() || state.language.workspace.showing() {
        return;
    }
    if !save_as && let Some(path) = state.files.path.clone() {
        save_to(state, path);
        return;
    }
    let path = state.files.path.as_deref();
    let session = state.files.session;
    let options = FileDialogOptions {
        title: "Save Typst file".to_owned(),
        directory: path
            .and_then(Path::parent)
            .map(Path::to_owned)
            .or_else(|| state.workbench.project.root.clone()),
        file_name: Some(path.and_then(Path::file_name).map_or_else(
            || "Untitled.typ".into(),
            |name| name.to_string_lossy().into_owned(),
        )),
        ..Default::default()
    }
    .filter("Typst documents", &["typ"]);
    tx.save_file(options, move |state: &mut State, result, tx| {
        match result {
            Ok(Some(path)) if session == state.files.session && !state.files.busy() => {
                save_to(state, path)
            }
            Ok(Some(_)) => {
                state.files.message = Some("The active tab changed before saving".into())
            }
            Ok(None) => {}
            Err(error) => state.files.message = Some(error.to_string()),
        }
        focus(state, tx);
    });
}

fn save_to(state: &mut State, path: PathBuf) {
    let (revision, snapshot) = state.editor.save_snapshot();
    let other_paths = state
        .files
        .other_buffers()
        .map(|(path, _)| path.to_owned())
        .collect::<Vec<_>>();
    let current = state.files.path.clone();
    let expected = state.files.disk.clone();
    state.files.saving = Some(revision);
    state.files.message = None;
    state.files.save_job.request(
        (path, snapshot, other_paths, current, expected),
        |(path, snapshot, others, current, expected)| {
            let resolved = path.canonicalize().ok();
            if others.contains(&path) || resolved.as_ref().is_some_and(|path| others.contains(path))
            {
                return Err(format!("{} is already open in another tab", path.display()));
            }
            let text = snapshot.snapshot();
            let saved = if let Some(current) =
                current.filter(|current| *current == path || resolved.as_ref() == Some(current))
            {
                file::save_if_unchanged(
                    &current,
                    &text,
                    expected.as_ref().map(|doc| doc.snapshot()).as_ref(),
                )
            } else {
                file::save(&path, &text)
            };
            saved
                .map(|path| (path, snapshot))
                .map_err(|error| format!("Couldn't save {}. {error}", path.display()))
        },
    );
}

pub(crate) fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let changed = external::poll(state, now, tx) | assets::poll(state, tx);
    if let Some(result) = state.files.open_job.take() {
        let target = state.files.opening.take().unwrap_or(Target::File);
        match result {
            Ok((path, editor)) => {
                if let Some(id) = state.files.parked_at(&path) {
                    activate(state, id, tx);
                } else if state.files.path.as_ref() != Some(&path) {
                    let keep = state.files.path.is_some() || state.files.dirty(&state.editor);
                    let id = if keep {
                        allocate(&mut state.files)
                    } else {
                        state.files.active
                    };
                    let tab = new_tab(state, Some(path), editor);
                    swap(state, id, tab, keep);
                    reveal(state, tx);
                }
                select_target(state, target, tx);
            }
            Err(error) => state.files.message = Some(error),
        }
        tx.instant();
        return true;
    }
    if let Some(result) = state.files.save_job.take() {
        state.files.external.rescan();
        tx.wake_after(std::time::Duration::ZERO);
        let saved = state.files.saving.take();
        match result {
            Ok((path, snapshot)) => {
                state.files.path = Some(path);
                if let Some(revision) = saved {
                    state.files.saved_revision = Some(revision);
                }
                state.files.disk = Some(Arc::new(Document::from_rope(snapshot.snapshot())));
                state.files.external.invalidate(state.files.active);
                state.files.message = Some("Saved".into());
                state.language.saved();
                crate::preview::on_saved(state, snapshot, tx);
                state.workbench.project.refresh();
            }
            Err(error) => state.files.message = Some(error),
        }
        tx.instant();
        return true;
    }
    changed
}
