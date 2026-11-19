# Fast

Fast is a native Typst IDE in development. See [the north star](docs/northstar.md) for the phased plan.

## Run

```sh
cargo run
cargo run -- path/to/main.typ
```

Use the document toolbar to create, open, and save files or to find and replace text. **New** (Command/Ctrl+N) opens an untitled tab. Use **Save** to choose its name and location. Standard shortcuts use Command on macOS and Ctrl on Windows and Linux. Use Command/Ctrl+Shift+S to save to another path and Command/Ctrl+Alt+Z to revisit a different redo branch.

## Configuration

Hover over the configuration status to see the path to `config.toml`. Set `FAST_CONFIG` to use a different file. Fast reads the file at startup and when it changes. A missing file uses the defaults. An invalid file leaves the last valid settings in place and shows the error in the status tooltip.

```toml
[ui]
theme = "darcula-classic"
font_size = 13
font_family = "inter"
density = "standard"
sidebar = "left"
preview_dock = "right"
tabs = "top"

[editor]
font_family = "system-mono"
font_size = 13
line_height = 22
ligatures = false
```

**Settings** updates the theme, typefaces, font sizes, density, sidebar side, preview dock, and tab placement live. Edit `config.toml` for startup defaults. UI families are `inter`, `system-mono`, and `serif`. Editor families are `system-mono` and `mono`. Density accepts `compact`, `standard`, or `comfortable`. Preview can open on the `left`, `right`, `bottom`, or in a `tab`.

## Workbench

Open files from the Project tree or **Open**. Each tab keeps its edits and undo history as you switch. A dot marks unsaved changes, and **Main** identifies the pinned Typst file. Right-click a Typst tab or project file to pin or unpin it while Tinymist is running.

Use the tool strips or Alt+1 for Project, Alt+2 for Outline, Alt+6 for Problems, and Alt+P for Preview. The arrow button in a tool header moves it between docks. Drag a divider to resize a dock or to split its space between tools. Fast saves dock locations and ratios beside the configuration in a hidden workbench file.

Use Ctrl+P for Quick Open, Ctrl+Shift+P for the Command Palette, and Ctrl+Shift+F for Find in Files. Command also works for these shortcuts on macOS. The Project tree starts in the first file’s directory or configured root. Use **Folder** to choose another root. Search respects `.gitignore` and includes unsaved edits in open files. Results are grouped by file, with a live excerpt and a jump to the match.

Find in Files skips binary and media files and files larger than 8 MiB. The results show how many files were skipped during the search.

## Language tools

Fast starts [Tinymist](https://github.com/Myriad-Dreamin/tinymist) when you open a Typst file. It looks up `tinymist` on `PATH` and uses the file’s directory as the initial workspace. Set these options when a project needs a different executable or root.

```toml
[lsp]
enabled = true
binary = "tinymist"
formatter = "typstyle"
# root = "/path/to/project"
```

The **Language** menu includes Problems, Outline, formatting, navigation, rename, quick fixes, and main-file pinning. Use Ctrl+Space for completion, F1 for hover documentation, F2 for rename, F12 for definition, and Shift+F12 for references. Workspace actions that affect other files show the paths and ask before writing.

## Live preview and export

Open a `.typ` file and choose **Preview** to show the rendered document beside the editor. Unsaved edits update the preview. Double-click rendered text to jump to its source. Scroll or drag to pan, and pinch or use Ctrl+wheel to zoom. **View** offers fit width, fit page, 100%, two-page spreads, dark paper, and following the editor caret. A compile error keeps the last rendered pages visible and shows the error below them.

Preview and export use the pinned main file when one is set. The embedded compiler and Tinymist share the following settings. Relative font paths are resolved from the compiler root.

```toml
[compiler]
# root = "/path/to/project"
system_fonts = true
# font_paths = ["fonts"]
# inputs = { edition = "draft" }
```

Use **Export** to choose PDF, PNG, SVG, HTML, Markdown, or plain text. Markdown and plain text use Tinymist. SVG uses the current preview page. PNG uses the current page by default, with numbered files for multi-page exports. Command/Ctrl+Shift+E writes a PDF to `build/<name>.pdf` by default. Use **Export** to replace an existing manual export. Enabling `on_save` updates the configured PDF whenever a Typst file is saved.

```toml
[export]
quick_path = "$root/build/$name.pdf"
on_save = false
png_ppi = 144
png_pages = "current" # "all", "2", or "2-4" also work
pdf_tagged = true
# pdf_standard = "a-2b" # "a-2u" and "a-3b" are also available
```

## Headless verification

```sh
cargo check --workspace
cargo test --workspace
python3 tests/visual/check.py
python3 tests/visual/editor.py
cargo test -p fast-lsp --test tinymist -- --ignored
cargo build -p fast
python3 tests/visual/language.py
python3 tests/visual/preview.py
python3 tests/visual/workbench.py
cargo test --release -p fast-preview --test incremental -- --ignored --nocapture
```

The visual check uses Rune’s GPU renderer and a temporary configuration file. It writes PNGs and element trees to `target/visual`, checks geometry and contrast at 1× and 2×, and replays keyboard and pointer theme changes. Open the PNGs and their matching JSON files to inspect the layout.

The editor check builds a release binary with Rune’s frame profiler. It opens a generated 20,000-line Typst project, checks find and replace, undo, redo, and save, and reports p99 update plus render-encoding time across 200 scripted edits and maximum CPU time per virtualized scroll commit. The captures and timing logs are in `target/visual/editor`.

The language checks require Tinymist on `PATH`. The Rust integration test also accepts `FAST_TEST_TINYMIST` and exercises recovery after a process crash. The Rune replay saves screenshots and element trees to `target/visual/language`.

The preview replay saves its captures to `target/visual/preview` and checks rendering, click-to-source, and exported files. Use `--native-only` to skip the Tinymist formats. The release benchmark measures incremental compilation and visible-page rasterization across 20 edits to a 20-page document.

The workbench replay saves dark and light captures to `target/visual/workbench`. It exercises project navigation, dock moves and resizing, dirty tabs, keyboard pickers, workspace search, and live settings.
