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
    let text = Rope::from_reader(BufReader::new(File::open(&path)?))?;
    Ok(OpenFile { path, text })
}

/// Temp file, flush and replace on the same filesystem. An error before persist leaves the
/// original in place. Return the resolved path so subsequent writes follow the same file.
pub fn save(path: &Path, text: &Rope) -> io::Result<PathBuf> {
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
    temporary.persist(&path)?;
    Ok(path)
}
