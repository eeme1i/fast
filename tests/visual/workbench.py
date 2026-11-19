"""Replay the dock layout, project navigation, tabs and live settings with Rune."""

import argparse
from pathlib import Path
from tempfile import TemporaryDirectory

import preview
from check import luminance
from editor import key, tap, typed

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "visual" / "workbench"


def node(tree, name):
    return next(element for element in tree if element.get("key") == name)


def has(tree, value):
    return any(value in (element.get("text") or "") for element in tree)


def capture(binary, file, name, events, at, scale=2):
    tree = preview.capture(binary, file, name, events, at, scale)
    print(f"Verified {name}", flush=True)
    return tree


def new_file(binary, main):
    original = main.read_text()
    target = main.with_name("draft.typ")
    edit = [key(.30, "End", cmd=True), key(.38, "Enter"), typed(.42, "Unsaved original.")]
    create = [tap(.65, "new-file"), typed(.85, "= Draft"), key(1.12, "s", cmd=True),
              dict(t=1.38, type="file-dialog", path=str(target))]
    saved = capture(binary, main, "new-file-saved", edit + create, 1.90)
    assert node(saved, "tab-0")["label"] == "main.typ •"
    assert node(saved, "tab-1")["label"] == target.name
    assert node(saved, "editor")["focused"] and has(saved, "= Draft")
    assert target.read_text() == "= Draft"
    assert main.read_text() == original

    commands = [key(.36, "n", cmd=True), typed(.52, "First draft"),
                key(.78, "p", ctrl=True, shift=True), typed(.95, "new file"),
                key(1.20, "Enter"), typed(1.36, "Second draft")]
    drafts = capture(binary, main, "new-file-shortcuts", commands, 1.65)
    assert node(drafts, "tab-1")["label"] == "Untitled •"
    assert node(drafts, "tab-2")["label"] == "Untitled •"
    assert has(drafts, "Second draft")


def chrome(tree, scale, dark):
    window = node(tree, "window")
    assert (luminance(window["fill"]) < 0.5) == dark
    elements = {element["path"]: element for element in tree}
    for key in ("document-bar", "tab-bar", "left-strip", "right-strip", "bottom-strip", "status-bar"):
        x, y, w, h = node(tree, key)["rect"]
        assert w > 0 and h > 0 and x >= 0 and y >= 0, key
        assert x + w <= window["rect"][2] + 0.02 and y + h <= window["rect"][3] + 0.02, key
    for element in tree:
        if element.get("key", "").startswith("resize-"):
            line = next(e for e in tree if e["path"].rpartition("/")[0] == element["path"])
            horizontal = element["key"] == "resize-bottom"
            axis = 3 if horizontal else 2
            assert abs(element["rect"][axis] - 4.0) < 0.02
            assert abs(line["rect"][axis] - 1 / scale) < 0.02
        if "text" in element and element.get("key") != "source":
            parent = elements[element["path"].rpartition("/")[0]]
            while parent["fill"][-2:] == "00":
                parent = elements[parent["path"].rpartition("/")[0]]
            low, high = sorted((luminance(element["color"]), luminance(parent["fill"])))
            assert (high + 0.05) / (low + 0.05) >= 4.5, element["path"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target" / "debug" / "fast")
    args = parser.parse_args()
    binary = args.binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    preview.OUT = OUT
    with TemporaryDirectory(prefix="fast-workbench-") as folder:
        root = Path(folder)
        main_file = preview.fixture(root)
        main_file.write_text(main_file.read_text() + "= Preview workbench\n")
        sub = root / "sections"
        sub.mkdir()
        chapter = sub / "methods.typ"
        chapter.write_text("= Methods\nA distinctive needle in the workspace.\n")
        (root / "ignored.typ").write_text("A distinctive needle in ignored content.\n")
        (root / ".gitignore").write_text("ignored.typ\n")
        config = root / "config.toml"
        config.write_text("[compiler]\nsystem_fonts = false\n")
        project = [tap(.35, "dock-project")]
        outline = [key(.58, "2", alt=True)]
        initial = capture(binary, main_file, "dark-project", project + outline, 1.45, 1)
        chrome(initial, 1, dark=True)
        assert node(initial, "tool-project")["rect"][0] < node(initial, "editor-area")["rect"][0]
        assert node(initial, "tool-project")["rect"][1] < node(initial, "tool-outline")["rect"][1]
        assert has(initial, "Preview workbench")

        divider = node(initial, "resize-left")["rect"]
        drag = dict(t=1.50, type="drag", x=divider[0] + 2, y=divider[1] + 70, dx=36, dy=0, dur=.18)
        resized = capture(binary, main_file, "dock-resized", project + outline + [drag], 1.92, 1)
        assert node(resized, "dock-area-left")["rect"][2] > node(initial, "dock-area-left")["rect"][2] + 20
        restored = capture(binary, main_file, "dock-size-restored", project, 1.12, 1)
        assert abs(node(restored, "dock-area-left")["rect"][2] - node(resized, "dock-area-left")["rect"][2]) < 1

        boundary = node(initial, "split-project-outline")["rect"]
        split = dict(t=1.50, type="drag", x=boundary[0] + 60, y=boundary[1] + 2, dx=0, dy=32, dur=.18)
        stacked = capture(binary, main_file, "stack-resized", project + outline + [split], 1.92, 1)
        assert node(stacked, "tool-project")["rect"][3] > node(initial, "tool-project")["rect"][3] + 20

        directory_key = next(e["key"] for e in initial if e.get("label") == "sections")
        expanded = project + [tap(.80, directory_key)]
        tree = capture(binary, main_file, "tree-expanded", expanded, 1.18)
        child = next(e["key"] for e in tree if e.get("label") == "methods.typ")
        opened = capture(binary, main_file, "tree-opened", expanded + [tap(1.20, child)], 1.95)
        assert node(opened, "tab-1")["label"] == "methods.typ"
        assert has(opened, "A distinctive needle")
        assert not any(e.get("label") == "ignored.typ" for e in opened)

        palette = [key(.42, "p", ctrl=True, shift=True), typed(.60, "toggle compiler"), key(.95, "Enter")]
        command = capture(binary, main_file, "command-palette-run", palette, 1.45)
        assert node(command, "tool-compiler") and node(command, "compiler-output")

        quick = [key(.72, "p", ctrl=True), typed(.85, "mthds"), key(1.16, "Enter")]
        edits = [key(.34, "End", cmd=True), key(.42, "Enter"), typed(.48, "Unsaved tab text.")]
        switched = capture(binary, main_file, "fuzzy-open-dirty-tab", edits + quick, 1.85)
        assert "•" in node(switched, "tab-0")["label"]
        assert node(switched, "tab-1")["label"] == "methods.typ"
        assert has(switched, "A distinctive needle")
        retained = capture(binary, main_file, "retained-dirty-buffer", edits + quick + [tap(1.90, "tab-0")], 2.30)
        assert has(retained, "Unsaved tab text.")
        close = [tap(2.35, "close-tab-0")]
        prompt = capture(binary, main_file, "confirm-dirty-tab", edits + quick + [tap(1.90, "tab-0")] + close, 2.56)
        assert node(prompt, "discard-confirmation")

        pin = [dict(t=.85, type="secondary-click", key="tab-0"), tap(1.02, "pin-tab-action")]
        later_quick = [dict(event, t=event["t"] + .70) for event in quick]
        export = root / "unsaved-export.html"
        export_html = [tap(2.18, "export-document"), tap(2.40, "export-html"),
                       dict(t=2.62, type="file-dialog", path=str(export))]
        pinned = capture(binary, main_file, "pinned-unsaved-main", edits + pin + later_quick + export_html, 3.75)
        assert "[Main]" in node(pinned, "tab-0")["label"]
        assert "Unsaved tab text." in export.read_text()
        assert "Unsaved tab text." not in main_file.read_text()

        search = [key(.60, "f", ctrl=True, shift=True), typed(.75, "distinctive needle")]
        with (root / "large-download.txt").open("wb") as large:
            large.write(b"distinctive needle\n")
            large.truncate(4 * 1024**3)
        (root / "clip.MP4").write_bytes(b"distinctive needle")
        (root / "binary-without-extension").write_bytes(b"distinctive needle\x00")
        results = capture(binary, main_file, "workspace-search", search, 1.35)
        assert node(results, "workspace-match-preview")
        skipped = node(results, "workspace-search-skipped")
        assert skipped["text"] == "3 skipped"
        assert skipped["help"] == "Files skipped · 2 binary or media, 1 over 8 MiB"
        assert has(results, "sections/methods.typ")
        assert not any(e.get("key") == "workspace-match-1" for e in results)
        jumped = capture(binary, main_file, "workspace-search-jump", search + [key(1.41, "Enter")], 2.1)
        assert node(jumped, "tab-1")["label"] == "methods.typ"
        assert any(e.get("key", "").startswith("selection-") for e in jumped)

        config.write_text('[ui]\ntheme = "intellij-light-classic"\n[compiler]\nsystem_fonts = false\n')
        right = project + outline + [tap(.95, "settings-menu"), tap(1.15, "sidebar-side"),
                                     tap(1.35, "ui-density"), tap(1.55, "settings-done")]
        light = capture(binary, main_file, "light-project-right", right, 2.10)
        chrome(light, 2, dark=False)
        assert node(light, "tool-project")["rect"][0] > node(light, "editor-area")["rect"][0]
        assert node(light, "tab-bar")["rect"][3] == 22.0

        enlarged = right[:3] + [tap(1.15, "ui-size-increase"), tap(1.35, "editor-size-increase"),
                                tap(1.55, "tabs-placement"), tap(1.75, "settings-done")]
        settings = capture(binary, main_file, "live-typography", enlarged, 2.20)
        assert node(settings, "tab-bar")["rect"][1] > node(settings, "source")["rect"][1]
        assert node(settings, "source")["rect"][2] > node(light, "source")["rect"][2]

        relocate = right + [tap(2.20, "move-outline"), tap(2.42, "move-to-bottom")]
        moved = capture(binary, main_file, "outline-docked-bottom", relocate, 2.95)
        assert "/dock-area-bottom/" in node(moved, "tool-outline")["path"]

        config.write_text('[ui]\npreview_dock = "tab"\n[compiler]\nsystem_fonts = false\n')
        show = [tap(.30, "toggle-preview")]
        preview_tab = capture(binary, main_file, "preview-in-tab", show, 1.85)
        assert node(preview_tab, "preview-tab") and node(preview_tab, "preview-page-1")
        source_tab = capture(binary, main_file, "source-from-preview-tab", show + [tap(1.85, "tab-0")], 2.10)
        assert node(source_tab, "preview-tab") and node(source_tab, "source")
        new_file(binary, main_file)


if __name__ == "__main__":
    main()
