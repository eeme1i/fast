use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use fast_preview::{Export, Format, Snapshot};
use fast_theme::tok::Palette;
use rune::{Color, El, FileDialogOptions, Tx, col, row, text};

use crate::{State, language, view::action};

use super::{Binding, Preview, start_worker};

#[derive(Default)]
pub(super) struct Controls {
    open: bool,
    pub busy: Option<Busy>,
    message: Option<String>,
    auto: Option<(Snapshot, Export)>,
}

pub(super) enum Busy {
    Native,
    Text {
        session: u64,
        main: PathBuf,
        path: PathBuf,
        markdown: bool,
        pending: bool,
    },
}

pub(crate) fn export_status(state: &State) -> Option<&str> {
    let controls = &state.preview.export;
    if controls.busy.is_some() {
        Some("Exporting…")
    } else {
        controls.message.as_deref()
    }
}

pub(crate) fn export_dialog(state: &mut State, tx: &mut Tx) {
    state.preview.export.open = true;
    tx.focus("export-pdf", false);
    tx.instant();
}

fn close(state: &mut State, tx: &mut Tx) {
    state.preview.export.open = false;
    tx.focus(fast_editor::VIEWPORT, false);
    tx.instant();
}

pub(crate) fn export_overlay(state: &State, palette: Palette) -> Option<El> {
    if !state.preview.export.open {
        return None;
    }
    let size = state.ui.font_size;
    let mut formats = col().gap(2.0);
    for row_formats in [
        [Format::Pdf, Format::Png, Format::Svg],
        [Format::Html, Format::Markdown, Format::Text],
    ] {
        let mut group = row().gap(4.0);
        for format in row_formats {
            let key = match format {
                Format::Pdf => "export-pdf",
                Format::Png => "export-png",
                Format::Svg => "export-svg",
                Format::Html => "export-html",
                Format::Markdown => "export-markdown",
                Format::Text => "export-text",
            };
            let enabled =
                !matches!(format, Format::Markdown | Format::Text) || state.language.ready();
            let control = action(
                key,
                format.label(),
                palette,
                size,
                enabled,
                move |state, tx| destination(state, format, tx),
            )
            .grow(1.0);
            group = group.child(if enabled {
                control
            } else {
                control.help("Requires a running Tinymist server")
            });
        }
        formats = formats.child(group);
    }
    Some(
        col()
            .key("export-dialog")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(Color::BLACK.alpha(0.42))
            .role("dialog")
            .label("Export document")
            .modal_tx(false, close)
            .child(
                col()
                    .w(340.0)
                    .padding(10.0)
                    .gap(8.0)
                    .fill(palette.chrome)
                    .border(1.0, palette.divider)
                    .child(
                        text("Export document")
                            .font(palette.control_text(size))
                            .color(palette.text),
                    )
                    .child(formats)
                    .child(row().justify_end().child(action(
                        "export-cancel",
                        "Cancel",
                        palette,
                        size,
                        true,
                        close,
                    ))),
            ),
    )
}

fn destination(state: &mut State, format: Format, tx: &mut Tx) {
    let Some(binding) = Binding::new(state) else {
        close(state, tx);
        return;
    };
    let suggested = binding.main.with_extension(format.extension());
    let page = state.preview.camera.current_page();
    let options = FileDialogOptions {
        title: format!("Export {}", format.label()),
        directory: suggested.parent().map(Path::to_owned),
        file_name: suggested
            .file_name()
            .map(|name| name.to_string_lossy().into_owned()),
        ..Default::default()
    }
    .filter(format.label(), &[format.extension()]);
    close(state, tx);
    tx.save_file(options, move |state: &mut State, result, tx| {
        match result {
            Ok(Some(path)) => {
                let path = if path.extension().is_none() {
                    path.with_extension(format.extension())
                } else {
                    path
                };
                if Binding::new(state).is_some_and(|current| {
                    current.session == binding.session && current.main == binding.main
                }) {
                    submit(
                        state,
                        Export {
                            format,
                            destination: path,
                            page,
                            settings: state.export_config.clone(),
                            overwrite: true,
                        },
                        tx,
                    );
                } else {
                    state.preview.export.message =
                        Some("The document changed while choosing an export location".into());
                }
            }
            Ok(None) => {}
            Err(error) => state.preview.export.message = Some(error.to_string()),
        }
        tx.focus(fast_editor::VIEWPORT, false);
        tx.instant();
    });
}

pub(crate) fn quick_export(state: &mut State, tx: &mut Tx) {
    let Some(binding) = Binding::new(state) else {
        return;
    };
    if let Some(job) = quick_job(state, &binding, false) {
        submit(state, job, tx);
    }
}

fn quick_job(state: &State, binding: &Binding, overwrite: bool) -> Option<Export> {
    let settings = state.export_config.clone();
    let name = binding.main.file_stem()?.to_str()?;
    let root = &binding.settings.root;
    let path = settings
        .quick_path
        .replace("$root", &root.to_string_lossy())
        .replace("$name", name);
    Some(Export {
        format: Format::Pdf,
        destination: root.join(path),
        page: 0,
        settings,
        overwrite,
    })
}

pub(crate) fn on_saved(state: &mut State, saved: fast_core::document::Document, tx: &mut Tx) {
    if !state.export_config.on_save {
        return;
    }
    let Some(binding) = Binding::new(state) else {
        return;
    };
    if let Some(job) = quick_job(state, &binding, true) {
        let mut snapshot = binding.snapshot(state);
        snapshot.text = saved.snapshot();
        state.preview.export.auto = Some((snapshot, job));
        tx.wake_after(Duration::ZERO);
    }
}

fn submit(state: &mut State, job: Export, tx: &mut Tx) {
    if state.preview.export.busy.is_some() {
        return;
    }
    let Some(binding) = Binding::new(state) else {
        return;
    };
    if job.destination == binding.main
        || job.destination == binding.active
        || job
            .destination
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("typ"))
    {
        state.preview.export.message =
            Some("Choose an export path outside the Typst source files".into());
        return;
    }
    if !start_worker(state) {
        state.preview.export.message = state.preview.error.clone();
        return;
    }
    state.preview.export.message = None;
    if matches!(job.format, Format::Markdown | Format::Text) {
        state.preview.export.busy = Some(Busy::Text {
            session: binding.session,
            main: binding.main,
            path: job.destination,
            markdown: job.format == Format::Markdown,
            pending: true,
        });
    } else if let Some(worker) = &state.preview.worker {
        let generation = if state.preview.binding.as_ref() == Some(&binding) {
            state.preview.generation
        } else {
            u64::MAX
        };
        worker.export(generation, binding.snapshot(state), job);
        state.preview.export.busy = Some(Busy::Native);
    }
    tx.wake_after(Duration::ZERO);
    tx.instant();
}

pub(super) fn poll(state: &mut State) -> bool {
    if state.preview.export.busy.is_none()
        && let Some((snapshot, job)) = state.preview.export.auto.take()
    {
        if job.destination == snapshot.main || job.destination == snapshot.active {
            finished(
                &mut state.preview,
                Err("Choose an export path outside the Typst source files".into()),
            );
        } else if start_worker(state)
            && let Some(worker) = &state.preview.worker
        {
            worker.export(u64::MAX, snapshot, job);
            state.preview.export.busy = Some(Busy::Native);
            state.preview.export.message = None;
        } else {
            state.preview.export.message = state.preview.error.clone();
        }
        return true;
    }
    let Some(Busy::Text {
        session,
        main,
        markdown,
        pending,
        ..
    }) = &mut state.preview.export.busy
    else {
        return false;
    };
    if *session != state.files.session || !state.language.ready() {
        finished(
            &mut state.preview,
            Err("Tinymist is unavailable for this export".into()),
        );
        return true;
    }
    if !std::mem::take(pending) {
        return false;
    }
    let query = fast_lsp::Query::export_text(main, *markdown);
    if !language::request(state, query, None) {
        finished(
            &mut state.preview,
            Err("Couldn't send the export request to Tinymist".into()),
        );
        return true;
    }
    false
}

pub(crate) fn text_reply(state: &mut State, result: Result<Vec<u8>, String>) {
    let Some(Busy::Text { path, session, .. }) = state.preview.export.busy.take() else {
        return;
    };
    if session != state.files.session {
        return;
    }
    match (result, &state.preview.worker) {
        (Ok(bytes), Some(worker)) => {
            worker.write(path, bytes, true);
            state.preview.export.busy = Some(Busy::Native);
        }
        (Err(error), _) => finished(&mut state.preview, Err(error)),
        _ => finished(
            &mut state.preview,
            Err("The Typst preview worker stopped".into()),
        ),
    }
}

pub(super) fn finished(preview: &mut Preview, result: Result<Vec<PathBuf>, String>) {
    preview.export.busy = None;
    preview.export.message = Some(match result {
        Ok(paths) if paths.len() > 1 => format!("Exported {} pages", paths.len()),
        Ok(paths) => paths.first().and_then(|path| path.file_name()).map_or_else(
            || "Exported".into(),
            |name| format!("Exported {}", name.to_string_lossy()),
        ),
        Err(error) => format!("Export failed. {error}"),
    });
}
