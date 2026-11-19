mod actions;
mod completion;
mod popup;
mod response;
pub(crate) mod view;
pub(crate) mod workspace;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use fast_core::config::LspConfig;
use fast_lsp::{Client, CompileStatus, Kind, Options, Query, Status, position, types::*};
use rune::{PointerPress, TextEdit, Tx};

use crate::State;

pub(crate) use actions::{Command, invoke, key, pin, pin_dialog, request_outline};

const REFRESH: Duration = Duration::from_millis(180);
const HOVER: Duration = Duration::from_millis(450);
const HOVER_LEAVE: Duration = Duration::from_millis(220);
const ROW_HEIGHT: f32 = 23.0;

#[derive(Default)]
pub(crate) struct Language {
    pub config: LspConfig,
    connection: Option<Connection>,
    pub status: Option<Status>,
    pub compile: Option<CompileStatus>,
    pub pinned: Option<PathBuf>,
    pub message: Option<String>,
    legend: Option<SemanticTokensLegend>,
    diagnostics: Vec<Diagnostic>,
    pub(crate) outline: Vec<Entry>,
    pub(crate) problems: Vec<Entry>,
    pub(crate) result: Option<Results>,
    pub(crate) popup: Option<Popup>,
    pub(crate) menu: bool,
    pub(crate) rename: Option<TextEdit>,
    hover: Option<HoverRequest>,
    hover_dismiss: Option<f64>,
    hover_card_bounds: Option<[f32; 4]>,
    hover_card_inside: bool,
    refresh: Option<f64>,
    save: bool,
    pub(crate) workspace: workspace::Workspace,
}

struct Connection {
    client: Client,
    options: Options,
    binding: Option<Binding>,
    version: i32,
    sequence: i32,
    last_session: u64,
    buffer_epoch: u64,
    documents: HashMap<Url, OpenDocument>,
    diagnostics: HashMap<Url, Vec<Diagnostic>>,
    pending: HashMap<u64, Waiting>,
}

struct OpenDocument {
    tab: u64,
    revision: u64,
    version: i32,
}

struct Binding {
    uri: Url,
}

struct Waiting {
    kind: Kind,
    version: i32,
    caret: usize,
    hover: Option<usize>,
}

#[derive(Clone)]
pub(crate) struct Entry {
    label: String,
    detail: String,
    location: Location,
}

pub(crate) enum Results {
    Locations(&'static str, Vec<Entry>),
    Actions(Vec<CodeActionOrCommand>),
}

pub(crate) enum Popup {
    Completion {
        items: Vec<CompletionItem>,
        selected: usize,
        version: i32,
        caret: usize,
    },
    Card {
        title: &'static str,
        body: String,
        hover: Option<usize>,
    },
}

struct HoverRequest {
    byte: usize,
    deadline: Option<f64>,
    sent: bool,
}

impl Language {
    pub fn ready(&self) -> bool {
        matches!(self.status, Some(Status::Ready { .. }))
    }

    pub fn saved(&mut self) {
        self.save = true;
    }

    pub fn configure(&mut self, config: LspConfig) {
        if self.config != config {
            self.config = config;
            if self.connection.is_none() {
                self.status = None;
            }
        }
    }

    fn binding(&self) -> Option<&Binding> {
        self.connection.as_ref()?.binding.as_ref()
    }

    fn clear_document(&mut self) {
        self.diagnostics.clear();
        self.problems.clear();
        self.outline.clear();
        self.result = None;
        self.popup = None;
        self.rename = None;
        self.menu = false;
        self.hover = None;
        self.hover_dismiss = None;
        self.hover_card_bounds = None;
        self.hover_card_inside = false;
        self.compile = None;
        self.refresh = None;
    }
}

pub(crate) fn options(state: &State, parent: &Path) -> Options {
    let config = &state.language.config;
    let config_dir = state.config_path.as_deref().and_then(Path::parent);
    let resolve = |path: &Path| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            config_dir.unwrap_or(parent).join(path)
        }
    };
    let executable = if config.binary.components().count() == 1 && !config.binary.is_absolute() {
        config.binary.clone()
    } else {
        resolve(&config.binary)
    };
    let root = state
        .compiler
        .root
        .as_deref()
        .or(config.root.as_deref())
        .map(resolve)
        .or_else(|| {
            state
                .workbench
                .project
                .root
                .clone()
                .filter(|root| parent.starts_with(root))
        })
        .unwrap_or_else(|| {
            state
                .language
                .connection
                .as_ref()
                .map(|conn| &conn.options.root)
                .filter(|root| parent.starts_with(root))
                .map_or_else(|| parent.to_owned(), PathBuf::clone)
        });
    let mut compiler = state.compiler.clone();
    compiler.root = None;
    compiler.font_paths = compiler
        .font_paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .collect();
    Options {
        executable,
        root,
        formatter: config.formatter.as_str(),
        compiler,
    }
}

fn typst(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("typ"))
}

fn bind(state: &mut State) -> bool {
    let active = state.files.path.as_deref().filter(|path| typst(path));
    let anchor = active.or_else(|| {
        state
            .files
            .parked_documents()
            .map(|(_, path, _)| path)
            .find(|path| typst(path))
    });
    let Some(parent) = anchor
        .filter(|_| state.language.config.enabled)
        .and_then(Path::parent)
    else {
        let connected = state.language.connection.take().is_some();
        let showed_status = state.language.status.take().is_some();
        if connected || showed_status {
            state.editor.track_edits(false);
            state.editor.set_problems(Vec::new());
            state.editor.set_semantic_marks(Vec::new());
            state.editor.set_inline_hints(Vec::new());
            state.editor.set_folds(Vec::new());
            state.language.clear_document();
            state.language.pinned = None;
            return true;
        }
        return false;
    };
    let expected = options(state, parent);
    if state
        .language
        .connection
        .as_ref()
        .is_none_or(|conn| conn.options != expected)
    {
        let Some(waker) = state.waker.clone() else {
            return false;
        };
        if state.language.connection.is_none()
            && matches!(state.language.status, Some(Status::Unavailable(_)))
        {
            return false;
        }
        state.language.connection = None;
        state.language.clear_document();
        state.language.pinned = None;
        state.language.legend = None;
        state.language.message = None;
        match Client::start(expected.clone(), move || waker()) {
            Ok(client) => {
                state.language.status = Some(Status::Starting);
                state.language.connection = Some(Connection {
                    client,
                    options: expected,
                    binding: None,
                    version: 0,
                    sequence: 0,
                    last_session: u64::MAX,
                    buffer_epoch: u64::MAX,
                    documents: HashMap::new(),
                    diagnostics: HashMap::new(),
                    pending: HashMap::new(),
                });
            }
            Err(error) => {
                state.language.status = Some(Status::Unavailable(error.to_string()));
                return true;
            }
        }
    }
    let Some(connection) = &mut state.language.connection else {
        return false;
    };
    let active = active.and_then(|path| Url::from_file_path(path).ok());
    let active_changed = connection.last_session != state.files.session
        || connection.binding.as_ref().map(|binding| &binding.uri) != active.as_ref();
    if !active_changed && connection.buffer_epoch == state.files.buffer_epoch {
        return false;
    }
    connection.last_session = state.files.session;
    connection.buffer_epoch = state.files.buffer_epoch;
    let parked = state
        .files
        .parked_documents()
        .filter(|(_, path, _)| typst(path) && path.starts_with(&connection.options.root))
        .filter_map(|(tab, path, document)| Some((Url::from_file_path(path).ok()?, tab, document)))
        .collect::<Vec<_>>();
    let documents = connection
        .documents
        .keys()
        .filter(|uri| {
            active.as_ref() != Some(*uri) && !parked.iter().any(|(parked, _, _)| parked == *uri)
        })
        .cloned()
        .collect::<Vec<_>>();
    for uri in documents {
        connection.client.close(uri.clone());
        connection.documents.remove(&uri);
        connection.diagnostics.remove(&uri);
        state
            .language
            .problems
            .retain(|entry| entry.location.uri != uri);
    }
    for (uri, tab, document) in parked {
        connection.publish(uri, tab, document);
    }
    if !active_changed {
        return true;
    }
    for id in connection.pending.drain().map(|(id, _)| id) {
        connection.client.cancel(id);
    }
    connection.binding = active.map(|uri| {
        connection.version =
            connection.publish(uri.clone(), state.files.active, state.editor.document());
        Binding { uri }
    });
    let diagnostics = connection
        .binding
        .as_ref()
        .and_then(|binding| connection.diagnostics.get(&binding.uri))
        .cloned()
        .unwrap_or_default();
    let tracked = connection.binding.is_some();
    let problems = std::mem::take(&mut state.language.problems);
    let compile = state.language.compile.take();
    state.language.clear_document();
    state.language.problems = problems;
    state.language.compile = compile;
    state.editor.track_edits(tracked);
    state.editor.set_semantic_marks(Vec::new());
    state.editor.set_inline_hints(Vec::new());
    state.editor.set_folds(Vec::new());
    response::show_diagnostics(state, diagnostics);
    state.language.refresh = tracked.then_some(0.0);
    true
}

impl Connection {
    fn publish(&mut self, uri: Url, tab: u64, document: &fast_core::document::Document) -> i32 {
        let old = self.documents.get(&uri);
        let revision = document.revision();
        if let Some(old) = old
            && old.tab == tab
            && old.revision == revision
        {
            return old.version;
        }
        self.sequence += 1;
        let version = self.sequence;
        if old.is_some() {
            self.client.replace(uri.clone(), version, document);
        } else {
            self.client.open(uri.clone(), version, document);
        }
        self.documents.insert(
            uri,
            OpenDocument {
                tab,
                revision,
                version,
            },
        );
        version
    }
}

fn sync(state: &mut State) -> Option<String> {
    let edits = state.editor.take_edits();
    let save = std::mem::take(&mut state.language.save);
    let connection = state.language.connection.as_mut()?;
    let binding = connection.binding.as_ref()?;
    let document = state.editor.document();
    let trigger = if let [edit] = edits.as_slice()
        && edit.text.len() <= 4
        && let Some(ch) = edit.text.chars().next()
        && (ch.is_alphanumeric() || "_#.(@</\",:".contains(ch))
    {
        Some(ch.to_string())
    } else if !edits.is_empty()
        && document.selections().len() == 1
        && edits.iter().all(|edit| edit.text.chars().nth(1).is_none())
        && completion::quoted_content(document, document.selection().head).is_some()
    {
        Some(String::new())
    } else {
        None
    };
    if !edits.is_empty() {
        state.language.popup = None;
        state.language.hover = None;
        state.language.hover_dismiss = None;
        state.language.hover_card_bounds = None;
        state.language.hover_card_inside = false;
        if matches!(state.language.result, Some(Results::Actions(_))) {
            state.language.result = None;
        }
        connection.sequence += 1;
        connection.version = connection.sequence;
        if let Some(open) = connection.documents.get_mut(&binding.uri) {
            open.revision = document.revision();
            open.version = connection.version;
        }
        connection
            .client
            .change(binding.uri.clone(), connection.version, edits, document);
        state.language.refresh = Some(0.0);
    }
    if save {
        connection.client.save(binding.uri.clone());
    }
    trigger
}

pub(crate) fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = workspace::poll(state, tx);
    changed |= bind(state);
    let trigger = sync(state);
    let events = state
        .language
        .connection
        .as_ref()
        .map(|connection| connection.client.events().collect::<Vec<_>>())
        .unwrap_or_default();
    for event in events {
        changed |= response::handle(state, event, tx);
    }
    if let Some(trigger) =
        trigger.filter(|_| state.language.ready() && tx.focused() == Some(fast_editor::VIEWPORT))
    {
        if matches!(trigger.as_str(), "(" | ",") {
            actions::invoke(state, Command::Signature, tx);
        } else {
            actions::complete(state, Some(trigger));
        }
        changed = true;
    }
    if let Some(scheduled) = state.language.refresh {
        if scheduled == 0.0 {
            state.language.refresh = Some(now + REFRESH.as_secs_f64());
            tx.wake_after(REFRESH);
        } else if now >= scheduled {
            state.language.refresh = None;
            actions::refresh(state);
        }
    }
    if !state.language.pointer_hover_allowed() {
        state.language.hover = None;
    }
    if let Some(hover) = &mut state.language.hover
        && !hover.sent
    {
        if let Some(deadline) = hover.deadline {
            if now >= deadline {
                hover.sent = true;
                let byte = hover.byte;
                actions::hover(state, byte, true);
            }
        } else {
            hover.deadline = Some(now + HOVER.as_secs_f64());
            tx.wake_after(HOVER);
        }
    }
    if let Some(dismiss) = state.language.hover_dismiss {
        if dismiss == 0.0 {
            state.language.hover_dismiss = Some(now + HOVER_LEAVE.as_secs_f64());
            tx.wake_after(HOVER_LEAVE);
        } else if now >= dismiss {
            state.language.hover_dismiss = None;
            if state.language.has_hover_card() {
                state.language.popup = None;
                state.language.hover_card_bounds = None;
                state.language.hover_card_inside = false;
                changed = true;
            }
        }
    }
    if changed {
        tx.instant();
    }
    changed
}

pub(crate) fn hover(
    state: &mut State,
    byte: Option<usize>,
    pointer: Option<PointerPress>,
    tx: &mut Tx,
) {
    tx.live_only();
    if !state.language.pointer_hover_allowed() {
        state.language.hover = None;
        return;
    }
    let language = &mut state.language;
    // Rune sends pointer moves to editor rows underneath an overlay as well.
    if language.has_hover_card()
        && match pointer {
            Some(point) => language.hover_card_bounds.is_some_and(|[x, y, w, h]| {
                (x..x + w).contains(&point.x) && (y..y + h).contains(&point.y)
            }),
            None => language.hover_card_inside,
        }
    {
        return;
    }
    if let Some(byte) = byte
        && matches!(language.popup, Some(Popup::Card { hover: Some(shown), .. }) if byte == shown)
    {
        language.hover = Some(HoverRequest {
            byte,
            deadline: None,
            sent: true,
        });
        language.hover_dismiss = None;
        return;
    }
    if language.hover.as_ref().map(|hover| hover.byte) != byte {
        language.hover = byte.map(|byte| HoverRequest {
            byte,
            deadline: None,
            sent: false,
        });
        if byte.is_some() {
            tx.wake_after(Duration::ZERO);
        }
    }
    language.schedule_hover_dismiss(tx);
}

fn keep_hover(state: &mut State, hovered: bool, tx: &mut Tx) {
    let language = &mut state.language;
    language.hover_card_inside = hovered;
    if hovered {
        language.hover = None;
        language.hover_dismiss = None;
    } else if let Some(shown) = language.hover.as_ref().map(|hover| hover.byte)
        && matches!(language.popup, Some(Popup::Card { hover: Some(byte), .. }) if shown == byte)
    {
        language.hover_dismiss = None;
    } else {
        language.schedule_hover_dismiss(tx);
    }
    tx.live_only();
}

impl Language {
    pub fn hover_anchor(&self) -> Option<usize> {
        match self.popup {
            Some(Popup::Card { hover, .. }) => hover,
            _ => None,
        }
    }

    fn pointer_hover_allowed(&self) -> bool {
        self.ready()
            && !self.menu
            && self.rename.is_none()
            && !matches!(self.popup, Some(Popup::Completion { .. }))
    }

    fn has_hover_card(&self) -> bool {
        matches!(self.popup, Some(Popup::Card { title: "Hover", .. }))
    }

    fn schedule_hover_dismiss(&mut self, tx: &mut Tx) {
        if self.has_hover_card() && self.hover_dismiss.is_none() {
            self.hover_dismiss = Some(0.0);
            tx.wake_after(Duration::ZERO);
        }
    }
}

pub(crate) fn request(state: &mut State, query: Query, hover: Option<usize>) -> bool {
    let Some(connection) = &mut state.language.connection else {
        return false;
    };
    let kind = query.kind();
    if !matches!(kind, Kind::Execute | Kind::Export) {
        let superseded: Vec<_> = connection
            .pending
            .iter()
            .filter(|(_, waiting)| waiting.kind == kind)
            .map(|(&id, _)| id)
            .collect();
        for id in superseded {
            connection.pending.remove(&id);
            connection.client.cancel(id);
        }
    }
    let Some(id) = connection.client.request(query) else {
        return false;
    };
    connection.pending.insert(
        id,
        Waiting {
            kind,
            version: connection.version,
            caret: state.editor.document().selection().head,
            hover,
        },
    );
    true
}

fn identifier(state: &State) -> Option<TextDocumentIdentifier> {
    Some(TextDocumentIdentifier::new(
        state.language.binding()?.uri.clone(),
    ))
}

fn at(state: &State, byte: usize) -> Option<TextDocumentPositionParams> {
    Some(TextDocumentPositionParams::new(
        identifier(state)?,
        position(state.editor.document(), byte)?,
    ))
}
