//! Tinymist language server integration.

mod protocol;
mod replica;
mod session;
mod snippet;
pub mod workspace;

use std::{
    io,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use fast_core::document::{Document, Edit};
use ropey::Rope;
use tokio::sync::mpsc as async_queue;

pub use lsp_types as types;
pub use protocol::{Kind, Query, Reply};
pub use replica::{byte, position, range};
pub use snippet::{Snippet, snippet};
use types::{PublishDiagnosticsParams, SemanticTokensLegend, Url};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub formatter: &'static str,
    pub compiler: fast_core::config::CompilerConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready { pid: u32 },
    Restarting(String),
    Unavailable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Compilation {
    Compiling,
    CompileSuccess,
    CompileError,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileStatus {
    pub status: Compilation,
    pub path: PathBuf,
    #[serde(default)]
    pub page_count: u32,
}

pub enum Event {
    Status(Status),
    Diagnostics(PublishDiagnosticsParams),
    Compile(CompileStatus),
    Pinned(Option<PathBuf>),
    Legend(SemanticTokensLegend),
    Response {
        id: u64,
        reply: Result<Reply, String>,
    },
    Notice(String),
}

struct Events {
    sender: mpsc::Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Events {
    fn emit(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            (self.wake)();
        }
    }
}

enum Command {
    Open {
        uri: Url,
        version: i32,
        text: Rope,
    },
    Change {
        uri: Url,
        version: i32,
        edits: Vec<Edit>,
        text: Rope,
    },
    Save(Url),
    Close(Url),
    Ask(u64, Query),
    Cancel(u64),
    FilesChanged(Vec<types::FileEvent>),
}

pub struct Client {
    commands: async_queue::UnboundedSender<Command>,
    events: mpsc::Receiver<Event>,
    next_id: u64,
}

impl Client {
    pub fn start(options: Options, wake: impl Fn() + Send + Sync + 'static) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (commands, receiver) = async_queue::unbounded_channel();
        let (sender, events) = mpsc::channel();
        thread::Builder::new()
            .name("fast-tinymist".into())
            .spawn(move || {
                runtime.block_on(session::supervise(
                    options,
                    receiver,
                    Events {
                        sender,
                        wake: Arc::new(wake),
                    },
                ));
            })?;
        Ok(Self {
            commands,
            events,
            next_id: 1,
        })
    }

    pub fn open(&self, uri: Url, version: i32, document: &Document) -> bool {
        self.commands
            .send(Command::Open {
                uri,
                version,
                text: document.snapshot(),
            })
            .is_ok()
    }

    pub fn change(&self, uri: Url, version: i32, edits: Vec<Edit>, document: &Document) -> bool {
        self.commands
            .send(Command::Change {
                uri,
                version,
                edits,
                text: document.snapshot(),
            })
            .is_ok()
    }

    pub fn replace(&self, uri: Url, version: i32, document: &Document) -> bool {
        self.change(uri, version, Vec::new(), document)
    }

    pub fn save(&self, uri: Url) -> bool {
        self.commands.send(Command::Save(uri)).is_ok()
    }

    pub fn close(&self, uri: Url) -> bool {
        self.commands.send(Command::Close(uri)).is_ok()
    }

    pub fn files_changed(&self, changes: Vec<types::FileEvent>) {
        drop(self.commands.send(Command::FilesChanged(changes)));
    }

    pub fn request(&mut self, query: Query) -> Option<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.commands
            .send(Command::Ask(id, query))
            .ok()
            .map(|()| id)
    }

    pub fn cancel(&self, id: u64) {
        // The request disappears with a stopped server, too.
        drop(self.commands.send(Command::Cancel(id)));
    }

    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        self.events.try_iter()
    }
}
