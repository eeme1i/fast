mod files;
mod keymap;
mod language;
mod preview;
mod view;
mod workbench;

use std::path::PathBuf;

use fast_core::config::{
    CompilerConfig, Config, ConfigError, ConfigFile, EditorConfig, ExportConfig, UiConfig,
};
use fast_editor::{Editor, Host, VIEWPORT};
use rune::{App, AppSpec, Chrome, Key, KeyEvent, PointerPress, Tx, Waker};

#[derive(Default)]
struct State {
    ui: UiConfig,
    editor_config: EditorConfig,
    config_path: Option<PathBuf>,
    config_loaded: bool,
    config_error: Option<String>,
    watcher_error: Option<String>,
    editor: Editor,
    keymap: keymap::Keymap,
    assets_dir: Option<PathBuf>,
    compiler: CompilerConfig,
    export_config: ExportConfig,
    files: files::Files,
    language: language::Language,
    preview: preview::Preview,
    settings_menu: bool,
    workbench: workbench::Workbench,
    waker: Option<Waker>,
    session_store: files::session::Store,
}

impl Host for State {
    fn editor(&self) -> &Editor {
        &self.editor
    }
    fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }
    fn editor_key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        language::key(self, event, tx) || keymap::key(self, event, tx, true)
    }
    fn editor_hover(&mut self, byte: Option<usize>, pointer: Option<PointerPress>, tx: &mut Tx) {
        language::hover(self, byte, pointer, tx);
    }
    fn editor_hover_anchor(&self) -> Option<usize> {
        self.language.hover_anchor()
    }
}

impl State {
    fn apply_config(&mut self, config: Result<Option<Config>, ConfigError>) {
        match config {
            Ok(file_config) => {
                let loaded = file_config.is_some();
                let config = file_config.unwrap_or_default();
                match keymap::Keymap::from_config(&config.keymap) {
                    Ok(keymap) => self.keymap = keymap,
                    Err(error) => {
                        self.config_error = Some(error);
                        return;
                    }
                }
                self.config_loaded = loaded;
                self.assets_dir = Some(config.assets_dir);
                if self.ui.sidebar != config.ui.sidebar {
                    self.workbench.sidebar_changed();
                }
                if self.ui.preview_dock != config.ui.preview_dock {
                    self.workbench
                        .preview_toggled(self.preview.visible, config.ui.preview_dock);
                }
                self.ui = config.ui;
                self.editor_config = config.editor;
                self.compiler = config.compiler;
                self.export_config = config.export;
                self.language.configure(config.lsp);
                self.config_error = None;
            }
            Err(error) => self.config_error = Some(error.to_string()),
        }
    }
}

fn main() {
    let app = App::new(AppSpec {
        min_size: Some((440.0, 240.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(window_key),
        ..AppSpec::new(
            "Fast — Typst IDE",
            (760.0, 460.0),
            State::default,
            view::window,
        )
    })
    .chrome(Chrome {
        full_size_content: false,
        ..Chrome::DEFAULT
    })
    .on_quit(files::session::flush)
    .on_ime(|state: &mut State, ime, tx| {
        if tx.focused() != Some(VIEWPORT) {
            return false;
        }
        state.editor.ime(ime, tx);
        true
    });

    let mut state = State::default();
    match rune::extra_args() {
        [] => {}
        [path] => files::initial_open(&mut state, PathBuf::from(path)),
        _ => state.files.message = Some("Pass one Typst file to open.".into()),
    }
    let mut watcher = None;
    match ConfigFile::locate() {
        Ok(file) => {
            state.config_path = Some(file.path().to_owned());
            let handle = app.handle();
            match file.watch(move |result| {
                handle.update(move |state, tx| {
                    state.apply_config(result);
                    tx.instant();
                });
            }) {
                Ok(config_watcher) => watcher = Some(config_watcher),
                Err(error) => state.watcher_error = Some(error.to_string()),
            }
            state.apply_config(file.load());
        }
        Err(error) => state.apply_config(Err(error)),
    }

    app.init(move || state).run();
    drop(watcher);
}

fn setup(state: &mut State, waker: Waker) {
    state.files.setup(waker.clone());
    state.editor.setup(waker.clone());
    state.language.workspace.setup(waker.clone());
    state
        .workbench
        .setup(state.config_path.as_deref(), waker.clone());
    state.session_store.setup(
        state.config_path.as_deref(),
        rune::extra_args().is_empty(),
        waker.clone(),
    );
    state.waker = Some(waker);
}

fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let files = files::poll(state, now, tx);
    let editor = state.editor.poll(now, tx);
    let lsp = language::poll(state, now, tx);
    let preview = preview::poll(state, now, tx);
    let workbench = workbench::poll(state, tx);
    let session = files::session::poll(state, now, tx);
    files || editor || lsp || preview || workbench || session
}

fn window_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if state.files.confirm_close.is_some() || state.language.workspace.showing() {
        return false;
    }
    if tx.focused() != Some(VIEWPORT) && keymap::key(state, event, tx, false) {
        return true;
    }
    match event.key {
        Key::Escape if state.editor.has_find() => state.editor.close_find(tx),
        Key::Escape if state.language.menu || state.language.popup.is_some() => {
            state.language.menu = false;
            state.language.popup = None;
            tx.focus(VIEWPORT, false);
            tx.instant();
        }
        _ => return false,
    }
    true
}
