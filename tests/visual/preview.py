"""Replay the live Typst preview and exports in the Rune editor."""

import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory
import xml.etree.ElementTree as ET

from editor import key, tap, typed


ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "visual" / "preview"


def capture(binary, file, name, events, at, scale=2):
    script = OUT / f"{name}.script.json"
    tree_path = OUT / f"{name}.json"
    script.write_text(json.dumps({"events": events}))
    subprocess.run(
        [str(binary), str(file), "--realtime", "--script", str(script), "--at", str(at),
         "--scale", str(scale), "--snap", str(OUT / f"{name}.png"), "--tree", str(tree_path)],
        cwd=ROOT, check=True, env={**os.environ, "FAST_CONFIG": str(file.parent / "config.toml")},
    )
    tree = json.loads(tree_path.read_text())
    assert not tree["warnings"], tree["warnings"]
    assert tree["scale"] == scale
    return tree["elements"]


def element(tree, name):
    return next(node for node in tree if node.get("key") == name)


def fixture(directory):
    main = directory / "main.typ"
    (directory / "template.typ").write_text('#let document(body) = { set page(width: 320pt, height: 400pt, margin: 28pt); body }\n')
    (directory / "chapter.typ").write_text('= Results\nA separate chapter compares $x^2 + y^2$ against @notes.\n')
    (directory / "refs.bib").write_text('@book{notes,title={Typesetting Notes},author={Doe, Jane},year={2025}}\n')
    main.write_text(
        '#import "template.typ": document\n'
        '#show: document\n\n'
        'A native preview updates while you type.\n\n'
        '#rect(width: 48pt, height: 14pt, fill: rgb("#527ca5"))\n'
        '#pagebreak(weak: true)\n'
        '#include "chapter.typ"\n\n'
        '#bibliography("refs.bib")\n'
    )
    return main


def verify_export(file, extension, png_size=(640, 800)):
    data = file.read_bytes()
    assert len(data) > 100, (file, len(data))
    if extension == "pdf":
        assert data.startswith(b"%PDF-") and data.rstrip().endswith(b"%%EOF")
    elif extension == "png":
        assert data.startswith(b"\x89PNG\r\n\x1a\n") and b"IEND" in data
        assert struct.unpack(">II", data[16:24]) == png_size
    elif extension == "svg":
        assert ET.fromstring(data).tag == "{http://www.w3.org/2000/svg}svg"
    elif extension == "html":
        assert b"<html" in data and b"A native preview updates" in data
    else:
        assert b"A native preview updates" in data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target" / "debug" / "fast")
    parser.add_argument("--native-only", action="store_true")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix="fast-preview-") as folder:
        directory = Path(folder)
        binary = args.binary.resolve()
        main = fixture(directory)
        config = directory / "config.toml"
        config.write_text('[compiler]\nsystem_fonts = false\n[lsp]\nenabled = false\n')
        show = [tap(0.15, "toggle-preview")]
        dark = capture(binary, main, "preview-dark", show, 2.00, 1)
        canvas = element(dark, "preview-canvas")["rect"]
        assert element(dark, "preview-page-1")["rect"]
        assert element(dark, "preview-status")["text"].startswith("Preview ")

        # Fit-width scales the first page to the clipped viewport minus its margins.
        zoom = (canvas[2] - 24.0) / 320.0
        x, y = canvas[0] + 12 + 52 * zoom, canvas[1] + 12 + 35 * zoom
        jump = show + [dict(t=1.80, type="double-click", x=x, y=y)]
        jumped = capture(binary, main, "preview-jump", jump, 2.25, 1)
        assert "Ln 4," in element(jumped, "position")["text"], element(jumped, "position")

        original = main.read_bytes()
        edited = jump + [key(2.15, "Home"), typed(2.25, "New text. ")]
        changed = capture(binary, main, "preview-edited", edited, 2.90, 1)
        assert any((node.get("text") or "").startswith("New text. A native") for node in changed)
        assert main.read_bytes() == original
        assert (OUT / "preview-edited.png").read_bytes() != (OUT / "preview-jump.png").read_bytes()

        config.write_text('[ui]\ntheme = "intellij-light-classic"\n[compiler]\nsystem_fonts = false\n[lsp]\nenabled = false\n')
        spread = show + [tap(1.60, "preview-view-options"), tap(1.80, "preview-spread")]
        light = capture(binary, main, "preview-spread-light", spread, 2.20)
        first, second = (element(light, f"preview-page-{page}")["rect"] for page in (1, 2))
        assert first[1] == second[1] and first[0] < second[0]

        dark_paper = show + [tap(1.45, "preview-view-options"), tap(1.60, "preview-invert"),
                             dict(t=1.85, type="wheel", x=x, y=y, dx=0, dy=-400, precise=True)]
        scrolled = capture(binary, main, "preview-dark-paper", dark_paper, 2.40)
        assert element(scrolled, "preview-page-number")["text"] == "2 / 2"
        assert element(scrolled, "preview-page-2")["rect"][1] < canvas[1] + canvas[3]

        if not args.native_only:
            config.write_text('[compiler]\nsystem_fonts = false\n')
            chapter = directory / "chapter.typ"
            pin = show + [tap(0.95, "language-menu"), tap(1.10, "language-pin"),
                          dict(t=1.30, type="file-dialog", path=str(main)),
                          tap(1.80, "preview-view-options"), tap(1.95, "preview-follow"),
                          dict(t=2.20, type="tap", x=126, y=85)]
            pinned = capture(binary, chapter, "preview-pinned-chapter", pin, 2.90, 1)
            assert "main.typ" in element(pinned, "pinned-main")["label"]
            assert element(pinned, "preview-page-number")["text"] == "2 / 2"
        text_document = directory / "text-export.typ"
        text_document.write_text('= Preview and export\nA native preview updates while you type.\n\n'
                                 'This document has a heading and enough ordinary text to verify the exported content.\n')
        formats = ["pdf", "png", "svg", "html"]
        if not args.native_only:
            formats += ["markdown", "text"]
        for format in formats:
            extension = {"markdown": "md", "text": "txt"}.get(format, format)
            path = directory / f"exported.{extension}"
            events = [tap(1.20, "export-document"), tap(1.40, f"export-{format}"),
                      dict(t=1.65, type="file-dialog", path=str(path))]
            source = text_document if format in {"markdown", "text"} else main
            exported = capture(binary, source, f"export-{format}", events, 2.90, 1)
            status = element(exported, "export-status")["text"]
            assert status == f"Exported {path.name}", status
            verify_export(path, extension)
        quick = capture(binary, main, "quick-export", [key(0.60, "e", cmd=True, shift=True)], 2.20, 1)
        assert element(quick, "export-status")["text"] == "Exported main.pdf"
        verify_export(directory / "build" / "main.pdf", "pdf")

        config.write_text('[compiler]\nsystem_fonts = false\n[lsp]\nenabled = false\n'
                          '[export]\npng_ppi = 72\npng_pages = "1-2"\non_save = true\n'
                          'quick_path = "$root/published/$name.pdf"\npdf_standard = "a-2b"\n')
        pages = [tap(0.35, "export-document"), tap(0.52, "export-png"),
                 dict(t=0.78, type="file-dialog", path=str(directory / "pages.png"))]
        exported = capture(binary, main, "export-page-range", pages, 2.20, 1)
        assert element(exported, "export-status")["text"] == "Exported 2 pages"
        for page in (1, 2):
            verify_export(directory / f"pages-{page:02}.png", "png", (320, 400))
        saved = capture(binary, main, "export-on-save", [key(0.60, "s", cmd=True)], 2.20, 1)
        assert element(saved, "export-status")["text"] == "Exported main.pdf"
        archive = directory / "published" / "main.pdf"
        verify_export(archive, "pdf")
        assert b"pdfaid:part" in archive.read_bytes()
    print("Verified preview, click-to-source, unsaved edits, spreads and exports")


if __name__ == "__main__":
    main()
