"""Exercise editor gestures, image paste, disk conflicts and session recovery with Rune."""

import argparse
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory
import time
import tomllib

from editor import ROOT, key, tap, typed
from preview import fixture
from workbench import has, node

OUT = ROOT / "target" / "visual" / "phase5"
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGNwqJ/5H4QZYAwAT6AJXV6dlPsAAAAASUVORK5CYII=")


def capture(binary, root, name, events=(), at=1.0, source=None, change=None):
    tape = OUT / f"{name}.script.json"
    tree = OUT / f"{name}.json"
    tape.write_text(json.dumps({"events": events}))
    arguments = [str(binary), *([str(source)] if source else []),
                 "--realtime", "--script", str(tape), "--at", str(at), "--scale", "2",
                 "--snap", str(OUT / f"{name}.png"), "--tree", str(tree)]
    environment = {**os.environ, "FAST_CONFIG": str(root / "config.toml")}
    with subprocess.Popen(arguments, env=environment, cwd=ROOT) as process:
        if change:
            deadline = time.monotonic() + 12.0
            started = root / ".config.toml.session.toml"
            while not started.exists() and time.monotonic() < deadline and process.poll() is None:
                time.sleep(.02)
            assert started.exists(), "editor did not finish loading the original file"
            change()
        assert process.wait(timeout=30) == 0, name
    capture = json.loads(tree.read_text())
    assert not capture["warnings"], capture["warnings"]
    print(f"Verified {name}", flush=True)
    return capture["elements"]


def gestures(binary, root):
    config = root / "config.toml"
    config.write_text('[lsp]\nenabled = false\n[keymap]\nselect-next-occurrence = "ctrl+k"\n')
    file = root / "gestures.typ"
    file.write_text("echo echo echo\nalpha one\nalpha two\nalpha three\n")
    select = [key(.15, "d", ctrl=True), key(.29, "k", ctrl=True), key(.42, "k", ctrl=True)]
    multi = capture(binary, root, "multi-cursor", select, .65, file)
    assert sum(e.get("key", "").startswith("selection-") for e in multi) == 2
    edited = capture(binary, root, "multi-edit", select + [typed(.58, "X")], .85, file)
    assert has(edited, "X X echo")
    word_end = [key(.16 + .06*i, "Right") for i in range(4)]
    word_end += [key(.45, "k", ctrl=True), typed(.60, "Y")]
    assert has(capture(binary, root, "select-at-word-end", word_end, .85, file), "Y echo echo")
    config.write_text('assets_dir = "media/figures"\n[ui]\ntheme = "intellij-light-classic"\n[lsp]\nenabled = false\n')
    initial = capture(binary, root, "gesture-geometry", source=file)
    source = {e.get("text"): e["rect"] for e in initial if e.get("key") == "source"}
    x, y, width, h = source["alpha one"]
    advance = width / len("alpha one")
    center = y + h / 2
    row_height = source["alpha two"][1] - y
    block = [dict(t=.20, hold="alt"),
             dict(t=.29, type="drag", x=x + 6*advance, y=center, dx=3*advance,
                  dy=2*row_height, dur=.32),
             dict(t=.67, hold="")]
    selected = capture(binary, root, "column-selection", block, .80, file)
    assert sum(e.get("key", "").startswith("selection-") for e in selected) == 3
    assert has(capture(binary, root, "column-edit", block + [typed(.79, "!")], 1.0, file), "alpha !")
    alt_click = [dict(t=.20, hold="alt"),
                 dict(t=.29, type="tap", x=x + advance / 5, y=center),
                 dict(t=.43, hold=""), typed(.53, "X")]
    added = capture(binary, root, "alt-click", alt_click, .78, file)
    assert has(added, "Xecho") and has(added, "Xalpha one")
    config.write_text('[lsp]\nenabled = false\n[keymap]\nselect-next-occurrence = "mod+s"\n')
    bad = capture(binary, root, "keymap-conflict", source=file)
    assert "select-next-occurrence" in node(bad, "config-status")["help"]


def image_paste(binary, root):
    (root / "config.toml").write_text('assets_dir = "media/figures"\n[lsp]\nenabled = false\n')
    media = root / "media" / "figures"
    media.mkdir(parents=True)
    (media / "image.png").write_bytes(b"Existing asset")
    source = root / "sections" / "chapter.typ"
    source.parent.mkdir()
    source.write_text("")
    image = root / "clipboard.png"
    image.write_bytes(PNG)
    config = root / "config.toml"
    config.write_text(config.read_text() + '[compiler]\nroot = "."\nsystem_fonts = false\n')
    tree = capture(binary, root, "paste-image", [
        dict(t=.25, type="paste", png=str(image)), key(.85, "s", cmd=True),
        key(1.10, "p", alt=True),
    ], 3.0, source)
    assert has(tree, '#image("../media/figures/image-2.png")')
    assert node(tree, "preview-page-1")
    assert source.read_text() == '#image("../media/figures/image-2.png")'
    assert (media / "image-2.png").read_bytes() == PNG
    assert (media / "image.png").read_bytes() == b"Existing asset"


def external_change(binary, root, light):
    (root / "config.toml").write_text(
        ('[ui]\ntheme = "intellij-light-classic"\n' if light else "") + "[lsp]\nenabled = false\n")
    source = root / "external.typ"
    source.write_text("= Original on disk\n")

    def change():
        replacement = root / "external.swap"
        replacement.write_text("= New disk version\n")
        replacement.replace(source)

    edits = [key(.25, "End"), typed(.33, " with local edits"), key(.65, "s", cmd=True)]
    comparison = edits + [tap(1.55, "compare-external")]
    if light:
        tree = capture(binary, root, "external-light", comparison, 2.05, source, change)
        assert node(tree, "external-comparison")
        assert has(tree, "with local edits") and has(tree, "New disk version")
        assert source.read_text() == "= New disk version\n"
    else:
        keep = comparison + [tap(1.90, "keep-compared"), key(2.12, "s", cmd=True)]
        tree = capture(binary, root, "external-keep", keep, 2.65, source, change)
        assert has(tree, "Saved") and source.read_text() == "= Original on disk with local edits\n"


def session(binary, root):
    source = fixture(root)
    config = root / "config.toml"
    tinymist = shutil.which("tinymist")
    config.write_text(f'[compiler]\nsystem_fonts = false\n[lsp]\nenabled = {str(bool(tinymist)).lower()}\n')
    chapter = root / "chapter.typ"
    last = "End of the recovered chapter"
    chapter.write_text("= Results\n" + ("A separate chapter.\n" * 95) + last + "\n")
    if tinymist:
        pin = [dict(t=.70, type="secondary-click", key="tab-0"), tap(.90, "pin-tab-action")]
    else:
        pin = []
    dock = [tap(.28, "dock-project"), key(.43, "2", alt=True)]
    before = capture(binary, root, "session-layout", dock, 1.25, source)
    width = node(before, "dock-area-left")["rect"][2]
    x, y, _, _ = node(before, "resize-left")["rect"]
    resize = dict(t=1.04, type="drag", x=x+2, y=y+70, dx=33, dy=0, dur=.16)
    tabs = [tap(1.43, "open-file"), dict(t=1.66, type="file-dialog", path=str(chapter)),
            key(2.02, "End", cmd=True), typed(2.18, "Unsaved chapter"),
            key(2.48, "n", cmd=True), typed(2.63, "Unsaved untitled"), tap(2.95, "tab-1")]
    edited = capture(binary, root, "session-original", dock + pin + [resize] + tabs, 3.60, source)
    assert has(edited, "Unsaved chapter")
    path = root / ".config.toml.session.toml"
    saved = tomllib.loads(path.read_text())
    assert len(saved["tabs"]) == 3 and saved["active"] == 1
    assert saved["tabs"][1]["scroll"][1] > 100
    if tinymist:
        assert Path(saved["pinned"]).name == "main.typ"
    after = capture(binary, root, "session-restored", at=1.70)
    assert has(after, "Unsaved chapter") and node(after, "tab-2")["label"] == "Untitled •"
    assert node(after, "dock-area-left")["rect"][2] > width + 20
    assert node(after, "tool-project") and node(after, "tool-outline")
    if tinymist:
        assert "[Main]" in node(after, "tab-0")["label"]
    assert "Unsaved chapter" not in chapter.read_text()
    chapter.write_text("Updated while Fast was closed.\n")
    conflict = capture(binary, root, "session-disk-conflict", at=1.45)
    assert node(conflict, "external-change") and has(conflict, "Unsaved chapter")
    reload_and_undo = [tap(1.50, "reload-external"), key(1.95, "z", cmd=True)]
    undo = capture(binary, root, "external-reload-undo", reload_and_undo, 2.35)
    assert has(undo, "Unsaved chapter") and chapter.read_text() == "Updated while Fast was closed.\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target" / "release" / "fast")
    parser.add_argument("--only", choices=["gestures", "image-paste", "external-dark", "external-light", "session"])
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    cases = {
        "image-paste": image_paste,
        "external-dark": lambda binary, root: external_change(binary, root, False),
        "external-light": lambda binary, root: external_change(binary, root, True),
        "session": session,
        "gestures": gestures,
    }
    for name, case in cases.items():
        if args.only is None or name == args.only:
            with TemporaryDirectory(prefix=f"fast-phase5-{name}-") as directory:
                case(args.binary.resolve(), Path(directory))


if __name__ == "__main__":
    main()
