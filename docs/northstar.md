# NORTH STAR: Native Rust Typst IDE

## 1. Mission & Product Identity

Build a fast, single-binary, GPU-accelerated desktop IDE dedicated to writing and publishing Typst documents in Rust, using [`anomalyco/rune`](https://github.com/anomalyco/rune) as the native UI framework.

The target aesthetic and ergonomics are **classic JetBrains (pre-New UI)**: high information density, crisp 1px structural borders, unmistakable active-pane focus contrast, persistent tool-window strips, zero wasted chrome, and instant keyboard-first response.

### Core Design Principles

1. **Native Speed Over Web Convenience:** Zero Electron, zero embedded Chromium, zero WebViews. Text editing, GPU compositing, incremental Typst preview rendering, and LSP communication run in native Rust.
2. **Information Density Over Whitespace:** Every pixel earns its keep. Toolbars, tabs, tree rows, and status bars stay compact (22–26 pt row heights by default, user-scalable).
3. **Zero-Latency Tactile Feel:** Typing, cursor movement, scrolling, and pane resizing never wait on disk I/O, LSP round-trips, or spring animations.
4. **Honest Visual Hierarchy:** Clear contrast between editor canvas, tool windows, headers, active tabs, and borders. Avoid low-contrast glassmorphism, oversized padding, or ornamental gradients.
5. **No Slop:** No placeholder buttons, no stubbed settings that do nothing, no silent `.unwrap()` crashes on user files or LSP responses, and no giant monolithic files.

---

## 2. Hard Technical Constraints & Stack

| Layer | Technology / Crate | Rationale |
| --- | --- | --- |
| **UI & GPU Rendering** | `rune` (`anomalyco/rune`, pinned `dev` commit rev) | Native GPU renderer, Parley-backed text shaping, OKLCH theming, live canvas transforms, and headless PNG + element-tree inspection for agent self-verification. |
| **Text Buffer** | `ropey` | O(log N) edits and line indexing for large multi-thousand-line `.typ` files without allocation churn. |
| **Syntax & Fast Local AST** | `typst-syntax` | Official Typst incremental parser for instant local syntax highlighting, bracket matching, and folding before LSP semantic tokens arrive. |
| **Language Server** | `tinymist` (external binary via stdio JSON-RPC + `lsp-types`) | Authoritative Typst LSP providing completions, diagnostics, hover, signature help, formatting (`typstyle`/`typstfmt`), rename, outline, and export commands. |
| **Live Preview Engine** | Embedded Typst crates (`typst`, `typst-render`, `typst-kit`) | In-process incremental compilation via `comemo` directly from memory snapshots — zero disk writes per keystroke, rendered straight to GPU textures. |
| **Multi-Format Export** | `typst-pdf`, `typst-svg`, `typst-render`, `typst-html` + Tinymist LSP export commands | Fast native export to PDF, PNG, SVG, and HTML, plus Markdown/Text via Tinymist (`tinymist.exportMarkdown`, `tinymist.exportText`). |
| **Async Runtime & I/O** | `tokio` (background thread), `crossbeam-channel`, `notify` | Isolates LSP stdio, file watching, compilation, and rasterization from Rune's UI thread. |
| **Config & Persistence** | `serde`, `toml`, `directories` | Human-editable `config.toml` with live hot-reload and per-workspace `.typst-ide.toml` overrides. |

### Rune Integration Rules (Read Before Writing UI Code)

Before implementing any UI, read `examples/hello`, `crates/rune/README.md`, and `crates/rune-kit/README.md` in `anomalyco/rune`. Pin a specific commit SHA on Rune's `dev` branch in `Cargo.toml` (`dev` is never rewritten).

* **Kill unnecessary motion on editing actions:** Rune animates layout/property changes by default; wrap typing, cursor moves, selection updates, popup list filtering, and pane dragging in `Tx::instant()` or `El::instant_changes()` so editing feels razor-sharp. Reserve subtle motion strictly for reduced-motion-safe overlays if used at all.
* **Respect Rune's Flexbox contracts:** `.grow(g)` uses `flex-basis: 0` (always bound an ancestor's height via `.h(..)` or `.h_full()`), put `.min_h(0.0)` on every nested flex column above a `.scroll(..)` viewport, and only attach `.virtualize(..)` to a `.growth(GrowthSpec)` stack that is the first child of its scroll viewport.
* **Use Live Camera for Preview Pan/Zoom:** Store preview viewport pan and zoom in Rune `Live` values (`live_offset`, `live_scale`) and signal `tx.live_only()` in scroll/pinch/drag handlers so zooming pages runs at native frame rate without re-running view diffs or layout.
* **Custom Theme Crate:** Do not ship OpenCode's default styling. Implement a dedicated `crates/theme` providing semantic OKLCH tokens (`Kit` / `tok`) modeled on classic IntelliJ Darcula (dark) and IntelliJ Light (light).

---

## 3. Workspace Architecture

Enforce strict crate boundaries. UI code never parses JSON-RPC or runs Typst compilation directly; core editor state never imports `rune`.

```text
typst-ide/
├── Cargo.toml
├── NORTH_STAR.md
├── crates/
│   ├── core/          # Buffer (ropey), cursors, selections, undo tree, UTF-8/UTF-16 codec, search, config
│   ├── theme/         # Classic IDE OKLCH token sets (Darcula / Light Classic), density & font metrics
│   ├── lsp/           # Async tinymist client, JSON-RPC transport, process supervisor, custom commands
│   ├── preview/       # Incremental Typst World, worker pool, HiDPI page rasterizer, source <-> preview map
│   ├── editor/        # Rune virtualized code view, gutter, syntax + semantic spans, popups, find bar
│   └── app/           # Workbench shell, docking/split manager, tool windows, command palette, headless harness
└── tests/
    └── visual/        # Headless Rune PNG & element-tree regression tests

```

### Threading & Data-Flow Model

1. **UI Thread (Rune):** Owns view state, handles keyboard/mouse events, runs `view(&State, &mut Cx) -> El`, and composites GPU frames. Never performs blocking file I/O, compilation, or network calls.
2. **LSP Worker (Tokio):** Manages `tinymist` child process over stdio, coalesces incremental `didChange` deltas, tracks document versions, and pushes typed events (`Diagnostics`, `Completions`, `Hover`, `Outline`, `CompileStatus`) to the UI queue.
3. **Preview Worker Pool:** Receives debounced (e.g., 16–30 ms coalesced) in-memory buffer snapshots + pinned main-file path. Runs incremental Typst compilation, caches compiled `PagedDocument` frames, and rasterizes only visible pages (±1 neighbor) at `device_scale * zoom` into RGBA bitmaps handed back as GPU textures.

---

## 4. Feature Requirements

### 4.1 Tinymist LSP Integration

* **Process Management:** Auto-detect `tinymist` on `PATH` or accept a user-configured binary path. Display LSP status (Starting / Ready / Compiling / Crashed) in the status bar via `tinymist/compileStatus`. Auto-restart with exponential backoff on unexpected exit.
* **Incremental Sync:** Full UTF-8 ↔ UTF-16 offset translation for LSP ranges; send incremental `textDocument/didChange` edits on every buffer mutation.
* **In-Editor Suggestions (Completion):**
* Trigger on typing identifiers, `#`, `.`, `(`, `@`, `<`, file paths, or manual `Ctrl+Space`.
* Render a crisp, high-density completion popup anchored at the caret: icon/kind badge, label, detail/type signature, and an optional side documentation panel showing Tinymist's `tidy`-formatted docs.
* Support `additionalTextEdits`, snippet tab-stops (e.g., function arguments), and fuzzy prefix filtering without flickering (`El::instant_changes()`).


* **Full Language Intelligence:** Diagnostics (inline squiggles + gutter icons + Problems tool window), Hover tooltips, Signature Help (active parameter highlighting), Go to Definition / References, Rename Symbol (including embedded paths), Code Actions / Quick Fixes (e.g., heading level adjustments, equation block/inline conversion), Document & Workspace Symbols (Outline), Folding Ranges, Inlay Hints, Color Swatches/Pickers, and Document Formatting (`typstyle` / `typstfmt`).
* **Multi-File Project Support:** First-class `tinymist.pinMain` / unpin action accessible from tab context menu, file tree, command palette, and status bar so editing chapter partials always compiles and checks against the project's root `main.typ`.

### 4.2 Fast Real-Time Preview & Multi-Format Export

* **In-Memory Incremental Preview:** Typing updates the preview in milliseconds with zero writes to disk.
* **Compiler Parity:** Share a single compiler configuration block (`root`, `font_paths`, `system_fonts`, `sys.inputs`) between the embedded preview compiler and Tinymist initialization options so fonts, packages, and errors never diverge.
* **Interactive Preview Canvas:**
* Smooth 120Hz pan and zoom via Rune's live camera (`live_offset`, `live_scale`, `tx.live_only()`), with crisp re-rasterization on zoom settle.
* Fit-width, fit-page, 100%, continuous vertical scroll, and two-page spread modes.
* Optional dark-mode page inversion (smart luminance inversion that preserves readability without tinting images harshly).
* **Bidirectional Sync:** Moving the editor cursor optionally scrolls the preview to the corresponding page/position; `Ctrl+Click` (or double-click) on any rendered preview element jumps the editor caret directly to the originating Typst source span.


* **Fast Multi-Format Export:**
* One-keystroke Quick Export (`Ctrl+Shift+E`) and an Export Dialog supporting:
* **PDF** (with optional PDF/A standard & tagged output via `typst-pdf` / `tinymist.exportPdf`)
* **PNG** (configurable PPI, single page / page range / all pages via `typst-render` / `tinymist.exportPng`)
* **SVG** (via `typst-svg` / `tinymist.exportSvg`)
* **HTML** (via `typst-html` / `tinymist.exportHtml`)
* **Markdown & Plain Text** (via `tinymist.exportMarkdown` / `tinymist.exportText`)


* Support export triggers (`manual`, `on_save`) and configurable output directory/naming templates (`$root/build/$name.pdf`) without blocking editing.



### 4.3 Classic High-Density Professional UI & Customization

Model the spatial layout on pre-redesign JetBrains IDEs:

```text
+-----------------------------------------------------------------------------------+
| [Menu / Quick Toolbar: Project | Main: main.typ | Export | Compile: 14ms | Zoom]  |
+----+----------------------+--------------------------------+----------------------+
| P  | PROJECT TREE         | [main.typ *] [ch1.typ] [x]     | PREVIEW (110% Fit)   |
| R  | v src/               |  1 | #set page(paper: "a4")    |  +----------------+  |
| O  |   - main.typ [PIN]   |  2 | #import "defs.typ": *     |  |                |  |
| J  |   - ch1.typ          |  3 |                           |  |  1. Intro      |  |
|    |   - refs.bib         |  4 | = Introduction            |  |  Lorem ipsum   |  |
| O  |----------------------|  5 | The proof follows from    |  |  ...           |  |
| U  | OUTLINE              |  6 | @einstein1905.            |  |                |  |
| T  | - 1. Introduction    |                                |  +----------------+  |
|    | - 2. Methodology     |--------------------------------|                      |
|    |                      | PROBLEMS (2) | COMPILER LOG    |                      |
|    |                      | [E] ch1.typ:42 unknown var `x` |                      |
+----+----------------------+--------------------------------+----------------------+
| main.typ (pinned) | Ln 6, Col 14 | UTF-8 | LF | Typst 0.15 | Tinymist: OK (14 ms) |
+-----------------------------------------------------------------------------------+

```

* **Dockable Tool Windows:** Project Files, Document Outline, Problems/Diagnostics, Compiler Output, and Workspace Search can be docked to **Left**, **Right**, or **Bottom** strips, split vertically within a dock, resized via 4px hit-target / 1px visual dividers, or collapsed with single-chord shortcuts (`Alt+1` Project, `Alt+2` Outline, `Alt+6` Problems, `Alt+P` Preview).
* **Full User Customization (`config.toml` + Settings UI):**
* **Typography:** Independent `editor.font_family`, `editor.font_size`, `editor.line_height`, `editor.ligatures`, `ui.font_family`, `ui.font_size`, `ui.density` (`compact` | `standard` | `comfortable`).
* **Layout:** Sidebar position (`left` | `right`), Preview dock position (`right` | `left` | `bottom` | `tab`), tool-window visibility & saved split ratios, tab bar placement.
* **Theme:** Built-in `Darcula Classic`, `IntelliJ Light Classic`, and `High Contrast`, plus user OKLCH color token overrides in `config.toml`.
* **Keybindings:** Remappable action table via TOML with conflict detection.



### 4.4 Expected Text Editor Features

All core editor features must be genuinely solid — no half-working text boxes:

* **Selection & Multi-Cursor:** Mouse drag, double-click word, triple-click line, `Shift+Arrows`, `Ctrl+D` (add next occurrence), `Alt+Click` (add caret), column/box selection, move lines up/down (`Alt+Up/Down`), duplicate line/selection (`Ctrl+D` or `Ctrl+Shift+D` per keymap), indent/dedent blocks.
* **Editing & Navigation:** Unlimited undo/redo tree with typing-burst coalescing, soft wrap toggle with hanging indent, smart bracket/delimiter auto-close & surround (`$ $`, `[ ]`, `{ }`, `( )`, `" "`), toggle line/block comments (`Ctrl+/`), matching bracket highlight, go-to-line (`Ctrl+G`), navigation history back/forward (`Alt+Left/Right`).
* **Search & Replace:**
* In-file Find/Replace bar (`Ctrl+F` / `Ctrl+H`): literal, regex, case-sensitive, whole-word, preserve-case replace, match count (`3/47`), and overview scrollbar ticks.
* Workspace-wide Find in Files (`Ctrl+Shift+F`) ignoring `.gitignore` entries, displaying grouped results in a bottom tool window with live preview.
* Fuzzy Quick Open (`Ctrl+P`) and Command Palette (`Ctrl+Shift+P`).


* **File Hygiene:** Dirty indicators on tabs, atomic save (write temp + rename), external modification watcher with non-destructive reload/diff prompt, autosave option, trailing-whitespace trim on save, and smart image/asset paste (saves clipboard PNG into configured assets folder and inserts `#image("...")`).

---

## 5. Performance Budgets & Quality Gates

Every milestone must pass these measurable targets:

| Metric | Target Budget | Measurement Method |
| --- | --- | --- |
| **Cold startup to interactive window** | Less than 250 ms | Release build wall-clock to first painted frame |
| **Keystroke-to-frame latency** | Less than 8 ms (p99) on a 20,000-line `.typ` file | Headless scripted input + frame timing benchmark |
| **Virtualized scroll commit** | Less than 4 ms per scroll frame | Virtualized editor line list (cf. `rune-kit` 50k benchmark) |
| **Incremental preview update** | Less than 50 ms typical edit (compile + visible page raster) | In-process `typst` + `typst-render` benchmark on a 20-page document |
| **Idle RAM footprint** | Less than 150 MB (including preview cache, excluding `tinymist`) | OS RSS measurement after opening a 10-file project |
| **Compiler cleanliness** | Zero warnings | `cargo clippy --workspace --all-targets -- -D warnings` |

---

## 6. Phased Implementation Plan

Deliver each phase in vertical slices with clean builds, tests, and headless visual inspection. Phase 4 takes priority after Phase 2. Its preview and export tools begin in the single-document editor, with docking to follow in Phase 3.

### Phase 0 — Foundation & Headless Verification Harness

* Initialize Cargo workspace with the 6-crate structure and pinned `anomalyco/rune` `dev` rev.
* Boot a minimal Rune window with the `Darcula Classic` and `IntelliJ Light Classic` OKLCH theme tokens.
* Implement the **headless verification harness** (renders PNGs at 1x and 2x DPI, dumps the Rune element tree, replays scripted keystrokes/clicks).
* Implement `config.toml` loading, defaults, validation, and file-watcher hot reload.

### Phase 1 — Core Text Engine & Editor View

* Build `crates/core`: `ropey` buffer, multi-cursor selection model, undo/redo transaction log, UTF-8 ↔ UTF-16 position mapper, atomic file I/O, and search/replace engine.
* Build `crates/editor`: virtualized line renderer using Rune `rich(text, spans)` and `.virtualize(..)`, `typst-syntax` highlighting, line-number gutter, active-line highlight, selection rectangles, blinking caret, smart bracket pairing, and in-file Find/Replace bar.
* **Gate:** Open, edit, search, undo/redo, and save a 20,000-line `.typ` file within the 8 ms keystroke budget.

### Phase 2 — Tinymist LSP Integration

* Implement `crates/lsp`: spawn `tinymist`, handle initialization, `didOpen`/`didChange`/`didSave`/`didClose`, diagnostics, completions, hover, signature help, goto def/refs, rename, formatting, document symbols (Outline), folding, semantic tokens, `tinymist.pinMain`, and `tinymist/compileStatus`.
* Wire completion, hover, signatures, outline, and clickable problems into the single-document editor. Expose main-file pinning through the language toolbar and status bar.
* **Gate:** Integration tests against a real `tinymist` process verify completions, diagnostics, `pinMain` on a multi-file fixture, and automatic recovery after `SIGKILL` of `tinymist`.

### Phase 3 — Classic IDE Workbench & Customization

* Implement the docking/split layout engine with draggable dividers, collapsible Left/Right/Bottom tool-window strips, tab bar with dirty/pin badges, and persistent status bar. Move Outline and Problems into dockable tool windows.
* Add Project File Tree, Command Palette (`Ctrl+Shift+P`), Fuzzy File Open (`Ctrl+P`), Workspace Search (`Ctrl+Shift+F`), and a live Settings dialog (font family, font size, UI density, sidebar side, theme switcher).
* **Gate:** Headless visual snapshots in both Dark and Light themes confirm crisp 1px borders, readable contrast, non-overlapping text, and functional sidebar relocation (left ↔ right).

### Phase 4 — Native GPU Live Preview & Multi-Format Export

* Build `crates/preview`: in-memory incremental Typst `World`, background compile/raster worker, LRU page texture cache, HiDPI scaling, and bidirectional source ↔ preview coordinate mapping.
* Mount the Preview Pane on Rune's live camera (`live_offset`, `live_scale`, `tx.live_only()`) with toolbar controls (zoom, fit mode, page indicator, dark inversion, follow-cursor toggle).
* Build the Export Dialog & Quick Export supporting PDF, PNG, SVG, HTML, Markdown, and Plain Text.
* **Gate:** Sub-50 ms incremental preview update benchmark; click-to-source jump verified via headless scripted click test; exported PDF/PNG/SVG/HTML verified non-empty and valid.

### Phase 5 — Ergonomic Polish & Hardening

* Add multi-cursor (`Ctrl+D`, `Alt+Click`), block/column selection, image clipboard paste with `#image(...)` insertion, external file-change handling, workspace session restoration (open tabs, splits, pinned main file, scroll positions), and keymap customization.
* Run full profiling pass, memory-leak check on long editing sessions, and cross-platform build checks (macOS, Linux, Windows MSVC).

---

## 7. Anti-Slop Directives for the Coding Agent

Follow these rules on every single change:

1. **Always Verify Visually with Rune Headless Mode:** After every UI change, run the headless harness to render PNGs and inspect both the generated PNGs and the dumped element tree. Check specifically for clipped labels, collapsed scroll viewports (missing `.min_h(0.0)`), misaligned baselines, low text contrast, and jittery focus borders.
2. **Never Fake Functionality:** Do not render a menu item, toolbar icon, or setting toggle unless its backing behavior is fully wired and tested.
3. **No Silent Failures:** Every I/O, JSON-RPC, font-loading, or compilation error must surface as either a structured diagnostic, a non-intrusive status/log entry, or an actionable error toast — never swallowed with `let _ = ...` or panicked with `.unwrap()`.
4. **Keep Code Small and Cohesive:** Prefer focused modules (under ~400 lines) with clear ownership and unit tests for pure logic (`crates/core`, coordinate conversions, LSP framing, config parsing) decoupled from UI rendering.
5. **Respect Real Typst Workflows:** Test continuously against a realistic multi-file Typst project (root `main.typ`, imported chapter files, custom template, `.bib` bibliography, figures, math blocks, and Typst Universe packages), not just a 5-line `"Hello world"` file.

---

For a practical look at how Rune's headless PNG and element-tree loop works with coding agents, see [Inspecting Rune UI headlessly with AI agents](https://www.youtube.com/watch?v=IhQmgQWBFfY), which demonstrates the exact self-verification workflow baked into Phase 0 and Section 7 above.
