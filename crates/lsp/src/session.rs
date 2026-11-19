use std::{collections::HashMap, io, process::Stdio, time::Duration};

use lsp_types::{InitializeResult, PositionEncodingKind, SemanticTokensServerCapabilities, Url};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::{Child, ChildStdin, Command as Process},
    sync::mpsc,
    time::{Instant, sleep, sleep_until, timeout},
};

use crate::{
    Command, Event, Events, Options, Query, Status,
    protocol::{self, Kind},
    replica::Replica,
};

const HANDSHAKE: Duration = Duration::from_secs(12);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const BUFFER_CHANGES: Duration = Duration::from_millis(16);
const PIN_RESTORE: u64 = 0x7fff_fffe;
const SHUTDOWN: u64 = 0x7fff_ffff;

pub(crate) async fn supervise(
    options: Options,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: Events,
) {
    let mut replica = Replica::default();
    let mut delay = Duration::from_millis(200);
    let mut args = vec![
        "--root".to_owned(),
        options.root.to_string_lossy().into_owned(),
    ];
    args.extend(
        options
            .compiler
            .inputs
            .iter()
            .map(|(key, value)| format!("--input={key}={value}")),
    );
    let configuration = json!({
        "compileStatus": "enable",
        "formatterMode": options.formatter,
        "fontPaths": options.compiler.font_paths,
        "systemFonts": options.compiler.system_fonts,
        "typstExtraArgs": args,
        "semanticTokens": "enable",
        "projectResolution": "singleFile"
    });
    loop {
        events.emit(Event::Status(Status::Starting));
        let result = Process::new(&options.executable)
            .arg("lsp")
            .current_dir(&options.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        match result {
            Ok(mut child) => {
                let started = Instant::now();
                let result = session(
                    &options,
                    &configuration,
                    &mut child,
                    &mut commands,
                    &mut replica,
                    &events,
                )
                .await;
                let exit = child.try_wait();
                if !matches!(&exit, Ok(Some(_))) {
                    drop(child.kill().await);
                }
                match result {
                    Ok(()) => return,
                    Err(error) => {
                        if started.elapsed() > Duration::from_secs(15) {
                            delay = Duration::from_millis(200);
                        }
                        let message = match exit {
                            Ok(Some(exit)) => format!("Tinymist exited ({exit}). {error}"),
                            _ => error,
                        };
                        events.emit(Event::Status(Status::Restarting(message)));
                    }
                }
            }
            Err(error) => {
                events.emit(Event::Status(Status::Unavailable(format!(
                    "Couldn't start {} in {}. {error}",
                    options.executable.display(),
                    options.root.display()
                ))));
                delay = Duration::from_secs(4);
            }
        }
        let deadline = sleep(delay);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                command = commands.recv() => match command {
                    None => return,
                    Some(Command::Ask(id, _)) => events.emit(Event::Response { id, reply: Err("Tinymist is restarting".into()) }),
                    Some(command) => { replica.notification(command, false); }
                },
            }
        }
        delay = (delay * 2).min(Duration::from_secs(4));
    }
}

struct Io {
    stdin: ChildStdin,
    messages: mpsc::Receiver<io::Result<Value>>,
    stdout: tokio::task::JoinHandle<()>,
    stderr: tokio::task::JoinHandle<()>,
}

impl Io {
    fn connect(child: &mut Child) -> Result<Self, String> {
        let stdin = child.stdin.take().ok_or("Tinymist stdin is unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Tinymist stdout is unavailable")?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or("Tinymist stderr is unavailable")?;
        let (send, messages) = mpsc::channel(64);
        let stdout = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                let frame = protocol::read(&mut reader).await;
                let eof = frame.is_err();
                if send.send(frame).await.is_err() || eof {
                    return;
                }
            }
        });
        // Consume server logs so a full stderr pipe cannot stop requests.
        let stderr = tokio::spawn(async move {
            let mut buffer = [0; 4096];
            while matches!(stderr.read(&mut buffer).await, Ok(size) if size > 0) {}
        });
        Ok(Self {
            stdin,
            messages,
            stdout,
            stderr,
        })
    }

    async fn send(&mut self, message: Value) -> Result<(), String> {
        timeout(
            Duration::from_secs(3),
            protocol::write(&mut self.stdin, &message),
        )
        .await
        .map_err(|_| "Tinymist stopped reading stdin".to_owned())?
        .map_err(|error| error.to_string())
    }

    async fn next(&mut self) -> Result<Value, String> {
        self.messages
            .recv()
            .await
            .ok_or("Tinymist stdout reader stopped".to_owned())?
            .map_err(|error| error.to_string())
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        self.stdout.abort();
        self.stderr.abort();
    }
}

struct Pending {
    kind: Kind,
    pin: Option<Option<String>>,
    deadline: Instant,
}

async fn session(
    options: &Options,
    config: &Value,
    child: &mut Child,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    replica: &mut Replica,
    events: &Events,
) -> Result<(), String> {
    let pid = child.id().ok_or("Tinymist process is missing")?;
    let mut io = Io::connect(child)?;
    let initialized = timeout(HANDSHAKE, initialize(&mut io, options, config))
        .await
        .map_err(|_| "Tinymist did not initialize in 12 seconds".to_owned())??;
    if let Some(provider) = initialized.capabilities.semantic_tokens_provider {
        let legend = match provider {
            SemanticTokensServerCapabilities::SemanticTokensOptions(options) => options.legend,
            SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(options) => {
                options.semantic_tokens_options.legend
            }
        };
        events.emit(Event::Legend(legend));
    }
    for message in replica.reopen() {
        io.send(message).await?;
    }
    if let Some(path) = replica.pinned.clone() {
        io.send(Query::pin(Some(path)).encode(PIN_RESTORE)).await?;
    }
    events.emit(Event::Status(Status::Ready { pid }));
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut flush_at = None;
    loop {
        let deadline = pending
            .values()
            .map(|request| request.deadline)
            .chain(flush_at)
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            result = io.next() => {
                let message = result?;
                if let Some(method) = message.get("method").and_then(Value::as_str) {
                    if let Some(id) = message.get("id") {
                        io.send(server_answer(id, method, message.get("params"), config)).await?;
                    } else {
                        notification(method, message.get("params").cloned().unwrap_or_default(), &options.root, events);
                    }
                } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    if let Some(waiting) = pending.remove(&id) {
                        let reply = match protocol::response_error(&message) {
                            Some(error) => Err(error),
                            None => waiting.kind.decode(message.get("result").cloned().unwrap_or_default()),
                        };
                        if reply.is_ok() && let Some(pin) = waiting.pin {
                            events.emit(Event::Pinned(pin.as_ref().map(std::path::PathBuf::from)));
                            replica.pinned = pin;
                        }
                        events.emit(Event::Response { id, reply });
                    } else if id == PIN_RESTORE && let Some(error) = protocol::response_error(&message) {
                        events.emit(Event::Notice(format!("Couldn't restore the pinned main file. {error}")));
                    }
                }
            }
            command = commands.recv() => {
                let Some(command) = command else {
                    shutdown(&mut io).await;
                    return Ok(());
                };
                if matches!(&command, Command::Change {..}) {
                    replica.notification(command, true);
                    flush_at.get_or_insert_with(|| Instant::now() + BUFFER_CHANGES);
                    continue;
                }
                flush(&mut io, replica).await?;
                flush_at = None;
                match command {
                    Command::Ask(id, query) => {
                        let mut pin = query.pin_target();
                        let query = if let Some(Some(path)) = &mut pin {
                            let resolved = std::fs::canonicalize(path.as_str())
                                .map_err(|error| format!("Couldn't open the main file. {error}"))
                                .and_then(|path| path.into_os_string().into_string().map_err(|_| "The main file path is not valid UTF-8".to_owned()));
                            match resolved {
                                Ok(resolved) => {
                                    *path = resolved.clone();
                                    Query::pin(Some(resolved))
                                }
                                Err(error) => {
                                    events.emit(Event::Response { id, reply: Err(error) });
                                    continue;
                                }
                            }
                        } else { query };
                        let request = Pending { kind: query.kind(), pin, deadline: Instant::now() + REPLY_TIMEOUT };
                        io.send(query.encode(id)).await?;
                        pending.insert(id, request);
                    }
                    Command::Cancel(id) if pending.remove(&id).is_some() => {
                        io.send(protocol::notice("$/cancelRequest", json!({"id": id}))).await?;
                    }
                    command => {
                        if let Some(message) = replica.notification(command, true) {
                            io.send(message).await?;
                        }
                    }
                }
            }
            _ = sleep_until(deadline) => {
                let now = Instant::now();
                if flush_at.is_some_and(|time| time <= now) {
                    flush(&mut io, replica).await?;
                    flush_at = None;
                }
                let expired: Vec<_> = pending.iter().filter(|(_, request)| request.deadline <= now).map(|(&id, _)| id).collect();
                for id in expired {
                    pending.remove(&id);
                    io.send(protocol::notice("$/cancelRequest", json!({"id": id}))).await?;
                    events.emit(Event::Response { id, reply: Err("Tinymist request timed out".into()) });
                }
            }
        }
    }
}

async fn flush(io: &mut Io, replica: &mut Replica) -> Result<(), String> {
    for message in replica.drain() {
        io.send(message).await?;
    }
    Ok(())
}

async fn initialize(
    io: &mut Io,
    options: &Options,
    config: &Value,
) -> Result<InitializeResult, String> {
    let root =
        Url::from_directory_path(&options.root).map_err(|()| "Invalid Tinymist workspace path")?;
    let mut options_map = config.clone();
    options_map["rootPath"] = json!(
        options
            .root
            .to_str()
            .ok_or("Tinymist requires a UTF-8 workspace path")?
    );
    io.send(protocol::request(0, "initialize", json!({
        "processId": std::process::id(),
        "clientInfo": {"name":"Fast","version":env!("CARGO_PKG_VERSION")},
        "rootUri": root,
        "workspaceFolders":[{"uri":root,"name":options.root.file_name().unwrap_or(options.root.as_os_str()).to_string_lossy()}],
        "initializationOptions": options_map,
        "capabilities": {
            "general":{"positionEncodings":["utf-16"]},
            "workspace":{
                "configuration":true,
                "workspaceFolders":true,
                "workspaceEdit":{"documentChanges":true,"resourceOperations":["create","rename"]}
            },
            "textDocument":{
                "completion":{"completionItem":{"snippetSupport":true,"documentationFormat":["markdown","plaintext"]}},
                "hover":{"contentFormat":["markdown","plaintext"]},
                "signatureHelp":{"signatureInformation":{"documentationFormat":["markdown","plaintext"],"parameterInformation":{"labelOffsetSupport":true}}},
                "documentSymbol":{"hierarchicalDocumentSymbolSupport":true},
                "definition":{"linkSupport":true},
                "publishDiagnostics":{"versionSupport":true,"relatedInformation":true},
                "codeAction":{"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["","quickfix","refactor","source"]}}},
                "semanticTokens":{"requests":{"full":true},"tokenTypes":[],"tokenModifiers":[],"formats":["relative"]},
                "foldingRange":{"lineFoldingOnly":true}
            }
        }
    }))).await?;
    loop {
        let message = io.next().await?;
        if let (Some(id), Some(method)) = (
            message.get("id"),
            message.get("method").and_then(Value::as_str),
        ) {
            io.send(server_answer(id, method, message.get("params"), config))
                .await?;
        } else if message.get("id").and_then(Value::as_u64) == Some(0) {
            if let Some(error) = protocol::response_error(&message) {
                return Err(error);
            }
            let init: InitializeResult =
                serde_json::from_value(message.get("result").cloned().unwrap_or_default())
                    .map_err(|error| format!("Invalid initialize reply: {error}"))?;
            if init
                .capabilities
                .position_encoding
                .as_ref()
                .is_some_and(|encoding| *encoding != PositionEncodingKind::UTF16)
            {
                return Err("Tinymist did not negotiate UTF-16 positions".into());
            }
            io.send(protocol::notice("initialized", json!({}))).await?;
            return Ok(init);
        }
    }
}

fn server_answer(id: &Value, method: &str, params: Option<&Value>, config: &Value) -> Value {
    let result = match method {
        "workspace/configuration" => Value::Array(
            params
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|item| match item.get("section").and_then(Value::as_str) {
                    Some(section) if section != "tinymist" => config
                        .get(section.trim_start_matches("tinymist."))
                        .cloned()
                        .unwrap_or_default(),
                    _ => config.clone(),
                })
                .collect(),
        ),
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create" => Value::Null,
        "workspace/applyEdit" => {
            json!({"applied":false,"failureReason":"Fast applies edits initiated from the editor"})
        }
        _ => {
            return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unsupported client request"}});
        }
    };
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn notification(method: &str, params: Value, root: &std::path::Path, events: &Events) {
    let parsed = match method {
        "textDocument/publishDiagnostics" => serde_json::from_value(params).map(Event::Diagnostics),
        "tinymist/compileStatus" => {
            serde_json::from_value::<crate::CompileStatus>(params).map(|mut status| {
                // Tinymist sends a virtual path rooted at the Typst workspace.
                if !status.path.as_os_str().is_empty() && !status.path.starts_with(root) {
                    let relative = status.path.strip_prefix("/").unwrap_or(&status.path);
                    if !relative.is_absolute() {
                        status.path = root.join(relative);
                    }
                }
                Event::Compile(status)
            })
        }
        "window/showMessage" => {
            if let Some(message) = params.get("message").and_then(Value::as_str) {
                events.emit(Event::Notice(message.to_owned()));
            }
            return;
        }
        _ => return,
    };
    events.emit(
        parsed.unwrap_or_else(|error| {
            Event::Notice(format!("Invalid {method} notification: {error}"))
        }),
    );
}

async fn shutdown(io: &mut Io) {
    let _result = timeout(Duration::from_millis(250), async {
        io.send(protocol::request(SHUTDOWN, "shutdown", Value::Null))
            .await?;
        while io.next().await?.get("id").and_then(Value::as_u64) != Some(SHUTDOWN) {}
        io.send(protocol::notice("exit", Value::Null)).await
    })
    .await;
}
