use std::{
    num::NonZeroUsize,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Instant,
};

use typst::introspection::PagedPosition;
use typst::layout::{Abs, Point};
use typst_layout::PagedDocument;

use crate::{
    Event, Export, PagePoint, PageSize, RasterRequest, Snapshot, SourcePoint, export,
    raster::Cache, world::Project,
};

pub struct Worker {
    queue: Arc<Queue>,
    events: mpsc::Receiver<Event>,
}

#[derive(Default)]
struct Queue {
    state: Mutex<Jobs>,
    ready: Condvar,
}

#[derive(Default)]
struct Jobs {
    closed: bool,
    compile: Option<(u64, Snapshot)>,
    raster: Option<(u64, RasterRequest)>,
    source: Option<(u64, PagePoint)>,
    position: Option<(u64, usize)>,
    export: Option<(u64, Snapshot, Export)>,
    written: Option<(std::path::PathBuf, Vec<u8>, bool)>,
}

enum Job {
    Compile(u64, Snapshot),
    Raster(u64, RasterRequest),
    Source(u64, PagePoint),
    Position(u64, usize),
    Export(u64, Snapshot, Export),
    Write(std::path::PathBuf, Vec<u8>, bool),
}

impl Jobs {
    fn take(&mut self) -> Option<Job> {
        if let Some((path, bytes, overwrite)) = self.written.take() {
            return Some(Job::Write(path, bytes, overwrite));
        }
        if let Some((generation, snapshot, export)) = self.export.take() {
            return Some(Job::Export(generation, snapshot, export));
        }
        if let Some((generation, snapshot)) = self.compile.take() {
            return Some(Job::Compile(generation, snapshot));
        }
        if let Some((generation, point)) = self.source.take() {
            return Some(Job::Source(generation, point));
        }
        if let Some((generation, request)) = self.raster.take() {
            return Some(Job::Raster(generation, request));
        }
        self.position
            .take()
            .map(|(generation, caret)| Job::Position(generation, caret))
    }
}

impl Worker {
    pub fn new(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let queue = Arc::new(Queue::default());
        let (sender, events) = mpsc::channel();
        let runner = Arc::clone(&queue);
        thread::Builder::new()
            .name("typst-preview".into())
            .spawn(move || {
                let mut engine = Engine::default();
                while let Some(job) = runner.next() {
                    let Some(event) = engine.run(job) else {
                        continue;
                    };
                    if sender.send(event).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self { queue, events })
    }

    pub fn poll(&self) -> Result<Option<Event>, mpsc::TryRecvError> {
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn compile(&self, generation: u64, snapshot: Snapshot) {
        self.push(|jobs| jobs.compile = Some((generation, snapshot)));
    }

    pub fn raster(&self, generation: u64, request: RasterRequest) {
        self.push(|jobs| jobs.raster = Some((generation, request)));
    }

    pub fn source(&self, generation: u64, point: PagePoint) {
        self.push(|jobs| jobs.source = Some((generation, point)));
    }

    pub fn position(&self, generation: u64, caret: usize) {
        self.push(|jobs| jobs.position = Some((generation, caret)));
    }

    pub fn export(&self, generation: u64, snapshot: Snapshot, export: Export) {
        self.push(|jobs| jobs.export = Some((generation, snapshot, export)));
    }

    pub fn write(&self, path: std::path::PathBuf, bytes: Vec<u8>, overwrite: bool) {
        self.push(|jobs| jobs.written = Some((path, bytes, overwrite)));
    }

    fn push(&self, add: impl FnOnce(&mut Jobs)) {
        let mut jobs = self
            .queue
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        add(&mut jobs);
        self.queue.ready.notify_one();
    }
}

impl Queue {
    fn next(&self) -> Option<Job> {
        let mut jobs = self.state.lock().ok()?;
        loop {
            if jobs.closed {
                return None;
            }
            if let Some(job) = jobs.take() {
                return Some(job);
            }
            jobs = self.ready.wait(jobs).ok()?;
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.push(|jobs| jobs.closed = true);
    }
}

#[derive(Default)]
struct Engine {
    generation: u64,
    world: Option<Project>,
    compiled: Option<PagedDocument>,
    cache: Cache,
}

impl Engine {
    fn run(&mut self, job: Job) -> Option<Event> {
        match job {
            Job::Write(path, bytes, overwrite) => Some(Event::Exported(export::write_bytes(
                path, &bytes, overwrite,
            ))),
            Job::Compile(generation, snapshot) => {
                let start = Instant::now();
                let result = self.compile(generation, snapshot).map(|doc| {
                    doc.pages()
                        .iter()
                        .map(|page| {
                            let size = page.frame.size();
                            PageSize {
                                width: size.x.to_pt() as f32,
                                height: size.y.to_pt() as f32,
                            }
                        })
                        .collect()
                });
                Some(Event::Compiled {
                    generation,
                    elapsed: start.elapsed(),
                    result,
                })
            }
            Job::Raster(generation, request) if generation == self.generation => {
                let result = self.cache.draw(self.compiled.as_ref()?, &request);
                Some(Event::Rasterized {
                    generation,
                    request,
                    result,
                })
            }
            Job::Source(generation, point) if generation == self.generation => {
                Some(Event::Source {
                    generation,
                    point: self.source(point),
                })
            }
            Job::Position(generation, caret) if generation == self.generation => {
                Some(Event::Position {
                    generation,
                    caret,
                    point: self.position(caret),
                })
            }
            Job::Export(generation, snapshot, job) => {
                // An export can target a different snapshot than the current preview.
                // Keep the preview world intact when an edit raced the export dialog.
                let result = if generation == self.generation
                    && let Some(world) = &self.world
                {
                    export::write(job, world, self.compiled.as_ref())
                } else {
                    let mut engine = Engine::default();
                    let compiled = engine.compile(generation, snapshot).map(|_| ());
                    match (compiled, &engine.world) {
                        (Ok(()), Some(world)) => {
                            export::write(job, world, engine.compiled.as_ref())
                        }
                        (Err(_), Some(world)) if job.format == crate::Format::Html => {
                            export::write(job, world, None)
                        }
                        (Err(error), _) => Err(error),
                        _ => Err("Typst compiler is unavailable".into()),
                    }
                };
                Some(Event::Exported(result))
            }
            _ => None,
        }
    }

    fn compile(&mut self, generation: u64, snapshot: Snapshot) -> Result<&PagedDocument, String> {
        self.generation = generation;
        self.compiled = None;
        if self
            .world
            .as_ref()
            .is_none_or(|world| world.settings != snapshot.settings)
        {
            self.world = None;
            self.world = Some(Project::new(snapshot.settings.clone(), &snapshot.main)?);
        }
        let world = self.world.as_mut().ok_or("Typst compiler is unavailable")?;
        if let Err(error) = world.update(snapshot) {
            self.world = None;
            return Err(error);
        }
        let compiled = typst::compile::<PagedDocument>(world).output;
        typst::comemo::evict(16);
        let compiled = compiled.map_err(|errors| world.message(&errors))?;
        Ok(self.compiled.insert(compiled))
    }

    fn source(&self, point: PagePoint) -> Option<SourcePoint> {
        let world = self.world.as_ref()?;
        let position = PagedPosition {
            page: NonZeroUsize::new(point.page.checked_add(1)?)?,
            point: Point::new(Abs::pt(f64::from(point.x)), Abs::pt(f64::from(point.y))),
        };
        let jump = typst_ide::jump_from_click(world, self.compiled.as_ref()?, &position)?;
        if let typst_ide::Jump::File(id, byte) = jump {
            return Some(SourcePoint {
                path: world.path(id)?,
                byte,
            });
        }
        None
    }

    fn position(&self, caret: usize) -> Option<PagePoint> {
        let source = self.world.as_ref()?.active_source().ok()?;
        let position = typst_ide::jump_from_cursor(self.compiled.as_ref()?, &source, caret)
            .into_iter()
            .next()?;
        Some(PagePoint {
            page: position.page.get() - 1,
            x: position.point.x.to_pt() as f32,
            y: position.point.y.to_pt() as f32,
        })
    }
}
