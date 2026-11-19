"""Replay Tinymist language tools in the Rune editor with a real server."""

import argparse
import json
import os
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory

from editor import key, tap, typed


ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "visual" / "language"


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
    return tree


def by_key(tree, key):
    return [node for node in tree["elements"] if node.get("key") == key]


def has_text(tree, text):
    return any(text in (node.get("text") or "") for node in tree["elements"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target" / "debug" / "fast")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    with TemporaryDirectory(prefix="fast-language-") as folder:
        directory = Path(folder)
        main = directory / "main.typ"
        chapter = directory / "chapter.typ"
        completion = directory / "completion.typ"
        config = directory / "config.toml"
        main.write_text('= Introduction\n#include "chapter.typ"\n\n== Details\nThe result follows.\n')
        chapter.write_text("= Chapter\nHere is an error.\n#missing_diagnostic_name\n")
        completion.write_text("#let item = 42\n" + "// filler\n" * 33 + "#ite\n" + "// filler\n" * 55)

        suggest = [dict(t=0.40, type="wheel", x=300, y=200, dx=0, dy=-500, precise=True),
                   tap(0.80, "line-35"), key(0.85, "End"), key(0.95, " ", cmd=True)]
        tree = capture(binary, completion, "completion-dark", suggest, 1.25, 1)
        assert by_key(tree, "completion-popup") and has_text(tree, "item")
        assert by_key(tree, "completion-label")[0]["text"] == "item"
        y = by_key(tree, "line-35")[0]["rect"][1]
        for name, accept in [
            ("accepted-completion", key(1.30, "Enter")),
            ("tab-completion", key(1.30, "Tab")),
            ("clicked-completion", tap(1.30, "completion-0")),
        ]:
            tree = capture(binary, completion, name, suggest + [accept], 1.60, 1)
            assert any(node.get("text") == "#item" for node in by_key(tree, "source"))
            assert abs(by_key(tree, "line-35")[0]["rect"][1] - y) < 0.5, name

        font = directory / "font.typ"
        font.write_text('#set text(font: "libertinus")')
        config.write_text('[compiler]\nsystem_fonts = false\n')
        type_font = [key(0.80, "End", cmd=True), key(0.85, "Left"), key(0.90, "Left"),
                     key(1.00, " "), key(1.10, "s"), key(1.20, "e"), key(1.20, "r")]
        tree = capture(binary, font, "filtered-fonts", type_font, 1.65)
        labels = [node["text"] for node in by_key(tree, "completion-label")]
        assert '"Libertinus Serif"' in labels
        assert all("libertinus ser" in label.lower() for label in labels), labels

        tree = capture(binary, font, "accepted-font", type_font + [key(1.70, "Enter")], 2.00)
        assert any(node.get("text") == '#set text(font: "Libertinus Serif")'
                   for node in by_key(tree, "source"))

        menu = tap(0.80, "language-menu")
        open_problems = [menu, tap(0.95, "language-problems")]
        tree = capture(binary, chapter, "problems-dark", open_problems, 1.45, 1)
        assert by_key(tree, "problem-marker") and by_key(tree, "language-result-0")
        assert has_text(tree, "missing_diagnostic_name") and has_text(tree, "Problems")
        tree = capture(binary, chapter, "problem-jump", open_problems + [tap(1.50, "language-result-0")], 1.75)
        assert any(node.get("key", "").startswith("selection-") for node in tree["elements"])

        config.write_text('[ui]\ntheme = "intellij-light-classic"\n')
        outline = [menu, tap(0.95, "language-outline")]
        tree = capture(binary, main, "outline-light", outline, 1.40)
        assert has_text(tree, "Outline") and by_key(tree, "language-result-0")

        pin = [menu, tap(0.95, "language-pin"), dict(t=1.15, type="file-dialog", path=str(main))]
        tree = capture(binary, chapter, "pinned-light", pin, 1.65)
        assert by_key(tree, "pinned-main") and has_text(tree, "Main  main.typ") and has_text(tree, "Compile error")

        folded = [tap(0.85, "line-4/fold-toggle")]
        tree = capture(binary, main, "folded-light", folded, 1.25)
        assert by_key(tree, "folded-lines") and not has_text(tree, "The result follows.")

        rename = directory / "rename.typ"
        rename.write_text("#let item = 42\n\n#item")
        events = [key(0.75, "End", cmd=True), key(0.80, "Left"), key(0.90, "F2"),
                  typed(1.05, "result"), key(1.25, "Enter")]
        tree = capture(binary, rename, "rename-light", events, 1.65)
        assert any(node.get("text") == "#result" for node in by_key(tree, "source"))

        display = directory / "help.typ"
        display.write_text('#text("Hello")\n#rect(width: 10pt)')
        hover = [dict(t=0.80, type="tap", x=96, y=63), key(0.95, "F1")]
        tree = capture(binary, display, "hover-light", hover, 1.35)
        line = by_key(tree, "line-1")[0]["rect"]
        assert by_key(tree, "language-card")[0]["rect"][1] >= line[1] + line[3]
        assert by_key(tree, "card-content") and has_text(tree, "Customizes the look and layout of text")
        assert not by_key(tree, "card-title")

        x, y, w, h = by_key(tree, "source")[0]["rect"]
        advance = w / len('#text("Hello")')
        over_text = [
            dict(t=0.80, type="move", x=x + 1.2 * advance, y=y + h / 2),
            dict(t=1.00, type="move", x=x + 3.8 * advance, y=y + h / 2),
        ]
        tree = capture(binary, display, "pointer-hover", over_text, 1.42)
        assert has_text(tree, "Customizes the look and layout of text")

        card = by_key(tree, "language-card")[0]["rect"]
        enter = over_text + [dict(t=1.49, type="move", x=card[0] + 72, y=card[1] + 10)]
        tree = capture(binary, display, "hover-card-enter", enter, 2.10)
        assert has_text(tree, "Customizes the look and layout of text")
        leave = enter + [dict(t=2.15, type="move", x=x + w + 60, y=y + h / 2)]
        tree = capture(binary, display, "hover-card-leave", leave, 2.60)
        assert not by_key(tree, "language-card")

        blank = [dict(t=0.80, type="move", x=x + w + 60, y=y + h / 2)]
        tree = capture(binary, display, "hover-blank", blank, 1.50)
        assert not by_key(tree, "language-card")

        formatting = directory / "format.typ"
        formatting.write_text("#let answer=42\n#answer\n")
        events = [menu, tap(0.95, "language-format")]
        tree = capture(binary, formatting, "formatted-light", events, 1.40)
        assert any(node.get("text") == "#let answer = 42" for node in by_key(tree, "source"))

    print("Verified symbol and font completion, problems, diagnostic navigation, outline, pinning, folding, rename, hover and formatting")


if __name__ == "__main__":
    main()
