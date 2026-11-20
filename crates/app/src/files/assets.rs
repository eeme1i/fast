use std::{
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use fast_core::document::Selection;
use fast_editor::VIEWPORT;
use rune::{ClipboardContent, ClipboardImage, Latest, Tx, Waker};
use tempfile::NamedTempFile;

use crate::{State, language};

#[derive(Default)]
pub(super) struct Paste {
    job: Latest<io::Result<Asset>>,
    target: Option<Target>,
}

#[derive(PartialEq, Eq)]
struct Target {
    session: u64,
    revision: u64,
    selections: Vec<Selection>,
}

impl Target {
    fn current(state: &State) -> Self {
        Self {
            session: state.files.session,
            revision: state.editor.document().revision(),
            selections: state.editor.document().selections().to_vec(),
        }
    }
}

struct Asset {
    path: PathBuf,
    source: String,
}

impl Paste {
    pub fn setup(&self, waker: Waker) {
        self.job.set_waker(waker);
    }
}

pub(crate) fn paste(state: &mut State, content: &ClipboardContent, tx: &mut Tx) {
    if tx.focused() != Some(VIEWPORT) {
        return;
    }
    let Some(image) = content.image() else {
        if !content.text().is_empty() {
            state.editor.insert_plain(content.text(), tx);
        }
        return;
    };
    if state.files.assets.target.is_some() {
        state.files.message = Some("An image paste is already in progress".into());
    } else if let Some(parent) = state.files.path.as_deref().and_then(Path::parent) {
        let root = language::options(state, parent).root;
        let target = Target::current(state);
        state.files.assets.target = Some(target);
        state.files.assets.job.request(
            (
                image.clone(),
                parent.to_owned(),
                root,
                state
                    .assets_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("assets")),
            ),
            |(image, source, root, assets)| write(&image, &source, &root, &assets),
        );
        state.files.message = Some("Saving clipboard image…".into());
    } else {
        state.files.message = Some("Save this tab before pasting an image".into());
    }
    tx.instant();
}

pub(super) fn poll(state: &mut State, tx: &mut Tx) -> bool {
    let Some(result) = state.files.assets.job.take() else {
        return false;
    };
    let can_insert = state.files.assets.target.take() == Some(Target::current(state))
        && !state.language.workspace.showing();
    match result {
        Ok(asset) if can_insert => {
            state.editor.insert_plain(&asset.source, tx);
            state.files.message = Some(format!("Pasted {}", asset.path.display()));
        }
        Ok(asset) => {
            state.files.message = Some(format!(
                "Image saved to {}. The target selection changed.",
                asset.path.display()
            ));
        }
        Err(error) => state.files.message = Some(format!("Couldn’t paste image. {error}")),
    }
    if state.workbench.project.root.is_some() {
        state.workbench.project.refresh();
    }
    tx.instant();
    true
}

fn write(image: &ClipboardImage, source: &Path, root: &Path, assets: &Path) -> io::Result<Asset> {
    let root = root.canonicalize()?;
    let source = source.canonicalize()?;
    if !source.starts_with(&root) {
        return Err(io::Error::other(
            "The source file is outside the compiler root",
        ));
    }
    let directory = asset_directory(&root, assets)?;
    let mut temporary = NamedTempFile::new_in(&directory)?;
    temporary.write_all(image.png())?;
    temporary.as_file().sync_all()?;
    for index in 1.. {
        let name = if index == 1 {
            "image.png".into()
        } else {
            format!("image-{index}.png")
        };
        let path = directory.join(&name);
        let reference = relative(&source, &path)?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {
                return Ok(Asset {
                    path,
                    source: format!("#image(\"{reference}\")"),
                });
            }
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                temporary = error.file;
            }
            Err(error) => return Err(error.error),
        }
    }
    unreachable!("unbounded image names")
}

fn asset_directory(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut directory = root.to_owned();
    for component in relative.components() {
        match component {
            Component::CurDir => continue,
            Component::Normal(name) => directory.push(name),
            _ => {
                return Err(io::Error::other(
                    "The assets folder must be inside the compiler root",
                ));
            }
        }
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        directory = directory.canonicalize()?;
        if !directory.starts_with(root) || !directory.is_dir() {
            return Err(io::Error::other(
                "The assets folder must be inside the compiler root",
            ));
        }
    }
    Ok(directory)
}

fn relative(from: &Path, target: &Path) -> io::Result<String> {
    let common = from
        .components()
        .zip(target.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts = vec!["..".to_owned(); from.components().count() - common];
    for part in target.components().skip(common) {
        let part = part
            .as_os_str()
            .to_str()
            .ok_or_else(|| io::Error::other("The assets path must be valid UTF-8"))?;
        parts.push(
            part.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t"),
        );
    }
    Ok(parts.join("/"))
}
