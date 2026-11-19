mod camera;
mod export;
pub(crate) mod view;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use fast_preview::{Event, PageSize, RasterRequest, Settings, Snapshot, Worker};
use rune::{Pixels, Tx};

use crate::{State, files, language};
use camera::Camera;
pub(crate) use export::{
    export_dialog, export_overlay, export_status, on_saved, quick_export, text_reply,
};

const DEBOUNCE: Duration = Duration::from_millis(22);
const ZOOM_SETTLE: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(crate) struct Preview {
    pub visible: bool,
    worker: Option<Worker>,
    generation: u64,
    binding: Option<Binding>,
    due: Option<f64>,
    ready: Option<u64>,
    pages: Vec<PageSize>,
    camera: Camera,
    draw: Option<RasterRequest>,
    images: Vec<SheetImage>,
    camera_changed_at: Option<f64>,
    zooming: bool,
    message: Option<String>,
    error: Option<String>,
    inverted: bool,
    follow: bool,
    caret: Option<usize>,
    drag_start: [f32; 2],
    settings_menu: bool,
    export: export::Controls,
}

struct SheetImage {
    page: usize,
    id: u64,
    pixels: Pixels,
}

#[derive(PartialEq, Eq)]
struct Binding {
    session: u64,
    buffer_epoch: u64,
    revision: u64,
    active: PathBuf,
    main: PathBuf,
    settings: Settings,
}

impl Binding {
    fn new(state: &State) -> Option<Self> {
        let active = state.files.path.as_ref()?.clone();
        if !is_typst(&active) {
            return None;
        }
        let main = state.language.pinned.as_ref().unwrap_or(&active).clone();
        let parent = main.parent()?;
        let options = language::options(state, parent);
        Some(Self {
            session: state.files.session,
            buffer_epoch: state.files.buffer_epoch,
            revision: state.editor.document().revision(),
            active,
            main,
            settings: Settings {
                root: options.root,
                font_paths: options.compiler.font_paths,
                system_fonts: options.compiler.system_fonts,
                inputs: options.compiler.inputs,
            },
        })
    }

    fn snapshot(&self, state: &State) -> Snapshot {
        Snapshot {
            settings: self.settings.clone(),
            main: self.main.clone(),
            active: self.active.clone(),
            text: state.editor.document().snapshot(),
            other_buffers: state
                .files
                .dirty_buffers()
                .map(|(path, document)| (path.to_owned(), document.snapshot()))
                .collect(),
        }
    }
}

fn is_typst(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("typ"))
}

pub(crate) fn available(state: &State) -> bool {
    state.files.path.as_deref().is_some_and(is_typst)
}

pub(crate) fn toggle(state: &mut State, tx: &mut Tx) {
    state.preview.visible = !state.preview.visible;
    state
        .workbench
        .preview_toggled(state.preview.visible, state.ui.preview_dock);
    if !state.preview.visible {
        state.preview.settings_menu = false;
        state.preview.draw = None;
        tx.focus(fast_editor::VIEWPORT, false);
    }
    tx.wake_after(Duration::ZERO);
    tx.instant();
}

pub(crate) fn report(preview: &Preview) -> Option<&str> {
    preview.error.as_deref().or(preview.message.as_deref())
}

fn start_worker(state: &mut State) -> bool {
    if state.preview.worker.is_some() {
        return true;
    }
    let Some(waker) = state.waker.clone() else {
        return false;
    };
    match Worker::new(move || waker()) {
        Ok(worker) => {
            state.preview.worker = Some(worker);
            true
        }
        Err(error) => {
            state.preview.error = Some(format!("Couldn't start preview: {error}"));
            false
        }
    }
}

pub(crate) fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = export::poll(state);
    if state.preview.visible && available(state) {
        if start_worker(state) {
            changed |= sync(state, now, tx);
        }
    } else if state.preview.visible {
        state.preview.visible = false;
        state.preview.camera.sheets.clear();
        state.preview.images.clear();
        state.preview.binding = None;
        changed = true;
    }
    loop {
        let event = match state.preview.worker.as_ref().map(Worker::poll) {
            Some(Ok(Some(event))) => event,
            Some(Err(_)) => {
                state.preview.worker = None;
                state.preview.ready = None;
                state.preview.binding = None;
                state.preview.export.busy = None;
                state.preview.error = Some("The Typst preview worker stopped".into());
                changed = true;
                break;
            }
            _ => break,
        };
        changed |= receive(state, event, tx);
    }
    if state.preview.visible {
        if std::mem::take(&mut state.preview.zooming) {
            state.preview.camera_changed_at = Some(now);
        }
        request_pages(&mut state.preview, now, tx);
        if state.preview.follow
            && let Some(generation) = state.preview.ready
        {
            let caret = state.editor.document().selection().head;
            if state.preview.caret != Some(caret)
                && let Some(worker) = &state.preview.worker
            {
                state.preview.caret = Some(caret);
                worker.position(generation, caret);
            }
        }
    }
    if changed {
        tx.instant();
    }
    changed
}

fn sync(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    let binding = Binding::new(state);
    let preview = &mut state.preview;
    if binding != preview.binding {
        let reset = preview.binding.as_ref().map(|b| (&b.main, &b.settings))
            != binding.as_ref().map(|b| (&b.main, &b.settings));
        if reset {
            preview.pages.clear();
            preview.images.clear();
            preview.camera.layout(&[]);
            preview.camera.pan([0.0, 12.0]);
        }
        preview.binding = binding;
        preview.generation += 1;
        preview.ready = None;
        preview.draw = None;
        preview.caret = None;
        preview.error = None;
        preview.due = Some(if reset {
            now
        } else {
            now + DEBOUNCE.as_secs_f64()
        });
        if preview.pages.is_empty() {
            preview.message = Some("Compiling preview…".into());
            changed = true;
        }
    }
    if let Some(due) = preview.due {
        if now >= due {
            preview.due = None;
            if let (Some(worker), Some(binding)) = (&state.preview.worker, &state.preview.binding) {
                worker.compile(state.preview.generation, binding.snapshot(state));
            }
        } else {
            tx.wake_at(due);
        }
    }
    changed
}

fn receive(state: &mut State, event: Event, tx: &mut Tx) -> bool {
    let preview = &mut state.preview;
    match event {
        Event::Compiled {
            generation,
            elapsed,
            result,
        } if preview.generation == generation => {
            match result {
                Ok(pages) => {
                    preview.ready = Some(generation);
                    preview.pages = pages;
                    preview.camera.layout(&preview.pages);
                    preview
                        .images
                        .retain(|image| image.page < preview.pages.len());
                    preview.error = None;
                    preview.message =
                        Some(format!("Preview {:.0} ms", elapsed.as_secs_f64() * 1000.0));
                }
                Err(error) => {
                    preview.error = Some(error);
                }
            }
            true
        }
        Event::Rasterized {
            generation,
            request,
            result,
        } if preview.ready == Some(generation) && preview.draw.as_ref() == Some(&request) => {
            match result {
                Ok(bitmaps) => {
                    let previous = std::mem::take(&mut preview.images);
                    preview.images = bitmaps
                        .into_iter()
                        .map(|bitmap| {
                            let pixels = previous
                                .iter()
                                .find(|image| image.id == bitmap.id)
                                .map(|image| image.pixels.clone())
                                .unwrap_or_else(|| {
                                    let pixels = Pixels::new();
                                    let rgba = std::sync::Arc::unwrap_or_clone(bitmap.rgba);
                                    pixels.set(bitmap.width, bitmap.height, rgba);
                                    pixels
                                });
                            SheetImage {
                                page: bitmap.page,
                                id: bitmap.id,
                                pixels,
                            }
                        })
                        .collect();
                }
                Err(error) => preview.error = Some(error),
            }
            true
        }
        Event::Source { generation, point }
            if preview.visible && preview.ready == Some(generation) =>
        {
            if let Some(point) = point {
                if state.files.path.as_ref() == Some(&point.path) {
                    state.workbench.show_editor();
                    state.editor.jump(point.byte..point.byte, tx);
                    tx.focus(fast_editor::VIEWPORT, false);
                } else {
                    files::jump(state, point.path, point.byte..point.byte, tx);
                }
            } else {
                preview.message = Some("No editable source at that point".into());
            }
            true
        }
        Event::Position {
            generation,
            caret,
            point,
        } if preview.visible
            && preview.follow
            && preview.ready == Some(generation)
            && preview.caret == Some(caret) =>
        {
            if let Some(point) = point {
                preview.camera.reveal(point);
            }
            false
        }
        Event::Exported(result) => {
            export::finished(preview, result);
            true
        }
        _ => false,
    }
}

fn request_pages(preview: &mut Preview, now: f64, tx: &mut Tx) {
    let (Some(generation), Some(worker)) = (preview.ready, &preview.worker) else {
        return;
    };
    if preview.camera.viewport[0] <= 0.0 {
        return;
    }
    let mut request = preview.camera.raster(preview.inverted);
    if let (Some(old), Some(at)) = (&preview.draw, preview.camera_changed_at) {
        let due = at + ZOOM_SETTLE.as_secs_f64();
        if now < due && old.scale != request.scale {
            request.scale = old.scale;
            tx.wake_at(due);
        }
    }
    if preview.draw.as_ref() != Some(&request) {
        preview.draw = Some(request.clone());
        worker.raster(generation, request);
    }
}
