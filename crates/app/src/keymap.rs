use std::collections::BTreeMap;

use fast_editor::{EditCommand, VIEWPORT};
use rune::{ChordKey, ChordMod, Key, KeyEvent, KeyPlatform, Keybind, Tx};

use crate::{
    State, files, language, preview,
    workbench::{self, Tool},
};

pub(crate) struct Action {
    pub id: &'static str,
    pub name: &'static str,
    default: &'static str,
    editor_only: bool,
    pub run: fn(&mut State, &mut Tx),
    pub available: fn(&State) -> bool,
}

impl Action {
    const fn new(
        id: &'static str,
        name: &'static str,
        default: &'static str,
        run: fn(&mut State, &mut Tx),
    ) -> Self {
        Self {
            id,
            name,
            default,
            editor_only: false,
            run,
            available: |_| true,
        }
    }

    const fn when(mut self, available: fn(&State) -> bool) -> Self {
        self.available = available;
        self
    }

    const fn editor(mut self) -> Self {
        self.editor_only = true;
        self
    }
}

const LANGUAGE: fn(&State) -> bool = |s| s.language.ready() && preview::available(s);

pub(crate) static ACTIONS: &[Action] = &[
    Action::new("new-file", "New file", "mod+n, ctrl+n", files::create).when(|s| !s.files.busy()),
    Action::new("open-file", "Open file", "mod+o", files::open),
    Action::new(
        "open-project",
        "Open project folder",
        "",
        workbench::project::choose_root,
    ),
    Action::new(
        "quick-open",
        "Quick open file",
        "mod+p, ctrl+p",
        workbench::quick_open,
    ),
    Action::new(
        "command-palette",
        "Command Palette",
        "mod+shift+p, ctrl+shift+p",
        workbench::commands,
    ),
    Action::new(
        "find-in-files",
        "Find in Files",
        "mod+shift+f, ctrl+shift+f",
        workbench::project::open_search,
    ),
    Action::new("find", "Find in document", "mod+f", |s, tx| {
        s.editor.find(false, tx)
    }),
    Action::new(
        "replace",
        "Replace in document",
        "mod+h, mod+alt+f",
        |s, tx| s.editor.find(true, tx),
    ),
    Action::new("save", "Save file", "mod+s", |s, tx| {
        files::save(s, false, tx)
    }),
    Action::new("save-as", "Save as", "mod+shift+s", |s, tx| {
        files::save(s, true, tx)
    }),
    Action::new("close-tab", "Close tab", "mod+w, ctrl+w", |s, tx| {
        files::close(s, s.files.active, tx)
    }),
    Action::new(
        "toggle-project",
        "Toggle Project files",
        "alt+1",
        |s, tx| workbench::toggle(s, Tool::Project, tx),
    ),
    Action::new("toggle-outline", "Toggle Outline", "alt+2", |s, tx| {
        workbench::toggle(s, Tool::Outline, tx)
    }),
    Action::new("toggle-problems", "Toggle Problems", "alt+6", |s, tx| {
        workbench::toggle(s, Tool::Problems, tx)
    }),
    Action::new("toggle-compiler", "Toggle Compiler output", "", |s, tx| {
        workbench::toggle(s, Tool::Compiler, tx)
    }),
    Action::new("toggle-preview", "Toggle Preview", "alt+p", preview::toggle)
        .when(preview::available),
    Action::new("export", "Export document", "", preview::export_dialog).when(preview::available),
    Action::new(
        "quick-export",
        "Quick PDF export",
        "mod+shift+e",
        preview::quick_export,
    )
    .when(preview::available),
    Action::new("format", "Format document", "", |s, tx| {
        language::invoke(s, language::Command::Format, tx)
    })
    .when(LANGUAGE),
    Action::new("definition", "Go to definition", "f12", |s, tx| {
        language::invoke(s, language::Command::Definition, tx)
    })
    .editor()
    .when(LANGUAGE),
    Action::new("references", "Find references", "shift+f12", |s, tx| {
        language::invoke(s, language::Command::References, tx)
    })
    .editor()
    .when(LANGUAGE),
    Action::new("rename", "Rename symbol", "f2", |s, tx| {
        language::invoke(s, language::Command::Rename, tx)
    })
    .editor()
    .when(LANGUAGE),
    Action::new("hover", "Show hover documentation", "f1", |s, tx| {
        language::invoke(s, language::Command::Hover, tx)
    })
    .editor()
    .when(LANGUAGE),
    Action::new(
        "complete",
        "Show completions",
        "ctrl+space, mod+space",
        |s, tx| language::invoke(s, language::Command::Complete, tx),
    )
    .editor()
    .when(|s| LANGUAGE(s) && s.editor.document().selections().len() == 1),
    Action::new(
        "code-actions",
        "Show code actions",
        "mod+period",
        |s, tx| language::invoke(s, language::Command::Actions, tx),
    )
    .editor()
    .when(LANGUAGE),
    Action::new("pin-main", "Pin main file", "", language::pin_dialog).when(LANGUAGE),
    Action::new("unpin-main", "Unpin main file", "", |s, tx| {
        language::invoke(s, language::Command::Unpin, tx)
    })
    .when(|s| s.language.pinned.is_some()),
    Action::new("settings", "Settings", "", workbench::settings::open),
    Action::new(
        "select-next-occurrence",
        "Select next occurrence",
        "mod+d, ctrl+d",
        |s, tx| s.editor.select_next_occurrence(tx),
    )
    .editor(),
    Action::new("undo", "Undo", "mod+z", |s, tx| {
        s.editor.run(EditCommand::Undo, tx)
    })
    .editor(),
    Action::new("redo", "Redo", "mod+shift+z, mod+y", |s, tx| {
        s.editor.run(EditCommand::Redo, tx)
    })
    .editor(),
    Action::new(
        "redo-alternate",
        "Redo alternate branch",
        "mod+alt+z",
        |s, tx| s.editor.run(EditCommand::RedoAlternate, tx),
    )
    .editor(),
    Action::new("select-all", "Select all", "mod+a", |s, tx| {
        s.editor.run(EditCommand::SelectAll, tx)
    })
    .editor(),
    Action::new("copy", "Copy", "mod+c", |s, tx| {
        s.editor.run(EditCommand::Copy, tx)
    })
    .editor(),
    Action::new("cut", "Cut", "mod+x", |s, tx| {
        s.editor.run(EditCommand::Cut, tx)
    })
    .editor(),
    Action::new("paste", "Paste", "mod+v", |s, tx| {
        s.editor.run(EditCommand::Paste, tx)
    })
    .editor(),
];

pub(crate) struct Keymap(Vec<Keybind>);

impl Default for Keymap {
    fn default() -> Self {
        Self(
            ACTIONS
                .iter()
                .map(|action| Keybind::new(action.default))
                .collect(),
        )
    }
}

impl Keymap {
    pub fn from_config(overrides: &BTreeMap<String, String>) -> Result<Self, String> {
        let mut map = Self::default();
        for (id, spec) in overrides {
            let index = ACTIONS
                .iter()
                .position(|action| action.id == id)
                .ok_or_else(|| format!("Unknown keymap action {id}"))?;
            let binding: Keybind = spec
                .parse()
                .map_err(|error| format!("Keymap {id}. {error}"))?;
            if binding.chords().iter().any(|chord| {
                !matches!(chord.key(), ChordKey::Function(_))
                    && ![
                        ChordMod::Primary,
                        ChordMod::Ctrl,
                        ChordMod::Alt,
                        ChordMod::Super,
                    ]
                    .into_iter()
                    .any(|modifier| chord.has(modifier))
            }) {
                return Err(format!("Keymap {id} requires a modifier or function key"));
            }
            map.0[index] = binding;
        }
        for (i, binding) in map.0.iter().enumerate() {
            for (j, other) in map.0[..i].iter().enumerate() {
                for platform in [KeyPlatform::Mac, KeyPlatform::Linux, KeyPlatform::Windows] {
                    for chord in binding.chords_on(platform) {
                        if other
                            .chords_on(platform)
                            .iter()
                            .any(|other| chord.same_keys(other, platform))
                        {
                            return Err(format!(
                                "Keymap conflict for {} between {} and {}",
                                chord.label_on(platform),
                                ACTIONS[j].id,
                                ACTIONS[i].id
                            ));
                        }
                    }
                }
            }
        }
        Ok(map)
    }

    pub fn label(&self, id: &str) -> String {
        ACTIONS
            .iter()
            .zip(&self.0)
            .find(|(action, _)| action.id == id)
            .map_or_else(String::new, |(_, binding)| binding.label())
    }

    pub fn help(&self, id: &str, label: &str) -> String {
        let chord = self.label(id);
        if chord.is_empty() {
            label.to_owned()
        } else {
            format!("{label} ({chord})")
        }
    }
}

pub(crate) fn key(state: &mut State, event: &KeyEvent, tx: &mut Tx, in_editor: bool) -> bool {
    let mods = event.mods;
    if !(mods.alt
        || mods.cmd
        || mods.ctrl
        || mods.logo
        || matches!(event.key, Key::Command(_) | Key::Function(_)))
        || !workbench::shortcuts_available(state)
    {
        return false;
    }
    let action = ACTIONS
        .iter()
        .zip(&state.keymap.0)
        .find(|(action, binding)| {
            (!action.editor_only || (in_editor && tx.focused() == Some(VIEWPORT)))
                && binding.matches_tx(event, tx)
                && (action.available)(state)
        })
        .map(|(action, _)| action);
    if let Some(action) = action {
        (action.run)(state, tx);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Keymap;
    use std::collections::BTreeMap;

    #[test]
    fn remaps_and_disables_chords_and_rejects_platform_alias_conflicts() {
        assert!(Keymap::from_config(&BTreeMap::new()).is_ok());
        let overrides = BTreeMap::from([
            ("select-next-occurrence".into(), "alt+k".into()),
            ("close-tab".into(), "none".into()),
        ]);
        let map = Keymap::from_config(&overrides).unwrap_or_else(|error| panic!("{error}"));
        assert!(!map.label("select-next-occurrence").is_empty());
        assert!(map.label("close-tab").is_empty());
        let overrides = BTreeMap::from([("select-next-occurrence".into(), "ctrl+s".into())]);
        let Err(error) = Keymap::from_config(&overrides) else {
            panic!("Ctrl+S conflicts with the primary Save chord on Linux and Windows");
        };
        assert!(error.contains("select-next-occurrence") && error.contains("save"));
    }
}
