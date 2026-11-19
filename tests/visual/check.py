"""Render Fast through Rune and check its initial window and theme controls."""

import json
import os
from pathlib import Path
import struct
import subprocess
import sys
from tempfile import TemporaryDirectory


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(__file__).with_name("appearance.json")
DEFAULT_STATUS = None


def luminance(color):
    rgb = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    linear = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in rgb]
    return sum(c * weight for c, weight in zip(linear, (0.2126, 0.7152, 0.0722)))


def inspect(tree, viewport, scale, theme, status, menu_open, focus):
    assert tree["viewport"] == list(viewport)
    assert tree["scale"] == scale
    assert not tree["warnings"], tree["warnings"]
    assert not tree["animating"]
    elements = {element["path"]: element for element in tree["elements"]}
    for path, element in elements.items():
        x, y, w, h = element["rect"]
        parent, _, _ = path.rpartition("/")
        px, py, pw, ph = elements[parent]["rect"] if parent else (0, 0, *viewport)
        assert w > 0 and h > 0, (path, element["rect"])
        assert px - 0.02 <= x and x + w <= px + pw + 0.02, (path, element["rect"])
        assert py - 0.02 <= y and y + h <= py + ph + 0.02, (path, element["rect"])
        if "text" in element:
            while elements[parent]["fill"][-2:] == "00":
                parent = parent.rpartition("/")[0]
            low, high = sorted((luminance(element["color"]), luminance(elements[parent]["fill"])))
            assert (high + 0.05) / (low + 0.05) >= 4.5, (path, "text contrast")

    toolbar = "window/document-bar"
    find, replace, settings = (elements[f"{toolbar}/{key}"]["rect"] for key in ("find-file", "replace-file", "settings-menu"))
    assert find[0] + find[2] < replace[0] and replace[0] + replace[2] < settings[0]
    assert settings[1] == replace[1] == find[1]
    assert elements[toolbar]["rect"][1] == 0.0
    assert "window/header" not in elements
    status_element = elements.get("window/status-bar/config-status")
    assert (status_element and status_element["text"]) == status
    if status == "Configuration error":
        assert "ui.font_size" in status_element["help"]
    dark = luminance(elements["window"]["fill"]) < 0.5
    assert dark == (theme == "darcula")

    def descendants(node):
        yield node
        for child in node.get("children", []):
            yield from descendants(child)

    popup = elements.get("window/settings-popup")
    radio = [node for node in descendants(tree["accessibility"]) if node.get("key") in ("darcula", "light")]
    if menu_open:
        assert popup is not None and popup["rect"][1] >= settings[1] + settings[3]
        assert [node["key"] for node in radio] == ["darcula", "light"]
        assert all(node["role"] == "menu-item-radio" for node in radio)
        assert [node["key"] for node in radio if "checked" in node["states"]] == [theme]
    else:
        assert popup is None and not radio
    if focus:
        assert elements[focus]["focused"], focus


def capture(output, config, name, scale, theme, status, *, viewport=(760, 460), at=None, menu_open=False, focus=None):
    png = output / f"{name}.png"
    dump = output / f"{name}.json"
    command = [
        "cargo", "run", "--locked", "--quiet", "-p", "fast", "--",
        "--size", f"{viewport[0]}x{viewport[1]}", "--scale", str(scale),
        "--snap", str(png), "--tree", str(dump), "--at", str(0.12 if at is None else at),
    ]
    if at is not None:
        command.extend(("--script", str(SCRIPT)))
    subprocess.run(command, cwd=ROOT, check=True, env={**os.environ, "FAST_CONFIG": str(config)})
    with png.open("rb") as image:
        header = image.read(24)
    assert header.startswith(b"\x89PNG\r\n\x1a\n"), png
    assert struct.unpack(">II", header[16:24]) == tuple(point * scale for point in viewport), png
    inspect(json.loads(dump.read_text()), viewport, scale, theme, status, menu_open, focus)
    print(f"Verified {name}", flush=True)


def main():
    output = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "target" / "visual"
    output.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix="fast-visual-") as temporary:
        config = Path(temporary) / "config.toml"
        for scale in (1, 2):
            capture(output, config, f"darcula-{scale}x", scale, "darcula", DEFAULT_STATUS)
        capture(output, config, "settings-dark", 2, "darcula", DEFAULT_STATUS, at=0.22, menu_open=True)
        capture(output, config, "settings-keyboard", 2, "darcula", DEFAULT_STATUS, at=0.98,
                menu_open=True, focus="window/settings-popup/light")
        capture(output, config, "keyboard-light", 2, "light", DEFAULT_STATUS, at=1.48,
                focus="window/document-bar/settings-menu")
        capture(output, config, "settings-light", 1, "light", DEFAULT_STATUS, at=2.02, menu_open=True)
        capture(output, config, "pointer-darcula", 2, "darcula", DEFAULT_STATUS, at=2.55)
        capture(output, config, "settings-escape", 1, "darcula", DEFAULT_STATUS, at=2.94)
        capture(output, config, "settings-outside", 1, "darcula", DEFAULT_STATUS, at=4.00)
        capture(output, config, "hover-exit", 1, "darcula", DEFAULT_STATUS, at=4.24)

        config.write_text('[ui]\ntheme = "intellij-light-classic"\n')
        for scale in (1, 2):
            capture(output, config, f"light-{scale}x", scale, "light", "Configuration loaded")
        config.write_text('[ui]\ntheme = "intellij-light-classic"\nfont_size = 16\n')
        capture(output, config, "light-minimum-size", 2, "light", "Configuration loaded", viewport=(440, 240))
        capture(output, config, "settings-minimum-size", 2, "light", "Configuration loaded",
                viewport=(440, 240), at=0.48, menu_open=True)
        config.write_text("[ui]\nfont_size = 17\n")
        capture(output, config, "invalid-config", 1, "darcula", "Configuration error")


if __name__ == "__main__":
    main()
