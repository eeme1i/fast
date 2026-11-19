#![cfg(unix)]

use std::{
    error::Error,
    path::PathBuf,
    thread::sleep,
    time::{Duration, Instant},
};

use fast_core::document::{Document, Selection};
use fast_lsp::types::*;
use fast_lsp::{Client, Event, Options, Query, Reply, Status, position};

fn wait<T>(client: &Client, mut find: impl FnMut(Event) -> Option<T>) -> Result<T, Box<dyn Error>> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        for event in client.events() {
            if let Event::Status(Status::Unavailable(error)) = event {
                return Err(error.into());
            }
            if let Some(value) = find(event) {
                return Ok(value);
            }
        }
        sleep(Duration::from_millis(10));
    }
    Err("timed out waiting for Tinymist".into())
}

fn answer(client: &Client, id: u64) -> Result<Reply, Box<dyn Error>> {
    Ok(wait(client, |event| match event {
        Event::Response {
            id: received,
            reply,
        } if received == id => Some(reply),
        _ => None,
    })??)
}

fn suggestions(
    client: &mut Client,
    uri: &Url,
    pos: Position,
) -> Result<Vec<CompletionItem>, Box<dyn Error>> {
    let query = Query::Completion(CompletionParams {
        text_document_position: TextDocumentPositionParams::new(
            TextDocumentIdentifier::new(uri.clone()),
            pos,
        ),
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
        context: None,
    });
    let id = client.request(query).ok_or("completion channel closed")?;
    match answer(client, id)? {
        Reply::Completion(Some(CompletionResponse::Array(items))) => Ok(items),
        Reply::Completion(Some(CompletionResponse::List(list))) => Ok(list.items),
        _ => Err("expected completions".into()),
    }
}

#[test]
#[ignore = "requires tinymist on PATH or FAST_TEST_TINYMIST, and a Unix process signal"]
fn completes_reports_diagnostics_pins_and_recovers_after_sigkill() -> Result<(), Box<dyn Error>> {
    let folder = tempfile::tempdir()?;
    let root = folder.path().canonicalize()?;
    let main = root.join("main.typ");
    let chapter = root.join("chapter.typ");
    let text = "#let item = 1\n🌱\n#ite";
    std::fs::write(&main, "#set text(size: 11pt)\n#include \"chapter.typ\"\n")?;
    std::fs::write(&chapter, text)?;
    let mut document = Document::new(text);
    let uri = Url::from_file_path(&chapter).map_err(|()| "invalid chapter URI")?;
    let mut client = Client::start(
        Options {
            compiler: Default::default(),
            executable: std::env::var_os("FAST_TEST_TINYMIST")
                .map(PathBuf::from)
                .unwrap_or_else(|| "tinymist".into()),
            root,
            formatter: "typstyle",
        },
        || {},
    )?;
    assert!(client.open(uri.clone(), 1, &document));
    let pid = wait(&client, |event| match event {
        Event::Status(Status::Ready { pid }) => Some(pid),
        _ => None,
    })
    .map_err(|error| format!("initialize: {error}"))?;
    assert!(
        suggestions(&mut client, &uri, Position::new(2, 4))?
            .iter()
            .any(|item| item.label == "item")
    );

    let id = client
        .request(Query::pin(Some(main.to_string_lossy().into_owned())))
        .ok_or("pin channel closed")?;
    assert!(matches!(answer(&client, id)?, Reply::Execute(_)));
    document.select(Selection::caret(document.len()));
    let edits = document.insert("m\n#not_defined_anywhere", false);
    assert!(client.change(uri.clone(), 2, edits, &document));
    let diagnostics = wait(&client, |event| match event {
        Event::Diagnostics(params)
            if params.uri == uri
                && params
                    .diagnostics
                    .iter()
                    .any(|diag| diag.message.contains("not_defined_anywhere")) =>
        {
            Some(params)
        }
        _ => None,
    })
    .map_err(|error| format!("diagnostics: {error}"))?;
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.range.start.line == 3 && diagnostic.message.contains("not_defined_anywhere")
    }));

    // Kill only the process the client just spawned for this fixture.
    assert!(
        std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()?
            .success()
    );
    let restarted = wait(&client, |event| match event {
        Event::Status(Status::Ready { pid: new }) if pid != new => Some(new),
        _ => None,
    })
    .map_err(|error| format!("restart: {error}"))?;
    assert_ne!(pid, restarted);
    let last = document.len();
    document.select(Selection::caret(last));
    let edits = document.insert("\n", false);
    assert!(client.change(uri.clone(), 3, edits, &document));
    let pinned = wait(&client, |event| match event {
        Event::Compile(status) if status.path == main => Some(status),
        _ => None,
    })
    .map_err(|error| format!("restored pin: {error}"))?;
    assert_eq!(pinned.path, main);
    let position = position(&document, text.len()).ok_or("invalid completion position")?;
    assert!(
        suggestions(&mut client, &uri, position)?
            .iter()
            .any(|item| item.label == "item")
    );
    assert!(client.save(uri.clone()));
    assert!(client.close(uri));
    Ok(())
}
