use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use fast_core::config::{ExportConfig, PdfStandard, PngPages};
use typst_layout::PagedDocument;

use crate::{raster::raster_options, world::Project};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Pdf,
    Png,
    Svg,
    Html,
    Markdown,
    Text,
}

impl Format {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Svg => "svg",
            Self::Html => "html",
            Self::Markdown => "md",
            Self::Text => "txt",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Png => "PNG",
            Self::Svg => "SVG",
            Self::Html => "HTML",
            Self::Markdown => "Markdown",
            Self::Text => "Plain text",
        }
    }
}

pub struct Export {
    pub format: Format,
    pub destination: PathBuf,
    pub page: usize,
    pub settings: ExportConfig,
    pub overwrite: bool,
}

pub(crate) fn write(
    job: Export,
    world: &Project,
    doc: Option<&PagedDocument>,
) -> Result<Vec<PathBuf>, String> {
    let bytes = match job.format {
        Format::Pdf => {
            let mut options = typst_pdf::PdfOptions {
                tagged: job.settings.pdf_tagged,
                timestamp: timestamp(),
                ..Default::default()
            };
            if let Some(standard) = job.settings.pdf_standard {
                let standard = match standard {
                    PdfStandard::A2b => typst_pdf::PdfStandard::A_2b,
                    PdfStandard::A2u => typst_pdf::PdfStandard::A_2u,
                    PdfStandard::A3b => typst_pdf::PdfStandard::A_3b,
                };
                options.standards = typst_pdf::PdfStandards::new(&[standard])
                    .map_err(|error| error.message().to_string())?;
            }
            typst_pdf::pdf(paged(doc)?, &options).map_err(|errors| world.message(&errors))?
        }
        Format::Png => return write_png(job, paged(doc)?),
        Format::Svg => {
            let page = paged(doc)?
                .pages()
                .get(job.page)
                .ok_or("Page no longer exists")?;
            typst_svg::svg(page, &typst_svg::SvgOptions::default()).into_bytes()
        }
        Format::Html => {
            let html = typst::compile::<typst_html::HtmlDocument>(world)
                .output
                .map_err(|errors| world.message(&errors))?;
            typst_html::html(&html, &typst_html::HtmlOptions::default())
                .map_err(|errors| world.message(&errors))?
                .into_bytes()
        }
        Format::Markdown | Format::Text => return Err("This export needs Tinymist".into()),
    };
    write_bytes(job.destination, &bytes, job.overwrite)
}

fn timestamp() -> Option<typst_pdf::Timestamp> {
    let now = time::OffsetDateTime::now_utc();
    let date = typst::foundations::Datetime::from_ymd_hms(
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )?;
    Some(typst_pdf::Timestamp::new_utc(date))
}

fn write_png(job: Export, doc: &PagedDocument) -> Result<Vec<PathBuf>, String> {
    let count = doc.pages().len();
    let (first, last) = match job.settings.png_pages {
        PngPages::Current => (job.page + 1, job.page + 1),
        PngPages::All => (1, count),
        PngPages::Range { first, last } => (first, last),
    };
    if first == 0 || last < first || last > count {
        return Err(format!(
            "Pages {first}–{last} are outside the {count}-page document"
        ));
    }
    let render = |page| {
        let options = raster_options(page, f32::from(job.settings.png_ppi) / 72.0)?;
        typst_render::render(page, &options)
            .encode_png()
            .map_err(|error| error.to_string())
    };
    if first == last {
        return write_bytes(
            job.destination,
            &render(&doc.pages()[first - 1])?,
            job.overwrite,
        );
    }
    let folder = job
        .destination
        .parent()
        .ok_or("Choose a destination folder")?;
    let stem = job
        .destination
        .file_stem()
        .ok_or("Choose a file name")?
        .to_string_lossy();
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let padding = last.to_string().len().max(2);
    let mut staged = Vec::new();
    for page in first..=last {
        let path = folder.join(format!("{stem}-{page:0padding$}.png"));
        let bytes = render(&doc.pages()[page - 1])?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(folder).map_err(|error| error.to_string())?;
        temporary
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        staged.push((path, temporary));
    }
    let mut written = Vec::new();
    for (path, temporary) in staged {
        if let Err(error) = temporary.persist_noclobber(&path) {
            let failures: Vec<_> = written
                .iter()
                .filter_map(|path: &PathBuf| {
                    fs::remove_file(path)
                        .err()
                        .map(|error| format!("{}: {error}", path.display()))
                })
                .collect();
            return Err(format!(
                "{}: {}{}",
                path.display(),
                error.error,
                if failures.is_empty() {
                    String::new()
                } else {
                    format!(". Could not undo writes to {}", failures.join(", "))
                }
            ));
        }
        written.push(path);
    }
    Ok(written)
}

pub(crate) fn write_bytes(
    path: PathBuf,
    bytes: &[u8],
    overwrite: bool,
) -> Result<Vec<PathBuf>, String> {
    atomic_write(&path, bytes, overwrite)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(vec![path])
}

fn paged(doc: Option<&PagedDocument>) -> Result<&PagedDocument, String> {
    doc.ok_or_else(|| "Fix the Typst error before exporting".into())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8], overwrite: bool) -> std::io::Result<()> {
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(folder)?;
    let mut temporary = tempfile::NamedTempFile::new_in(folder)?;
    temporary.write_all(bytes)?;
    if overwrite {
        temporary.persist(path)?;
    } else {
        temporary.persist_noclobber(path)?;
    }
    Ok(())
}
