#!/usr/bin/env python3
import argparse
import os
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCES = ["qml", "src"]

SINGULAR = {
    "%1: active for %n more minutes. Say %2, then your command.": "%1: active for %n more minute. Say %2, then your command.",
    "%1. %n syllables. %2": "%1. %n syllable. %2",
    "Asked %n times, most recently %1": "Asked once, most recently %1",
    "Cancels by itself in %n seconds.": "Cancels by itself in %n second.",
    "Closes in %n s": "Closes in %n s",
    "Deleted %n items.": "Deleted %n item.",
    "Delete %n actions": "Delete %n action",
    "Exported %n items.": "Exported %n item.",
    "Found %n apps.": "Found %n app.",
    "%n actions": "%n action",
    "%n apps": "%n app",
    "%n chosen folders are removed": "%n chosen folder is removed",
    "%n components": "%n component",
    "%n hidden": "%n hidden",
    "%n history items": "%n history item",
    "%n items": "%n item",
    "%n minutes": "%n minute",
    "%n seconds": "%n second",
    "Opened %n times": "Opened once",
    "Say it %n more times.": "Say it %n more time.",
    "%n s": "%n s",
    "%n timers running": "%n timer running",
    "%n to-dos left": "%n to-do left",
    "%n commands": "%n command",
}

ACCENTS = str.maketrans("aceinosuyACEINOSUY", "àçéîñøšüýÀÇÉÎÑØŠÜÝ")


def tool(name):
    for cand in (name + "-qt6", name, "/usr/lib64/qt6/bin/" + name, "/usr/lib/qt6/bin/" + name):
        path = shutil.which(cand) or (cand if os.path.isfile(cand) else None)
        if path:
            return path
    sys.exit(f"{name} not found; install the Qt 6 Linguist tools")


def lupdate(ts):
    subprocess.run([tool("lupdate"), "-silent", "-locations", "none", "-extensions", "qml,cpp,h"] + [os.path.join(ROOT, s) for s in SOURCES] + ["-ts", ts], check=True)


def fill_english(ts):
    tree = ET.parse(ts)
    missing = []
    for msg in tree.iter("message"):
        src = msg.findtext("source") or ""
        tr = msg.find("translation")
        if msg.get("numerus") != "yes":
            continue
        one = SINGULAR.get(src)
        if one is None:
            missing.append(src)
            continue
        for child in list(tr):
            tr.remove(child)
        tr.attrib.pop("type", None)
        tr.text = None
        for form in (one, src):
            e = ET.SubElement(tr, "numerusform")
            e.text = form
    if missing:
        sys.exit("Add a singular form to SINGULAR in scripts/i18n.py for:\n  " + "\n  ".join(missing))
    tree.write(ts, encoding="utf-8", xml_declaration=True)


def pseudo(text):
    out = []
    i = 0
    while i < len(text):
        if text[i] == "%" and i + 1 < len(text) and (text[i + 1].isdigit() or text[i + 1] == "n"):
            out.append(text[i:i + 2])
            i += 2
            continue
        out.append(text[i].translate(ACCENTS))
        i += 1
    s = "".join(out)
    pad = max(1, len(text) * 2 // 5)
    return "[" + s + " " + ("·" * pad) + "]"


def make_pseudo(source_ts, out_ts, lang):
    tree = ET.parse(source_ts)
    tree.getroot().set("language", lang)
    for msg in tree.iter("message"):
        src = msg.findtext("source") or ""
        tr = msg.find("translation")
        for child in list(tr):
            tr.remove(child)
        tr.attrib.pop("type", None)
        if msg.get("numerus") == "yes":
            tr.text = None
            for form in (SINGULAR.get(src, src), src):
                e = ET.SubElement(tr, "numerusform")
                e.text = pseudo(form)
        else:
            tr.text = pseudo(src)
    os.makedirs(os.path.dirname(os.path.abspath(out_ts)), exist_ok=True)
    tree.write(out_ts, encoding="utf-8", xml_declaration=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--update", action="store_true", help="refresh i18n/*.ts from the sources")
    ap.add_argument("--pseudo", metavar="DIR", help="write pseudo-locale .ts files into DIR")
    a = ap.parse_args()
    en = os.path.join(ROOT, "i18n", "setu_en.ts")
    if a.update or not os.path.exists(en):
        os.makedirs(os.path.dirname(en), exist_ok=True)
        for f in sorted(os.listdir(os.path.dirname(en))):
            if f.endswith(".ts"):
                lupdate(os.path.join(os.path.dirname(en), f))
        if not os.path.exists(en):
            lupdate(en)
        fill_english(en)
    if a.pseudo:
        make_pseudo(en, os.path.join(a.pseudo, "setu_qps-ploc.ts"), "en")
        make_pseudo(en, os.path.join(a.pseudo, "setu_qps-plocm.ts"), "en")


if __name__ == "__main__":
    main()
