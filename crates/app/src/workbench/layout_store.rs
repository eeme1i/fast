use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use fast_core::{document::Document, file};
use rune::{Latest, Waker};
use serde::{Deserialize, Serialize};

use super::dock::{Dock, Layout, Tool};

#[derive(Default)]
pub(super) struct Store {
    path: Option<PathBuf>,
    read: Latest<Result<Option<Preferences>, String>>,
    write: Latest<Result<(), String>>,
    changed: bool,
}

#[derive(Serialize, Deserialize)]
struct Preferences {
    ratios: [f32; 3],
    weights: [f32; 6],
    positions: BTreeMap<Tool, Dock>,
}

impl Store {
    pub fn setup(&mut self, config: Option<&Path>, waker: Waker) {
        self.read.set_waker(waker.clone());
        self.write.set_waker(waker);
        let Some(config) = config else { return };
        let mut name = OsString::from(".");
        name.push(config.file_name().unwrap_or_default());
        name.push(".workbench.toml");
        let path = config.with_file_name(name);
        self.path = Some(path.clone());
        self.read
            .request(path, |path| match fs::read_to_string(&path) {
                Ok(source) => toml::from_str::<Preferences>(&source)
                    .map(Some)
                    .map_err(|error| format!("Couldn't read workbench layout. {error}")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(format!("Couldn't read {}. {error}", path.display())),
            });
    }

    pub fn save(&mut self, layout: &Layout) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.changed = true;
        let prefs = Preferences {
            ratios: layout.ratios,
            weights: layout.weights,
            positions: Tool::ALL
                .into_iter()
                .filter_map(|tool| Some((tool, layout.placements[tool as usize]?)))
                .collect(),
        };
        self.write.request((path, prefs), |(path, prefs)| {
            let text = toml::to_string(&prefs).map_err(|error| error.to_string())?;
            file::save(&path, &Document::new(&text).snapshot())
                .map(|_| ())
                .map_err(|error| format!("Couldn't save {}. {error}", path.display()))
        });
    }

    pub fn poll(&mut self, layout: &mut Layout, error: &mut Option<String>) -> bool {
        if let Some(result) = self.read.take() {
            match result {
                Ok(Some(prefs)) if !self.changed => {
                    for (index, ratio) in prefs.ratios.into_iter().enumerate() {
                        if ratio.is_finite() {
                            layout.ratios[index] = ratio.clamp(0.15, 0.68);
                        }
                    }
                    for (index, weight) in prefs.weights.into_iter().enumerate() {
                        if weight.is_finite() {
                            layout.weights[index] = weight.clamp(0.2, 5.0);
                        }
                    }
                    for (tool, dock) in prefs.positions {
                        layout.placements[tool as usize] = Some(dock);
                    }
                    return true;
                }
                Err(reason) => {
                    *error = Some(reason);
                    return true;
                }
                _ => {}
            }
        }
        if let Some(result) = self.write.take() {
            *error = result.err();
            return true;
        }
        false
    }
}
