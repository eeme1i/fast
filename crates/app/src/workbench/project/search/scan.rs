use std::{
    fs::{self, File},
    io::{self, Read},
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const READ_CHUNK: usize = 32 * 1024;
const MAX_MATCHES: usize = 500;

#[derive(Default)]
pub(super) struct Results {
    pub matches: Vec<Match>,
    pub skipped_binary: usize,
    pub skipped_large: usize,
}

pub(super) struct Match {
    pub path: PathBuf,
    pub line: usize,
    pub range: Range<usize>,
    pub snippet: String,
    pub highlight: Range<usize>,
}

pub(super) type Found = Result<Results, String>;

pub(super) enum Source {
    Disk,
    Buffer(String),
    Large,
}

enum FileText {
    Text(String),
    Large,
    Binary,
    Unavailable,
}

pub(super) fn find(
    paths: impl Iterator<Item = PathBuf>,
    query: &str,
    source: impl Fn(&Path) -> Source,
    cancelled: &AtomicBool,
) -> Found {
    let search = regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
        .map_err(|error| error.to_string())?;
    let mut results = Results::default();
    for path in paths {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let text = match source(&path) {
            Source::Disk => read_text(&path, cancelled)
                .map_err(|error| format!("Couldn't search {}. {error}", path.display()))?,
            Source::Buffer(text) if binary_bytes(text.as_bytes()) => FileText::Binary,
            Source::Buffer(text) => FileText::Text(text),
            Source::Large => FileText::Large,
        };
        let source = match text {
            FileText::Text(text) => text,
            FileText::Large => {
                results.skipped_large += 1;
                continue;
            }
            FileText::Binary => {
                results.skipped_binary += 1;
                continue;
            }
            FileText::Unavailable => continue,
        };
        let mut byte = 0;
        for (line, text) in source.split_inclusive('\n').enumerate() {
            if cancelled.load(Ordering::Relaxed) {
                return Ok(results);
            }
            for found in search.find_iter(text) {
                let start = text[..found.start()]
                    .char_indices()
                    .rev()
                    .nth(59)
                    .map_or(0, |(index, _)| index);
                let snippet: String = text[start..]
                    .trim_end_matches(['\r', '\n'])
                    .chars()
                    .take(240)
                    .collect();
                results.matches.push(Match {
                    path: path.clone(),
                    line: line + 1,
                    range: byte + found.start()..byte + found.end(),
                    highlight: (found.start() - start).min(snippet.len())
                        ..(found.end() - start).min(snippet.len()),
                    snippet,
                });
                if results.matches.len() == MAX_MATCHES {
                    return Ok(results);
                }
            }
            byte += text.len();
        }
    }
    Ok(results)
}

fn read_text(path: &Path, cancelled: &AtomicBool) -> io::Result<FileText> {
    if binary_extension(path) {
        return Ok(FileText::Binary);
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(FileText::Unavailable),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() {
        return Ok(FileText::Unavailable);
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Ok(FileText::Large);
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(FileText::Unavailable),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Ok(FileText::Unavailable);
    }
    // Bound the read even if the file grows after the metadata check.
    read_bounded(file, cancelled)
}

fn read_bounded(reader: impl Read, cancelled: &AtomicBool) -> io::Result<FileText> {
    let mut reader = reader.take((MAX_FILE_BYTES + 1) as u64);
    let mut bytes = Vec::new();
    let mut chunk = [0; READ_CHUNK];
    let mut checked = 0;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(FileText::Unavailable);
        }
        let count = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if bytes.len() + count > MAX_FILE_BYTES {
            return Ok(FileText::Large);
        }
        if binary_bytes(&chunk[..count]) {
            return Ok(FileText::Binary);
        }
        bytes.extend_from_slice(&chunk[..count]);
        // A UTF-8 character can straddle chunks. Save its prefix for the next read.
        match std::str::from_utf8(&bytes[checked..]) {
            Ok(_) => checked = bytes.len(),
            Err(error) if error.error_len().is_none() => checked += error.valid_up_to(),
            Err(_) => return Ok(FileText::Binary),
        }
    }
    Ok(String::from_utf8(bytes).map_or(FileText::Binary, FileText::Text))
}

fn binary_bytes(bytes: &[u8]) -> bool {
    bytes.iter().any(|&byte| {
        byte.is_ascii_control() && !matches!(byte, b'\t' | b'\n' | b'\r' | b'\x0c' | b'\x1b')
    })
}

fn binary_extension(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "7z" | "a"
            | "aac"
            | "aiff"
            | "avi"
            | "avif"
            | "bin"
            | "bmp"
            | "bz2"
            | "class"
            | "dmg"
            | "dll"
            | "dylib"
            | "epub"
            | "exe"
            | "flac"
            | "gif"
            | "gz"
            | "heic"
            | "ico"
            | "iso"
            | "jar"
            | "jpeg"
            | "jpg"
            | "m4a"
            | "m4v"
            | "mkv"
            | "mov"
            | "mp3"
            | "mp4"
            | "o"
            | "ogg"
            | "otf"
            | "pdf"
            | "png"
            | "rar"
            | "so"
            | "sqlite"
            | "tar"
            | "tif"
            | "tiff"
            | "ttf"
            | "wasm"
            | "wav"
            | "webm"
            | "webp"
            | "woff"
            | "woff2"
            | "xz"
            | "zip"
            | "zst"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_reads_even_when_the_source_keeps_growing() -> io::Result<()> {
        let mut read = 0;
        let mut reader = io::repeat(b'a');
        let counting = |chunk: &mut [u8]| {
            read += chunk.len();
            reader.read(chunk)
        };
        let result = read_bounded(TestReader(counting), &AtomicBool::new(false))?;
        assert!(matches!(result, FileText::Large));
        assert_eq!(read, MAX_FILE_BYTES + 1);
        Ok(())
    }

    #[test]
    fn stops_reading_when_cancelled_or_binary() -> io::Result<()> {
        let cancelled = AtomicBool::new(false);
        for byte in [b'a', 0] {
            cancelled.store(false, Ordering::Relaxed);
            let mut calls = 0;
            let simulated = |chunk: &mut [u8]| {
                calls += 1;
                chunk[0] = byte;
                if byte == b'a' {
                    cancelled.store(true, Ordering::Relaxed);
                }
                Ok(1)
            };
            let result = read_bounded(TestReader(simulated), &cancelled)?;
            assert_eq!(calls, 1);
            if byte == 0 {
                assert!(matches!(result, FileText::Binary));
            } else {
                assert!(matches!(result, FileText::Unavailable));
            }
        }
        Ok(())
    }

    #[test]
    fn accepts_utf8_across_read_boundaries() -> io::Result<()> {
        let mut text = "a".repeat(READ_CHUNK - 1);
        text.push_str("é\r\nneedle");
        let result = read_bounded(text.as_bytes(), &AtomicBool::new(false))?;
        assert!(matches!(result, FileText::Text(decoded) if decoded == text));
        Ok(())
    }

    struct TestReader<F>(F);

    impl<F: FnMut(&mut [u8]) -> io::Result<usize>> Read for TestReader<F> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            (self.0)(buffer)
        }
    }
}
