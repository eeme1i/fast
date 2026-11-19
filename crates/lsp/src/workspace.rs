use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use fast_core::{
    document::{Document, Edit},
    file,
};
use lsp_types::{
    DocumentChangeOperation, DocumentChanges, OneOf, ResourceOp, TextEdit, Url, WorkspaceEdit,
};
use ropey::Rope;

use crate::range;

pub struct WorkspacePlan {
    changes: BTreeMap<PathBuf, Vec<TextEdit>>,
    creates: BTreeSet<PathBuf>,
    rename: Option<(PathBuf, PathBuf)>,
}

pub struct Applied {
    pub path: PathBuf,
    pub edits: Vec<Edit>,
    pub moved: Option<(PathBuf, PathBuf)>,
    pub updated: Vec<PathBuf>,
}

impl WorkspacePlan {
    pub fn from_edit(edit: WorkspaceEdit, active: &Url, version: i32) -> Result<Self, String> {
        let mut plan = Self {
            changes: BTreeMap::new(),
            creates: BTreeSet::new(),
            rename: None,
        };
        if let Some(changes) = edit.changes {
            for (uri, edits) in changes {
                plan.changes.entry(path(&uri)?).or_default().extend(edits);
            }
        }
        let operations = match edit.document_changes {
            Some(DocumentChanges::Edits(edits)) => edits
                .into_iter()
                .map(DocumentChangeOperation::Edit)
                .collect(),
            Some(DocumentChanges::Operations(operations)) => operations,
            None => Vec::new(),
        };
        for operation in operations {
            match operation {
                DocumentChangeOperation::Edit(document) => {
                    if document.text_document.version.is_some()
                        && (&document.text_document.uri != active
                            || document.text_document.version != Some(version))
                    {
                        return Err(
                            "Tinymist supplied an edit for a different document version".into()
                        );
                    }
                    plan.changes
                        .entry(path(&document.text_document.uri)?)
                        .or_default()
                        .extend(document.edits.into_iter().map(|edit| match edit {
                            OneOf::Left(edit) => edit,
                            OneOf::Right(edit) => edit.text_edit,
                        }));
                }
                DocumentChangeOperation::Op(ResourceOp::Create(create)) => {
                    plan.creates.insert(path(&create.uri)?);
                }
                DocumentChangeOperation::Op(ResourceOp::Rename(rename))
                    if plan.rename.is_none() =>
                {
                    plan.rename = Some((path(&rename.old_uri)?, path(&rename.new_uri)?));
                }
                DocumentChangeOperation::Op(_) => {
                    return Err("This action requests an unsupported file operation".into());
                }
            }
        }
        Ok(plan)
    }

    pub fn local_edits(&self, active: &Path) -> Option<&[TextEdit]> {
        if !self.creates.is_empty()
            || self.rename.is_some()
            || self.changes.keys().any(|path| path != active)
        {
            return None;
        }
        Some(self.changes.get(active).map_or(&[], Vec::as_slice))
    }

    pub fn paths(&self) -> Vec<&Path> {
        let mut paths: BTreeSet<_> = self
            .changes
            .keys()
            .chain(self.creates.iter())
            .map(PathBuf::as_path)
            .collect();
        if let Some((from, to)) = &self.rename {
            paths.extend([from.as_path(), to.as_path()]);
        }
        paths.into_iter().collect()
    }

    /// Validate every path, text range and file before writing. The active document must
    /// still match the saved bytes. A failure restores writes already made.
    pub fn apply(self, root: &Path, active: &Path, snapshot: &Document) -> Result<Applied, String> {
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        let active = checked(&root, active, false)?;
        let mut originals = BTreeMap::new();
        let mut target_edits = Vec::new();
        let mut prepared = Vec::new();
        let mut paths = self.changes;
        for create in &self.creates {
            paths.entry(create.clone()).or_default();
        }
        for (requested, edits) in paths {
            let new = self.creates.contains(&requested);
            let path = checked(&root, &requested, new)?;
            if originals.contains_key(&path) {
                return Err(format!("Multiple edits target {}", path.display()));
            }
            let original = if new {
                None
            } else {
                Some(
                    file::open(&path)
                        .map_err(|error| format!("{}: {error}", path.display()))?
                        .text,
                )
            };
            if path == active && original.as_ref() != Some(&snapshot.snapshot()) {
                return Err(
                    "The active file changed on disk. Reopen it before applying this edit.".into(),
                );
            }
            let mut document = Document::from_rope(original.clone().unwrap_or_default());
            let edits = text_edits(&document, &edits)
                .ok_or_else(|| format!("Invalid edit range in {}", path.display()))?;
            let applied = document.replace_ranges(edits.clone());
            if path == active {
                target_edits = edits;
            }
            if new || !applied.is_empty() {
                prepared.push((path.clone(), document.snapshot()));
            }
            originals.insert(path, original);
        }
        let moved = self
            .rename
            .map(|(from, to)| -> Result<_, String> {
                let from = checked(&root, &from, false)?;
                if from == active
                    && file::open(&from).map_err(|error| error.to_string())?.text
                        != snapshot.snapshot()
                {
                    return Err("The file being renamed changed on disk".into());
                }
                let to = checked(&root, &to, true)?;
                if originals.contains_key(&to) {
                    return Err("The rename target also has a pending write".into());
                }
                Ok((from, to))
            })
            .transpose()?;

        let mut updated = Vec::new();
        let result = (|| -> Result<(), String> {
            for (path, text) in prepared {
                match &originals[&path] {
                    Some(original) => {
                        let current = file::open(&path)
                            .map_err(|error| format!("{}: {error}", path.display()))?;
                        if &current.text != original {
                            return Err(format!(
                                "{} changed before the edits could be applied",
                                path.display()
                            ));
                        }
                        file::save(&path, &text)
                            .map_err(|error| format!("{}: {error}", path.display()))?;
                    }
                    None => create(&path, &text)
                        .map_err(|error| format!("{}: {error}", path.display()))?,
                }
                updated.push(path);
            }
            if let Some((from, to)) = &moved {
                move_file(from, to)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut failures = Vec::new();
            for path in updated.into_iter().rev() {
                let restore = match &originals[&path] {
                    Some(original) => file::save(&path, original).map(|_| ()),
                    None => fs::remove_file(&path),
                };
                if let Err(error) = restore {
                    failures.push(format!("{}: {error}", path.display()));
                }
            }
            return if failures.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error}. Could not restore {}",
                    failures.join(", ")
                ))
            };
        }
        let path = moved
            .as_ref()
            .filter(|(from, _)| from == &active)
            .map_or(active, |(_, to)| to.clone());
        Ok(Applied {
            path,
            edits: target_edits,
            moved,
            updated,
        })
    }
}

pub fn text_edits(document: &Document, edits: &[TextEdit]) -> Option<Vec<Edit>> {
    let mut changes: Vec<_> = edits
        .iter()
        .map(|edit| {
            Some(Edit {
                range: range(document, edit.range)?,
                text: edit.new_text.clone(),
            })
        })
        .collect::<Option<_>>()?;
    changes.sort_by_key(|edit| (edit.range.start, edit.range.end));
    (!changes
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start))
    .then_some(changes)
}

fn path(uri: &Url) -> Result<PathBuf, String> {
    uri.to_file_path()
        .map_err(|()| "Tinymist tried to write a non-file location".into())
}

fn checked(root: &Path, path: &Path, new: bool) -> Result<PathBuf, String> {
    let path = if new {
        let parent = path.parent().ok_or("Missing target directory")?;
        let name = path.file_name().ok_or("Missing target file name")?;
        fs::canonicalize(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?
            .join(name)
    } else {
        fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?
    };
    if !path.starts_with(root) {
        return Err(format!("{} is outside the Typst workspace", path.display()));
    }
    let metadata = fs::symlink_metadata(&path);
    match (new, metadata) {
        (true, Err(error)) if error.kind() == io::ErrorKind::NotFound => Ok(path),
        (true, Ok(_)) => Err(format!("{} already exists", path.display())),
        (false, Ok(entry)) if entry.is_file() => Ok(path),
        (_, Ok(_)) => Err(format!("{} is not a regular file", path.display())),
        (_, Err(error)) => Err(format!("{}: {error}", path.display())),
    }
}

fn create(path: &Path, text: &Rope) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or(io::Error::other("Missing parent directory"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    text.write_to(file.as_file_mut())?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    Ok(())
}

fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    fs::hard_link(from, to)
        .map_err(|error| format!("Couldn't rename {}. {error}", from.display()))?;
    if let Err(error) = fs::remove_file(from) {
        let rollback = fs::remove_file(to);
        if let Err(rollback) = rollback {
            return Err(format!(
                "Couldn't rename {}. {error}. Couldn't remove {}. {rollback}",
                from.display(),
                to.display()
            ));
        }
        return Err(format!("Couldn't rename {}. {error}", from.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range, RenameFile};

    #[test]
    fn updates_an_import_and_renames_its_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let main = root.join("main.typ");
        let part = root.join("part.typ");
        let next = root.join("chapter.typ");
        let text = "#include \"part.typ\"\n";
        fs::write(&main, text)?;
        fs::write(&part, "Chapter\n")?;
        let uri = Url::from_file_path(&main).map_err(|()| "invalid URI")?;
        let rename = RenameFile {
            old_uri: Url::from_file_path(&part).map_err(|()| "invalid source")?,
            new_uri: Url::from_file_path(&next).map_err(|()| "invalid target")?,
            options: None,
            annotation_id: None,
        };
        let changes = WorkspaceEdit {
            changes: Some(
                [(
                    uri.clone(),
                    vec![TextEdit::new(
                        Range::new(Position::new(0, 10), Position::new(0, 18)),
                        "chapter.typ".into(),
                    )],
                )]
                .into(),
            ),
            document_changes: Some(DocumentChanges::Operations(vec![
                DocumentChangeOperation::Op(ResourceOp::Rename(rename)),
            ])),
            change_annotations: None,
        };
        let snapshot = Document::new(text);
        let applied = WorkspacePlan::from_edit(changes, &uri, 1)?.apply(&root, &main, &snapshot)?;
        assert_eq!(fs::read_to_string(&main)?, "#include \"chapter.typ\"\n");
        assert_eq!(fs::read_to_string(&next)?, "Chapter\n");
        assert!(!part.exists());
        assert_eq!(applied.edits.len(), 1);
        Ok(())
    }

    #[test]
    fn validates_all_ranges_before_writing() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let main = root.join("main.typ");
        let part = root.join("part.typ");
        fs::write(&main, "main")?;
        fs::write(&part, "part")?;
        let uri = Url::from_file_path(&main).map_err(|()| "invalid URI")?;
        let part_uri = Url::from_file_path(&part).map_err(|()| "invalid part URI")?;
        let changes = WorkspaceEdit::new(
            [
                (
                    uri.clone(),
                    vec![TextEdit::new(
                        Range::new(Position::new(0, 0), Position::new(0, 4)),
                        "edit".into(),
                    )],
                ),
                (
                    part_uri,
                    vec![TextEdit::new(
                        Range::new(Position::new(0, 0), Position::new(1, 0)),
                        "edit".into(),
                    )],
                ),
            ]
            .into(),
        );
        assert!(
            WorkspacePlan::from_edit(changes, &uri, 1)?
                .apply(&root, &main, &Document::new("main"))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&main)?, "main");
        assert_eq!(fs::read_to_string(&part)?, "part");
        Ok(())
    }
}
