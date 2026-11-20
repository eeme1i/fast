use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::ProjectDirs;
use notify_debouncer_full::notify::{self, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::Deserialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    DarculaClassic,
    #[serde(rename = "intellij-light-classic")]
    IntelliJLightClassic,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub theme: Theme,
    pub font_size: f32,
    pub font_family: FontFamily,
    pub density: Density,
    pub sidebar: Sidebar,
    pub preview_dock: PreviewDock,
    pub tabs: TabPlacement,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            font_size: 13.0,
            font_family: FontFamily::default(),
            density: Density::default(),
            sidebar: Sidebar::default(),
            preview_dock: PreviewDock::default(),
            tabs: TabPlacement::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FontFamily {
    #[default]
    Inter,
    SystemMono,
    Serif,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Density {
    Compact,
    #[default]
    Standard,
    Comfortable,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sidebar {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreviewDock {
    Left,
    #[default]
    Right,
    Bottom,
    Tab,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TabPlacement {
    #[default]
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EditorConfig {
    pub font_family: EditorFont,
    pub font_size: f32,
    pub line_height: f32,
    pub ligatures: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            font_family: EditorFont::SystemMono,
            font_size: 13.0,
            line_height: 22.0,
            ligatures: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EditorFont {
    #[default]
    SystemMono,
    Mono,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub editor: EditorConfig,
    pub lsp: LspConfig,
    pub compiler: CompilerConfig,
    pub export: ExportConfig,
    pub keymap: BTreeMap<String, String>,
    pub assets_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            editor: EditorConfig::default(),
            lsp: LspConfig::default(),
            compiler: CompilerConfig::default(),
            export: ExportConfig::default(),
            keymap: BTreeMap::new(),
            assets_dir: "assets".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ExportConfig {
    pub quick_path: String,
    pub on_save: bool,
    pub png_ppi: u16,
    pub png_pages: PngPages,
    pub pdf_tagged: bool,
    pub pdf_standard: Option<PdfStandard>,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            quick_path: "$root/build/$name.pdf".into(),
            on_save: false,
            png_ppi: 144,
            png_pages: PngPages::Current,
            pdf_tagged: true,
            pdf_standard: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum PngPages {
    Current,
    All,
    Range { first: usize, last: usize },
}

impl TryFrom<String> for PngPages {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "current" {
            return Ok(Self::Current);
        }
        if value == "all" {
            return Ok(Self::All);
        }
        let (first, last) = value.split_once('-').unwrap_or((&value, &value));
        if let (Ok(first), Ok(last)) = (first.trim().parse(), last.trim().parse())
            && first > 0
            && first <= last
        {
            return Ok(Self::Range { first, last });
        }
        Err("expected current, all, a one-based page, or a range such as 2-4")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum PdfStandard {
    #[serde(rename = "a-2b")]
    A2b,
    #[serde(rename = "a-2u")]
    A2u,
    #[serde(rename = "a-3b")]
    A3b,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CompilerConfig {
    pub root: Option<PathBuf>,
    pub font_paths: Vec<PathBuf>,
    pub system_fonts: bool,
    pub inputs: BTreeMap<String, String>,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            root: None,
            font_paths: Vec::new(),
            system_fonts: true,
            inputs: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LspConfig {
    pub enabled: bool,
    pub binary: PathBuf,
    pub root: Option<PathBuf>,
    pub formatter: Formatter,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            binary: "tinymist".into(),
            root: None,
            formatter: Formatter::Typstyle,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Formatter {
    #[default]
    Typstyle,
    Typstfmt,
}

impl Formatter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typstyle => "typstyle",
            Self::Typstfmt => "typstfmt",
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not locate the user configuration directory")]
    NoConfigDirectory,
    #[error("expected a configuration file path, got {}", .0.display())]
    InvalidPath(PathBuf),
    #[error("configuration I/O: {0}")]
    Io(#[from] io::Error),
    #[error("invalid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("ui.font_size must be between 11 and 16 points, got {0}")]
    FontSize(f32),
    #[error("editor.font_size must be between 10 and 24 points, got {0}")]
    EditorFontSize(f32),
    #[error("editor.line_height must be between editor.font_size + 4 and 42 points, got {0}")]
    EditorLineHeight(f32),
    #[error("export.png_ppi must be between 36 and 600, got {0}")]
    ExportPpi(u16),
    #[error("export.quick_path must end in .pdf")]
    ExportPath,
    #[error("assets_dir must be a directory relative to the project root")]
    AssetsDir,
    #[error("configuration watcher: {0}")]
    Watch(#[from] notify::Error),
}

impl Config {
    pub fn parse(toml: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(toml)?;
        if !(11.0..=16.0).contains(&config.ui.font_size) {
            return Err(ConfigError::FontSize(config.ui.font_size));
        }
        if !(10.0..=24.0).contains(&config.editor.font_size) {
            return Err(ConfigError::EditorFontSize(config.editor.font_size));
        }
        if !(config.editor.font_size + 4.0..=42.0).contains(&config.editor.line_height) {
            return Err(ConfigError::EditorLineHeight(config.editor.line_height));
        }
        if !(36..=600).contains(&config.export.png_ppi) {
            return Err(ConfigError::ExportPpi(config.export.png_ppi));
        }
        if !config.export.quick_path.ends_with(".pdf") {
            return Err(ConfigError::ExportPath);
        }
        if config.assets_dir.as_os_str().is_empty()
            || config.assets_dir.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            return Err(ConfigError::AssetsDir);
        }
        Ok(config)
    }
}

pub struct ConfigFile {
    path: PathBuf,
}

impl ConfigFile {
    pub fn locate() -> Result<Self, ConfigError> {
        let path = match std::env::var_os("FAST_CONFIG") {
            Some(path) => PathBuf::from(path),
            None => ProjectDirs::from("", "", "Fast")
                .ok_or(ConfigError::NoConfigDirectory)?
                .config_dir()
                .join("config.toml"),
        };
        Self::new(path)
    }

    pub fn new(path: PathBuf) -> Result<Self, ConfigError> {
        if path.file_name().is_none() {
            return Err(ConfigError::InvalidPath(path));
        }
        Ok(Self {
            path: std::path::absolute(path)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A missing configuration file uses defaults.
    pub fn load(&self) -> Result<Option<Config>, ConfigError> {
        match fs::read_to_string(&self.path) {
            Ok(contents) => Config::parse(&contents).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Watch the directory so an editor's atomic replacement of the file is picked up.
    /// The callback and subsequent reads run on the debouncer's worker thread.
    pub fn watch(
        &self,
        mut changed: impl FnMut(Result<Option<Config>, ConfigError>) + Send + 'static,
    ) -> Result<ConfigWatcher, ConfigError> {
        let (Some(parent), Some(name)) = (self.path.parent(), self.path.file_name()) else {
            return Err(ConfigError::InvalidPath(self.path.clone()));
        };
        fs::create_dir_all(parent)?;
        let parent = parent.canonicalize()?;
        let file = Self {
            path: parent.join(name),
        };
        let directory = parent.clone();
        let mut watcher = new_debouncer(
            Duration::from_millis(150),
            None,
            move |events: DebounceEventResult| match events {
                Ok(events)
                    if events.iter().any(|event| {
                        event.need_rescan()
                            || (!event.kind.is_access()
                                && event
                                    .paths
                                    .iter()
                                    .any(|path| path == &file.path || path == &directory))
                    }) =>
                {
                    changed(file.load());
                }
                Ok(_) => {}
                Err(errors) => {
                    for error in errors {
                        changed(Err(error.into()));
                    }
                }
            },
        )?;
        watcher.watch(&parent, RecursiveMode::NonRecursive)?;
        Ok(ConfigWatcher { _watcher: watcher })
    }
}

/// Keeps the configuration watch alive until dropped.
pub struct ConfigWatcher {
    _watcher: Debouncer<RecommendedWatcher, RecommendedCache>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_defaults_and_validates_overrides() -> Result<(), ConfigError> {
        assert_eq!(Config::parse("")?, Config::default());
        let config = Config::parse("[ui]\ntheme = 'intellij-light-classic'\nfont_size = 15")?;
        assert_eq!(config.ui.theme, Theme::IntelliJLightClassic);
        assert_eq!(config.ui.font_size, 15.0);
        assert!(matches!(
            Config::parse("[ui]\nfont_size = 17"),
            Err(ConfigError::FontSize(17.0))
        ));
        assert!(Config::parse("[ui]\nfont_size = nan").is_err());
        assert!(Config::parse("[ui]\ntheme = 'blue'").is_err());
        assert!(Config::parse("[ui]\nfontsize = 14").is_err());
        Ok(())
    }

    #[test]
    fn reloads_created_replaced_and_deleted_config() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let file = ConfigFile::new(dir.path().join("config.toml"))?;
        let (send, updates) = std::sync::mpsc::channel();
        let _watch = file.watch(move |result| drop(send.send(result)))?;
        let timeout = Duration::from_secs(5);
        assert_eq!(file.load()?, None);

        fs::write(file.path(), "[ui]\nfont_size = 14")?;
        let created = updates.recv_timeout(timeout)??;
        assert_eq!(
            created.as_ref().map(|config| config.ui.font_size),
            Some(14.0)
        );

        let replacement = dir.path().join("config.tmp");
        fs::write(&replacement, "[ui]\nfont_size = 15")?;
        fs::rename(&replacement, file.path())?;
        loop {
            let updated = updates.recv_timeout(timeout)??;
            if updated != created {
                assert_eq!(updated.map(|config| config.ui.font_size), Some(15.0));
                break;
            }
        }

        fs::remove_file(file.path())?;
        while updates.recv_timeout(timeout)??.is_some() {}
        Ok(())
    }
}
