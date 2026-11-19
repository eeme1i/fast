use std::{collections::HashMap, path::Path};

use typst::diag::{FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Dict, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Library, LibraryExt, World, WorldExt};
use typst_ide::IdeWorld;
use typst_kit::{
    datetime::Time,
    downloader::SystemDownloader,
    files::{FileLoader, FileStore, FsRoot, SystemFiles},
    fonts::{self, FontStore},
    packages::SystemPackages,
};

use crate::{Settings, Snapshot};

struct Files {
    disk: SystemFiles,
    buffers: HashMap<FileId, Bytes>,
}

impl FileLoader for Files {
    fn load(&self, id: FileId) -> FileResult<Bytes> {
        if let Some(text) = self.buffers.get(&id) {
            return Ok(text.clone());
        }
        self.disk.load(id)
    }
}

pub(crate) struct Project {
    pub settings: Settings,
    root: std::path::PathBuf,
    library: LazyHash<Library>,
    fonts: FontStore,
    files: FileStore<Files>,
    main: FileId,
    active: FileId,
    time: Time,
}

impl Project {
    pub fn new(settings: Settings, main: &Path) -> Result<Self, String> {
        let root = std::fs::canonicalize(&settings.root)
            .map_err(|error| format!("Compiler root {}: {error}", settings.root.display()))?;
        let main = project_id(&settings.root, &root, main)?;
        let mut fonts = FontStore::new();
        for directory in &settings.font_paths {
            if !directory.is_dir() {
                return Err(format!(
                    "Font directory unavailable: {}",
                    directory.display()
                ));
            }
            fonts.extend(fonts::scan(directory));
        }
        if settings.system_fonts {
            fonts.extend(fonts::system());
        }
        fonts.extend(fonts::embedded());
        let inputs: Dict = settings
            .inputs
            .iter()
            .map(|(key, value)| {
                (
                    key.as_str().into(),
                    typst::foundations::Value::Str(value.as_str().into()),
                )
            })
            .collect();
        let library = Library::builder()
            .with_inputs(inputs)
            .with_features([Feature::Html].into_iter().collect())
            .build();
        let files = Files {
            disk: SystemFiles::new(
                FsRoot::new(root.clone()),
                SystemPackages::new(SystemDownloader::new("Fast Typst Preview")),
            ),
            buffers: HashMap::new(),
        };
        Ok(Self {
            settings,
            root,
            library: LazyHash::new(library),
            fonts,
            files: FileStore::new(files),
            main,
            active: main,
            time: Time::system(),
        })
    }

    pub fn update(&mut self, snapshot: Snapshot) -> Result<(), String> {
        let main = project_id(&self.settings.root, &self.root, &snapshot.main)?;
        let active = project_id(&self.settings.root, &self.root, &snapshot.active)?;
        let text = snapshot.text.to_string();
        self.main = main;
        self.active = active;
        let buffers = &mut self.files.loader_mut().buffers;
        buffers.clear();
        for (path, source) in snapshot.other_buffers {
            if let Ok(id) = project_id(&self.settings.root, &self.root, &path) {
                buffers.insert(id, Bytes::from_string(source.to_string()));
            }
        }
        buffers.insert(active, Bytes::from_string(text));
        self.files.reset();
        self.time.reset();
        Ok(())
    }

    pub fn active_source(&self) -> FileResult<Source> {
        self.source(self.active)
    }

    pub fn path(&self, id: FileId) -> Option<std::path::PathBuf> {
        // Package files are dependencies and should never open for editing here.
        matches!(id.root(), VirtualRoot::Project)
            .then(|| self.files.loader().disk.resolve(id).ok())
            .flatten()
    }

    pub fn message(&self, diagnostics: &[SourceDiagnostic]) -> String {
        let Some(first) = diagnostics.first() else {
            return "Typst compilation failed".into();
        };
        let location = first
            .span
            .id()
            .and_then(|id| {
                let source = self.source(id).ok()?;
                let line = source.lines().byte_to_line(self.range(first.span)?.start)? + 1;
                Some(format!("{}:{line}: ", id.vpath().get_without_slash()))
            })
            .unwrap_or_default();
        let extra = diagnostics.len().saturating_sub(1);
        let suffix = if extra > 0 {
            format!(" (+{extra} more)")
        } else {
            String::new()
        };
        format!("{location}{}{suffix}", first.message)
    }
}

fn project_id(configured: &Path, canonical: &Path, path: &Path) -> Result<FileId, String> {
    let root = if path.starts_with(configured) {
        configured
    } else {
        canonical
    };
    let path = VirtualPath::virtualize(root, path)
        .map_err(|error| format!("{} (root {}): {error}", path.display(), root.display()))?;
    Ok(FileId::new(RootedPath::new(VirtualRoot::Project, path)))
}

impl World for Project {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        self.files.source(id)
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.file(id)
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }
    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        self.time.today(offset)
    }
}

impl IdeWorld for Project {
    fn upcast(&self) -> &dyn World {
        self
    }
}
