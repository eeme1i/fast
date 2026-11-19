//! Background Typst compilation, rasterization, source mapping, and export.

mod export;
mod raster;
mod worker;
mod world;

use std::{collections::BTreeMap, ops::Range, path::PathBuf, sync::Arc, time::Duration};

pub use export::{Export, Format};
pub use worker::Worker;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub root: PathBuf,
    pub font_paths: Vec<PathBuf>,
    pub system_fonts: bool,
    pub inputs: BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct Snapshot {
    pub settings: Settings,
    pub main: PathBuf,
    pub active: PathBuf,
    pub text: ropey::Rope,
    pub other_buffers: Vec<(PathBuf, ropey::Rope)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagePoint {
    pub page: usize,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SourcePoint {
    pub path: PathBuf,
    pub byte: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterRequest {
    pub pages: Range<usize>,
    /// Scale as a multiple of one eighth of a pixel per point.
    pub scale: u16,
    pub inverted: bool,
}

pub struct Bitmap {
    pub page: usize,
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

pub enum Event {
    Compiled {
        generation: u64,
        elapsed: Duration,
        result: Result<Vec<PageSize>, String>,
    },
    Rasterized {
        generation: u64,
        request: RasterRequest,
        result: Result<Vec<Bitmap>, String>,
    },
    Source {
        generation: u64,
        point: Option<SourcePoint>,
    },
    Position {
        generation: u64,
        caret: usize,
        point: Option<PagePoint>,
    },
    Exported(Result<Vec<PathBuf>, String>),
}
