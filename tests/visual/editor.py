"""Inspect the virtual Typst editor and replay a 20,000-line editing session."""

import argparse
import json
import math
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "visual" / "editor"
NEEDLE = "PhaseOneNeedle"
REPLACEMENT = "PhaseOneChanged"


def key(t, name, **modifiers):
    return dict(t=t, type="key", name=name, **modifiers)


def typed(t, text):
    return dict(t=t, type="type", text=text, every=0.01)


def tap(t, target):
    return dict(t=t, type="tap", key=target)


def project(directory):
    (directory / "template.typ").write_text('#let document(body) = { set page(paper: "a4"); body }\n')
    (directory / "chapter.typ").write_text("= Background\nThe result follows from $a^2 + b^2 = c^2$.\n")
    (directory / "references.bib").write_text("@book{key,title={Notes},author={Doe, J.},year={2025}}\n")
    lines = [
        '#import "template.typ": document',
        '#show: document',
        '#include "chapter.typ"',
        "",
        "= A study of document layout",
        '// The Typst editor colours comments, strings and code.',
        '#let count = 42',
        'A cross-reference to @results and a citation @key.',
        '$ sum_(k=1)^n k = (n(n+1))/2 $',
        '#figure(rect(width: 24pt, height: 12pt), caption: [Sample])',
        "",
    ]
    for i in range(len(lines), 19_997):
        lines.append(f'Section {i:05}: #emph[Measured value] $x_i$' if i != 10_000 else f'= {NEEDLE} <results>')
    lines.extend(["", '#bibliography("references.bib")', "End of the document."])
    assert len(lines) == 20_000
    main = directory / "main.typ"
    main.write_text("\n".join(lines))
    return main


def run(binary, file, name, events=(), at=0.0, scale=2, flags=(), log=False, open_initial=True):
    script = OUT / f"{name}.script.json"
    script.write_text(json.dumps({"events": events}))
    tree = OUT / f"{name}.json"
    environment = {**os.environ, "FAST_CONFIG": str(file.parent / "config.toml")}
    if log:
        environment["RUNE_FRAME_LOG"] = str(OUT / f"{name}.frames.jsonl")
    arguments = [str(binary), *([str(file)] if open_initial else [])]
    subprocess.run(arguments + [
        "--script", str(script), "--at", str(at),
        "--scale", str(scale), "--snap", str(OUT / f"{name}.png"), "--tree", str(tree), *flags,
    ], cwd=ROOT, check=True, env=environment)
    output = json.loads(tree.read_text())
    assert not output["warnings"], output["warnings"]
    lines = [element for element in output["elements"] if element.get("key") == "source"]
    assert 0 < len(lines) < 120, f"{name}: {len(lines)} mounted source lines"
    print(f"Verified {name} ({len(lines)} mounted lines)", flush=True)
    return output


def contains(tree, text):
    return any(text in (element.get("text") or "") for element in tree["elements"])


def word_selection(binary, directory):
    file = directory / "words.typ"
    source = "alpha bravo charlie"
    file.write_text(source)
    tree = run(binary, file, "word-selection-initial", at=0.10)
    x, y, width, height = next(e["rect"] for e in tree["elements"] if e.get("text") == source)
    advance = width / len(source)

    def pointer(t, kind, column):
        return dict(t=t, type=kind, x=x + column * advance, y=y + height / 2)

    # Keep the second press held, including a small move inside the original word.
    press = [pointer(0.15, "tap", 8.25), pointer(0.31, "down", 8.25),
             pointer(0.36, "move", 8.50)]
    for name, column, expected in [
        ("word-selection", 8.5, "alpha X charlie"),
        ("word-drag-forward", 14.5, "alpha X"),
        ("word-drag-backward", 1.5, "X charlie"),
    ]:
        events = press + [pointer(0.42, "move", column), pointer(0.48, "up", column),
                          typed(0.55, "X")]
        result = run(binary, file, name, events, at=0.70)
        assert any(e.get("text") == expected for e in result["elements"]), name


def auto_indent(binary, directory):
    file = directory / "indent.typ"
    file.write_bytes(b"heading\r\n \t  payload")
    events = [key(0.15, "End", cmd=True), key(0.25, "Enter"), key(0.35, "Enter"),
              typed(0.45, "next"), key(0.65, "s", cmd=True)]
    run(binary, file, "auto-indent", events, at=1.15, flags=("--realtime",))
    assert file.read_bytes() == b"heading\r\n \t  payload\r\n \t  \r\n \t  next"


def workflow(binary, file):
    (file.parent / "config.toml").write_text("[lsp]\nenabled = false\n")
    initial = run(binary, file, "darcula-editor", at=0.10, scale=1)
    assert contains(initial, "A study of document layout")
    (file.parent / "config.toml").write_text('[ui]\ntheme = "intellij-light-classic"\n[lsp]\nenabled = false\n')
    initial = run(binary, file, "light-editor", at=0.10)
    assert contains(initial, "A study of document layout")
    assert any(e.get("key") == "caret-0" and e["opacity"] == 1.0 for e in initial["elements"])
    blink = run(binary, file, "caret-blink", at=0.80)
    assert any(e.get("key") == "caret-0" and e["opacity"] == 0.0 for e in blink["elements"])

    dialog = [tap(0.10, "open-file"), dict(t=0.30, type="file-dialog", path=str(file))]
    opened = run(binary, file, "opened", dialog, at=1.0, flags=("--realtime",), open_initial=False)
    assert contains(opened, "main.typ") and contains(opened, "A study of document layout")

    x, y, _, h = next(e["rect"] for e in initial["elements"] if e.get("key") == "source")
    drag = [dict(t=0.15, type="drag", x=x + 17, y=y + h / 2, dx=57, dy=0, dur=0.25)]
    selection = run(binary, file, "pointer-selection", drag, at=0.48)
    assert any(e.get("key", "").startswith("selection-") for e in selection["elements"])

    events = [
        tap(0.10, "replace-file"),
        typed(0.25, NEEDLE),
        tap(0.58, "find-replacement"),
        typed(0.70, REPLACEMENT),
        tap(0.90, "replace-current"),
        tap(1.02, "find-close"),
        key(1.15, "z", cmd=True),
        key(1.30, "z", cmd=True, shift=True),
        key(1.50, "s", cmd=True),
    ]
    find = run(binary, file, "find-replace", events, at=0.86)
    assert contains(find, "1/1") and contains(find, NEEDLE) and contains(find, REPLACEMENT)
    assert any(element.get("key", "").startswith("selection-") for element in find["elements"])
    undo = run(binary, file, "undo", events, at=1.25)
    assert contains(undo, NEEDLE) and not contains(undo, REPLACEMENT)
    redo = run(binary, file, "redo", events, at=1.42)
    assert contains(redo, REPLACEMENT) and not contains(redo, NEEDLE)

    original = file.read_text()
    saved = run(binary, file, "saved", events, at=2.0, flags=("--realtime",))
    assert contains(saved, "Saved")
    assert file.read_text() == original.replace(NEEDLE, REPLACEMENT)

    bracket = [key(0.12, "End", cmd=True), typed(0.25, "("), typed(0.4, "x"), typed(0.55, ")")]
    edited = run(binary, file, "smart-brackets", bracket, at=0.75)
    assert contains(edited, "End of the document.(x)")


def latency(binary, file):
    # Alternate insert/delete so every sampled frame changes text without turning the
    # benchmark into a progressively longer single line.
    events = [key(0.1, "End", cmd=True)]
    for i in range(200):
        t = round(0.5 + i / 30, 6)
        events.append(typed(t, "x") if i % 2 == 0 else key(t, "Backspace"))
    out = run(binary, file, "keystroke-latency", events, at=7.25, log=True)
    assert contains(out, "End of the document.")
    frames = [json.loads(line) for line in (OUT / "keystroke-latency.frames.jsonl").read_text().splitlines()]
    samples = sorted(row["ms"] for row in frames if row.get("events") and row["k"] >= 29)
    assert len(samples) == 200, f"expected 200 keystroke frames, found {len(samples)}"
    p99 = samples[math.ceil(len(samples) * 0.99) - 1]
    print(f"20,000 lines, {len(samples)} keystrokes, p99 {p99:.3f} ms, max {samples[-1]:.3f} ms")
    assert p99 < 8.0, f"keystroke p99 exceeded 8 ms ({p99:.3f} ms)"

    scroll_events = [
        dict(t=round(0.50 + i / 30, 6), type="wheel", x=300, y=200, dx=0, dy=-44, precise=True)
        for i in range(100)
    ]
    scrolled = run(binary, file, "scroll-latency", scroll_events, at=4.0, log=True)
    assert not contains(scrolled, "A study of document layout")
    frames = [json.loads(line) for line in (OUT / "scroll-latency.frames.jsonl").read_text().splitlines()]
    commits = [row["commit_cpu"] for row in frames if row.get("k", 0) >= 29 and row.get("commits")]
    assert commits, "the scroll script produced no virtual-row commits"
    maximum = max(commits)
    print(f"{len(commits)} scroll commits, max {maximum:.3f} ms CPU")
    assert maximum < 4.0, f"scroll commit exceeded 4 ms ({maximum:.3f} ms)"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--skip-latency", action="store_true")
    args = parser.parse_args()
    binary = args.binary
    if binary is None:
        subprocess.run(["cargo", "build", "--locked", "--release", "--features", "frame-profile", "-p", "fast"],
                       cwd=ROOT, check=True)
        binary = ROOT / "target" / "release" / ("fast.exe" if os.name == "nt" else "fast")
    binary = binary.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix="fast-editor-") as temporary:
        file = project(Path(temporary))
        workflow(binary, file)
        word_selection(binary, file.parent)
        auto_indent(binary, file.parent)
        if not args.skip_latency:
            latency(binary, file)


if __name__ == "__main__":
    main()
