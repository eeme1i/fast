mod search;

use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

use fast_theme::tok::{Palette, row_height};
use rune::{El, FileDialogOptions, GrowthSpec, Latest, Tx, Waker, col, text};

use super::{dock::Tool, item};
use crate::{State, files};
pub(crate) use search::open_search;
pub(super) use search::search;
use search::{Search, rerun_search};

#[derive(Default)]
pub(crate) struct Project {
    pub root: Option<PathBuf>,
    pub(super) index: Option<Arc<Index>>,
    indexing: bool,
    pub error: Option<String>,
    expanded: BTreeSet<PathBuf>,
    visible: Vec<usize>,
    pub(super) version: u64,
    job: Latest<(PathBuf, Result<Index, String>)>,
    search: Search,
}

pub(super) struct Index {
    entries: Vec<Entry>,
    children: BTreeMap<PathBuf, Vec<usize>>,
}

struct Entry {
    path: PathBuf,
    key: String,
    name: String,
    depth: usize,
    directory: bool,
}

impl Project {
    pub(super) fn cancel_search(&mut self) {
        self.search.cancel();
    }

    pub fn setup(&self, waker: Waker) {
        self.job.set_waker(waker.clone());
        self.search.setup(waker);
    }

    pub(crate) fn use_root(&mut self, root: PathBuf) {
        self.expanded.clear();
        self.visible.clear();
        self.index = None;
        self.root = Some(root);
        self.refresh();
    }

    pub fn refresh(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.indexing = true;
        self.error = None;
        self.job.request(root, |root| {
            let result = Index::read(&root);
            (root, result)
        });
    }

    fn rebuild_visible(&mut self) {
        self.visible.clear();
        if let Some(index) = &self.index {
            index.flatten(Path::new(""), &self.expanded, &mut self.visible);
        }
        self.version += 1;
    }

    fn toggle(&mut self, path: PathBuf) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        self.rebuild_visible();
    }
}

impl Index {
    pub(super) fn files(&self) -> impl Iterator<Item = &Path> {
        self.entries
            .iter()
            .filter(|entry| !entry.directory)
            .map(|entry| entry.path.as_path())
    }

    fn read(root: &Path) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut children = BTreeMap::<PathBuf, Vec<usize>>::new();
        let walker = ignore::WalkBuilder::new(root)
            .require_git(false)
            .parents(false)
            .git_global(false)
            .sort_by_file_name(|a, b| a.cmp(b))
            .build();
        for entry in walker {
            let entry =
                entry.map_err(|error| format!("Couldn't read {}. {error}", root.display()))?;
            if let Some(error) = entry.error() {
                return Err(format!("Couldn't read project ignore rules. {error}"));
            }
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if entry.depth() == 0 || !(kind.is_dir() || kind.is_file()) {
                continue;
            }
            let Ok(path) = entry.path().strip_prefix(root) else {
                continue;
            };
            let Some(parent) = path.parent() else {
                continue;
            };
            children
                .entry(parent.to_owned())
                .or_default()
                .push(entries.len());
            entries.push(Entry {
                path: path.to_owned(),
                key: entry_key(path),
                name: entry.file_name().to_string_lossy().into_owned(),
                depth: entry.depth() - 1,
                directory: kind.is_dir(),
            });
        }
        for siblings in children.values_mut() {
            siblings.sort_by_key(|&i| !entries[i].directory);
        }
        Ok(Self { entries, children })
    }

    fn flatten(&self, path: &Path, expanded: &BTreeSet<PathBuf>, visible: &mut Vec<usize>) {
        for &i in self.children.get(path).into_iter().flatten() {
            visible.push(i);
            let entry = &self.entries[i];
            if entry.directory && expanded.contains(&entry.path) {
                self.flatten(&entry.path, expanded, visible);
            }
        }
    }
}

fn entry_key(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("project-entry-{:016x}", hasher.finish())
}

pub(crate) fn choose_root(state: &mut State, tx: &mut Tx) {
    tx.open_folder(
        FileDialogOptions {
            title: "Open project folder".into(),
            directory: state.workbench.project.root.clone(),
            ..Default::default()
        },
        |state: &mut State, result, tx| {
            match result {
                Ok(Some(root)) => {
                    state.workbench.project.use_root(root);
                    state.workbench.open(Tool::Project);
                    rerun_search(state);
                }
                Ok(None) => {}
                Err(error) => state.workbench.project.error = Some(error.to_string()),
            }
            tx.instant();
        },
    );
}

pub(super) fn poll(state: &mut State, tx: &mut Tx) -> bool {
    let mut changed = false;
    let project = &mut state.workbench.project;
    if project.root.is_none()
        && let Some(parent) = state.files.path.as_deref().and_then(Path::parent)
    {
        let root = if state.compiler.root.is_some() || state.language.config.root.is_some() {
            crate::language::options(state, parent).root
        } else {
            parent.to_owned()
        };
        state.workbench.project.use_root(root);
        changed = true;
    }
    let project = &mut state.workbench.project;
    if let Some((root, result)) = project.job.take()
        && project.root.as_ref() == Some(&root)
    {
        project.indexing = false;
        match result {
            Ok(index) => {
                project.index = Some(Arc::new(index));
                project.rebuild_visible();
            }
            Err(error) => project.error = Some(error),
        }
        rerun_search(state);
        changed = true;
    }
    changed |= search::poll(state);
    if changed {
        tx.instant();
    }
    changed
}

pub(super) fn tree(state: &State, palette: Palette) -> El {
    let project = &state.workbench.project;
    let mut body = col().key("project-body").grow(1.0).min_h(0.0).w_full();
    let Some(root) = &project.root else {
        return body.child(message(
            palette,
            project
                .error
                .as_deref()
                .unwrap_or("Open a file or choose a project folder."),
        ));
    };
    body = body.child(
        text(root.file_name().map_or_else(
            || root.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        ))
        .key("project-root")
        .font(palette.control_text(12.0))
        .color(palette.text)
        .px(8.0)
        .py(4.0)
        .w_full()
        .truncate()
        .help(root.display().to_string()),
    );
    if let Some(error) = &project.error {
        body = body.child(message(palette, error));
    }
    if project.indexing && project.index.is_none() {
        return body.child(message(palette, "Reading project files…"));
    }
    if project.visible.is_empty() && project.error.is_none() {
        return body.child(message(palette, "No project files"));
    }
    let height = row_height(state.ui.density);
    body.child(
        col()
            .key("project-scroll")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("project-rows")
                    .w_full()
                    .growth(GrowthSpec::default())
                    .virtual_rows(
                        project.visible.len(),
                        height,
                        |state: &State, i| {
                            let project = &state.workbench.project;
                            project
                                .index
                                .as_ref()
                                .and_then(|index| index.entries.get(project.visible[i]))
                                .map_or_else(
                                    || i.to_string().into(),
                                    |entry| entry.key.clone().into(),
                                )
                        },
                        move |state: &State, i| tree_row(state, i, height, palette),
                    )
                    .virtual_rows_version(project.version),
            ),
    )
}

fn tree_row(state: &State, i: usize, height: f32, palette: Palette) -> El {
    let project = &state.workbench.project;
    let index = project.visible[i];
    let Some(entry) = project
        .index
        .as_ref()
        .and_then(|files| files.entries.get(index))
    else {
        return col();
    };
    let path = entry.path.clone();
    let key = entry.key.clone();
    if entry.directory {
        let expanded = project.expanded.contains(&path);
        let label = format!("{}  {}", if expanded { "▾" } else { "▸" }, entry.name);
        item(palette, key, &label, height, false, move |state, tx| {
            state.workbench.project.toggle(path.clone());
            tx.instant();
        })
        .label(&entry.name)
        .expanded(expanded)
        .pl(6.0 + 12.0 * entry.depth.min(10) as f32)
    } else {
        let Some(path) = project.root.as_ref().map(|root| root.join(path)) else {
            return col();
        };
        let selected = state.files.path.as_ref() == Some(&path);
        let open = path.clone();
        let button = item(
            palette,
            &key,
            &entry.name,
            height,
            selected,
            move |state, tx| {
                files::open_path(state, open.clone(), tx);
            },
        )
        .pl(20.0 + 12.0 * entry.depth.min(10) as f32);
        super::with_pin_menu(state, key, Some(&path), button)
    }
}

pub(super) fn message(palette: Palette, value: &str) -> El {
    text(value)
        .font(palette.ui_text(12.0))
        .color(palette.muted_text)
        .w_full()
        .px(8.0)
        .py(4.0)
        .truncate()
        .help(value)
}
