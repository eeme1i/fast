use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use fast_core::{
    document::{Document, Selection},
    file,
};
use fast_editor::Editor;
use rune::{Latest, Tx, Waker};
use serde::{Deserialize, Serialize};

use super::{Tab, allocate, disk_snapshot, load, reveal, swap};
use crate::{State, workbench::WorkbenchSession};

#[derive(Default)]
pub(crate) struct Store {
    path: Option<PathBuf>,
    read: Latest<Result<Option<Loaded>, String>>,
    write: Latest<Result<(), String>>,
    committed: Arc<Mutex<u64>>,
    serial: u64,
    last: Option<(View, u64)>,
    due: Option<(f64, f64)>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    #[serde(flatten)]
    view: View,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    drafts: Vec<Draft>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct View {
    root: Option<PathBuf>,
    pinned: Option<PathBuf>,
    active: usize,
    workbench: WorkbenchSession,
    tabs: Vec<TabView>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct TabView {
    path: Option<PathBuf>,
    scroll: [f32; 2],
    selections: Vec<ViewSelection>,
    #[serde(skip)]
    draft_revision: Option<u64>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct ViewSelection {
    anchor: usize,
    head: usize,
}

#[derive(Serialize, Deserialize)]
struct Draft {
    tab: usize,
    text: String,
    base: Option<String>,
}

type Drafts = Vec<(usize, Arc<Document>, Option<Arc<Document>>)>;

struct Loaded {
    view: View,
    tabs: Vec<(usize, Tab)>,
    warning: Option<String>,
}

impl Store {
    pub fn setup(&mut self, config: Option<&Path>, restore: bool, waker: Waker) {
        self.read.set_waker(waker.clone());
        self.write.set_waker(waker);
        let Some(config) = config else { return };
        let Some(name) = config.file_name() else {
            return;
        };
        let mut state_file = std::ffi::OsString::from(".");
        state_file.push(name);
        state_file.push(".session.toml");
        let path = config.with_file_name(state_file);
        self.path = Some(path.clone());
        if restore {
            self.read.request(path, |path| {
                match fs::read_to_string(&path) {
                    Ok(source) => load_session(&source).map(Some),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error.to_string()),
                }
                .map_err(|error| format!("Couldn’t restore {}. {error}", path.display()))
            });
        }
    }

    fn next_write(&mut self) -> Option<(PathBuf, u64, Arc<Mutex<u64>>)> {
        let path = self.path.clone()?;
        self.serial += 1;
        Some((path, self.serial, Arc::clone(&self.committed)))
    }
}

fn load_session(source: &str) -> Result<Loaded, String> {
    let Saved { view, drafts } = toml::from_str(source).map_err(|error| format!("{error}"))?;
    let mut drafts: BTreeMap<_, _> = drafts.into_iter().map(|draft| (draft.tab, draft)).collect();
    let mut seen = BTreeSet::new();
    let mut tabs = Vec::new();
    let mut failures = Vec::new();
    for (index, view) in view.tabs.iter().enumerate() {
        let mut tab = if let Some(draft) = drafts.remove(&index) {
            Tab {
                path: view.path.clone(),
                editor: Editor::new(Document::new(&draft.text)),
                saved_revision: None,
                disk: draft.base.map(|base| Arc::new(Document::new(&base))),
            }
        } else if let Some(path) = &view.path {
            match load(path.clone()) {
                Ok((path, editor)) => Tab {
                    path: Some(path),
                    disk: Some(disk_snapshot(&editor)),
                    saved_revision: Some(editor.document().revision()),
                    editor,
                },
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            }
        } else {
            Tab {
                path: None,
                editor: Editor::default(),
                saved_revision: Some(0),
                disk: None,
            }
        };
        if let Some(path) = &tab.path
            && !seen.insert(path.clone())
        {
            failures.push(format!("Duplicate tab {}", path.display()));
            continue;
        }
        tab.editor.restore_view(
            view.selections
                .iter()
                .map(|selection| Selection {
                    anchor: selection.anchor,
                    head: selection.head,
                })
                .collect(),
            view.scroll,
        );
        tabs.push((index, tab));
    }
    Ok(Loaded {
        view,
        tabs,
        warning: (!failures.is_empty()).then(|| failures.join("\n")),
    })
}

fn restore(state: &mut State, loaded: Loaded, tx: &mut Tx) {
    let Loaded {
        view,
        tabs,
        warning,
    } = loaded;
    if let Some(root) = view.root {
        state.workbench.project.use_root(root);
    }
    state.language.pinned = view.pinned;
    let active = tabs
        .iter()
        .position(|(index, _)| *index == view.active)
        .unwrap_or(0);
    for (index, (_, tab)) in tabs.into_iter().enumerate() {
        let id = if index == 0 {
            state.files.active
        } else {
            allocate(&mut state.files)
        };
        if let Some(waker) = &state.waker {
            tab.editor.setup(waker.clone());
        }
        if index == active {
            swap(state, id, tab, false);
        } else {
            state.files.parked.insert(id, tab);
        }
    }
    reveal(state, tx);
    state
        .workbench
        .restore_session(view.workbench, &mut state.preview);
    state.session_store.error = warning;
    tx.wake_after(Duration::ZERO);
}

fn capture(state: &State) -> View {
    let files = &state.files;
    let mut entries: Vec<_> = files
        .parked
        .iter()
        .map(|(&id, tab)| {
            (
                id,
                &tab.path,
                &tab.editor,
                tab.saved_revision != Some(tab.editor.document().revision()),
            )
        })
        .collect();
    entries.push((
        files.active,
        &files.path,
        &state.editor,
        files.dirty(&state.editor),
    ));
    entries.sort_by_key(|entry| entry.0);
    let mut active = 0;
    let tabs = entries
        .into_iter()
        .enumerate()
        .map(|(index, (id, path, editor, dirty))| {
            if id == files.active {
                active = index;
            }
            TabView {
                path: path.clone(),
                scroll: editor.scroll(),
                selections: editor
                    .document()
                    .selections()
                    .iter()
                    .map(|selection| ViewSelection {
                        anchor: selection.anchor,
                        head: selection.head,
                    })
                    .collect(),
                draft_revision: dirty.then(|| editor.document().revision()),
            }
        })
        .collect();
    View {
        root: state.workbench.project.root.clone(),
        pinned: state.language.pinned.clone(),
        workbench: state.workbench.session(&state.preview),
        tabs,
        active,
    }
}

fn drafts(state: &State) -> Drafts {
    let files = &state.files;
    let mut entries: Vec<_> = files
        .parked
        .iter()
        .map(|(&id, tab)| {
            (
                id,
                &tab.editor,
                tab.saved_revision != Some(tab.editor.document().revision()),
                &tab.disk,
            )
        })
        .collect();
    entries.push((
        files.active,
        &state.editor,
        files.dirty(&state.editor),
        &files.disk,
    ));
    entries.sort_by_key(|entry| entry.0);
    entries
        .into_iter()
        .enumerate()
        .filter(|(_, entry)| entry.2)
        .map(|(index, (_, editor, _, base))| (index, disk_snapshot(editor), base.clone()))
        .collect()
}

fn write(
    path: &Path,
    serial: u64,
    committed: &Mutex<u64>,
    view: View,
    drafts: Drafts,
) -> Result<(), String> {
    let drafts = drafts
        .into_iter()
        .map(|(tab, document, base)| Draft {
            tab,
            text: document.snapshot().to_string(),
            base: base.map(|document| document.snapshot().to_string()),
        })
        .collect();
    let source = toml::to_string(&Saved { view, drafts }).map_err(|error| error.to_string())?;
    let mut committed = committed.lock().unwrap_or_else(|error| error.into_inner());
    if serial <= *committed {
        return Ok(());
    }
    let result = file::save(path, &Document::new(&source).snapshot())
        .map_err(|error| format!("Couldn’t save {}. {error}", path.display()));
    if result.is_ok() {
        *committed = serial;
    }
    result.map(|_| ())
}

pub(crate) fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    if let Some(result) = state.session_store.read.take() {
        match result {
            Ok(Some(loaded))
                if state.files.session == 0
                    && !state.files.dirty(&state.editor)
                    && !state.files.busy()
                    && state.workbench.project.root.is_none() =>
            {
                restore(state, loaded, tx);
                changed = true;
            }
            Err(error) => {
                state.session_store.last =
                    Some((capture(state), state.files.external.generation()));
                state.session_store.error = Some(error);
                changed = true;
            }
            _ => {}
        }
    }
    if let Some(Err(error)) = state.session_store.write.take() {
        state.session_store.error = Some(error);
        changed = true;
    }
    if state.session_store.path.is_none() || state.session_store.read.is_pending() {
        return changed;
    }
    let view = capture(state);
    let epoch = state.files.external.generation();
    let store = &mut state.session_store;
    if store
        .last
        .as_ref()
        .is_none_or(|(last, last_epoch)| *last != view || *last_epoch != epoch)
    {
        let first = store.due.map_or(now, |(_, first)| first);
        store.due = Some((
            if store.last.is_none() {
                now
            } else {
                now + 0.45
            },
            first,
        ));
        store.last = Some((view, epoch));
    }
    if let Some((settled, first)) = store.due {
        let deadline = settled.min(first + 5.0);
        if now >= deadline {
            store.due = None;
            if let Some((path, serial, committed)) = store.next_write() {
                let view = capture(state);
                let documents = drafts(state);
                state.session_store.write.request(
                    (path, serial, committed, view, documents),
                    |(path, serial, committed, view, documents)| {
                        write(&path, serial, &committed, view, documents)
                    },
                );
            }
        } else {
            tx.wake_at(deadline);
        }
    }
    if changed {
        tx.instant();
    }
    changed
}

pub(crate) fn flush(state: &mut State, _: &mut Tx) {
    if state.session_store.read.is_pending() {
        return;
    }
    let Some((path, serial, committed)) = state.session_store.next_write() else {
        return;
    };
    if let Err(error) = write(&path, serial, &committed, capture(state), drafts(state)) {
        eprintln!("{error}");
    }
}
