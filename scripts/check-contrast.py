#!/usr/bin/env python3
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
THEME = os.path.join(ROOT, "qml", "theme", "Theme.qml")

TEXT = 4.5
UI = 3.0
CONTRAST_TEXT = 7.0

PAIRS = [
    ("textPrimary", "bgBase", TEXT, "body text"),
    ("textPrimary", "bgRaised", TEXT, "text on panels"),
    ("textPrimary", "bgSunken", TEXT, "text in fields"),
    ("textPrimary", "bgHover", TEXT, "hovered row text"),
    ("textPrimary", "bgSelected", TEXT, "selected row text"),
    ("textSecondary", "bgBase", TEXT, "secondary text"),
    ("textSecondary", "bgRaised", TEXT, "secondary text on panels"),
    ("textSecondary", "bgSunken", TEXT, "helper text in fields"),
    ("textTertiary", "bgBase", TEXT, "placeholder and disabled text"),
    ("textTertiary", "bgSunken", TEXT, "placeholder in fields"),
    ("accent", "bgBase", TEXT, "quiet button and link text"),
    ("accent", "bgRaised", TEXT, "links on panels"),
    ("onAccent", "accent", TEXT, "primary button label"),
    ("onAccent", "accentHover", TEXT, "hovered primary button label"),
    ("onAccent", "accentPressed", TEXT, "pressed primary button label"),
    ("onDanger", "danger", TEXT, "filled danger button label"),
    ("danger", "bgBase", TEXT, "danger outline button label"),
    ("danger", "dangerBg", TEXT, "danger banner icon and border"),
    ("success", "bgBase", TEXT, "success text"),
    ("success", "successBg", UI, "success banner border"),
    ("warning", "bgBase", TEXT, "warning text"),
    ("warning", "warningBg", UI, "warning banner border"),
    ("info", "bgBase", TEXT, "info text"),
    ("info", "infoBg", UI, "info banner border"),
    ("textPrimary", "successBg", TEXT, "text in success banner"),
    ("textPrimary", "warningBg", TEXT, "text in warning banner"),
    ("textPrimary", "dangerBg", TEXT, "text in danger banner"),
    ("textPrimary", "infoBg", TEXT, "text in info banner"),
    ("borderStrong", "bgBase", UI, "control borders"),
    ("borderStrong", "bgSunken", UI, "field borders"),
    ("accent", "bgSunken", UI, "focus ring and slider fill on tracks"),
    ("accent", "bgBase", UI, "focus ring"),
    ("accent", "bgSelected", UI, "selection marker"),
]


def parse(path):
    src = open(path, encoding="utf-8").read()
    start = src.index("readonly property var palettes: ({")
    depth = 0
    i = src.index("{", src.index("({", start))
    j = i
    while True:
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                break
        j += 1
    return json.loads(src[i:j + 1])


def rgb(h):
    h = h.lstrip("#")
    if len(h) == 8:
        h = h[2:]
    return tuple(int(h[k:k + 2], 16) / 255 for k in (0, 2, 4))


def luminance(c):
    def ch(v):
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
    r, g, b = (ch(v) for v in c)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(a, b):
    la, lb = luminance(rgb(a)), luminance(rgb(b))
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


NOTCH = [
    ("notchInk", "notchBg", TEXT, "notch text"),
    ("notchMuted", "notchBg", TEXT, "notch secondary text"),
    ("notchListen", "notchBg", UI, "listening wave"),
    ("notchListenDeep", "notchBg", UI, "listening wave end"),
    ("notchSpeak", "notchBg", UI, "speaking wave"),
    ("notchSpeakDeep", "notchBg", UI, "speaking wave end"),
    ("notchTimer", "notchBg", UI, "timer ring"),
    ("notchDone", "notchBg", UI, "done tick"),
]


def notch_tokens(path):
    src = open(path, encoding="utf-8").read()
    return dict(re.findall(r'readonly property color (notch\w+): "(#[0-9A-Fa-f]{6})"', src))


def main():
    palettes = parse(THEME)
    palettes["notch"] = notch_tokens(THEME)
    failures = 0
    rows = []
    for name, pal in palettes.items():
        strict = name.startswith("contrast")
        for fg, bg, need, what in (NOTCH if name == "notch" else PAIRS):
            if fg not in pal or bg not in pal:
                print(f"{name}: missing token {fg if fg not in pal else bg}")
                failures += 1
                continue
            r = ratio(pal[fg], pal[bg])
            target = max(need, CONTRAST_TEXT) if strict and need == TEXT else need
            ok = r >= target
            rows.append((name, fg, bg, r, target, ok, what))
            if not ok:
                failures += 1
    width = max(len(f"{r[1]} on {r[2]}") for r in rows)
    for name, fg, bg, r, target, ok, what in rows:
        if not ok or "-v" in sys.argv:
            print(f"{'PASS' if ok else 'FAIL'}  {name:<13} {(fg + ' on ' + bg):<{width}}  {r:5.2f} (needs {target:.1f})  {what}")
    print(f"{len(rows) - failures} of {len(rows)} pairs pass")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
