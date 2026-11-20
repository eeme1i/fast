use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use ropey::Rope;
use tempfile::NamedTempFile;

pub struct OpenFile {
    pub path: PathBuf,
    pub text: Rope,
}

pub fn open(path: &Path) -> io::Result<OpenFile> {
    let path = fs::canonicalize(path)?;
    let text = read_text(&path)?;
    Ok(OpenFile { path, text })
}

pub fn read_text(path: &Path) -> io::Result<Rope> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    Rope::from_reader(BufReader::new(File::open(path)?))
}

/// Temp file, flush and replace on the same filesystem. An error before persist leaves the
/// original in place. Return the resolved path so subsequent writes follow the same file.
pub fn save(path: &Path, text: &Rope) -> io::Result<PathBuf> {
    save_checked(path, text, |_| Ok(()))
}

/// Verify the last version seen by the editor immediately before publishing the new version.
/// A missing expected version allows creating a file without replacing a newly arrived one.
pub fn save_if_unchanged(path: &Path, text: &Rope, expected: Option<&Rope>) -> io::Result<PathBuf> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()) {
        return Err(io::Error::other(
            "The file was replaced with a symbolic link",
        ));
    }
    if expected.is_none() {
        return save_checked_impl(path, text, |_| Ok(()), false);
    }
    save_checked(path, text, |path| {
        let changed =
            || io::Error::other("The file changed on disk. Review the changes before saving.");
        match read_text(path) {
            Ok(text) if Some(&text) == expected => Ok(()),
            Ok(_) => Err(changed()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(changed()),
            Err(error) => Err(error),
        }
    })
}

fn save_checked(
    path: &Path,
    text: &Rope,
    check: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    save_checked_impl(path, text, check, true)
}

fn save_checked_impl(
    path: &Path,
    text: &Rope,
    check: impl FnOnce(&Path) -> io::Result<()>,
    replace: bool,
) -> io::Result<PathBuf> {
    let path = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok_and(|entry| entry.is_symlink()) {
                return Err(error);
            }
            let name = path
                .file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            fs::canonicalize(parent)?.join(name)
        }
        Err(error) => return Err(error),
    };
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing parent directory"))?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    match fs::metadata(&path) {
        Ok(existing) if existing.is_file() => temporary
            .as_file()
            .set_permissions(existing.permissions())?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    text.write_to(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    check(&path)?;
    if replace {
        temporary.persist(&path)?;
    } else {
        temporary.persist_noclobber(&path)?;
    }
    Ok(path)
}
