#!/usr/bin/env python3
import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

QT_NOTICE = """This application uses the Qt framework, version 6, under the GNU Lesser General Public License version 3 (LGPL-3.0-only).

Qt is linked dynamically. You may replace the Qt libraries shipped with this application with your own build:

1. Get the Qt source for the version listed above from https://download.qt.io/official_releases/qt/ or https://code.qt.io.
2. Build it with the same major version and configure options documented at https://doc.qt.io/qt-6/build-sources.html.
3. Replace the Qt libraries in the application's library folder (lib on Linux, the app bundle's Frameworks folder on macOS, the install folder on Windows) with the ones you built.

The application's own code does not need to be rebuilt for this. If you need the exact build configuration used, or a copy of the Qt source on a physical medium for three years from the date you received this application, ask through the support link in About.

The full LGPL-3.0 and GPL-3.0 texts follow."""

MIT_ORT = """MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""

ESPEAK_NOTICE = """espeak-ng is a separate program that this application runs to turn text into phonemes for speech. It is not linked into the application. It is distributed under the GNU General Public License version 3 or later. Its source is available at https://github.com/espeak-ng/espeak-ng."""


class Texts:
    def __init__(self):
        self.map = {}

    def add(self, text):
        text = text.replace("\r\n", "\n").strip()
        if not text:
            return None
        key = hashlib.sha1(text.encode()).hexdigest()[:12]
        self.map[key] = text
        return key

    def files(self, paths):
        ids = []
        for p in paths:
            try:
                with open(p, encoding="utf-8", errors="replace") as f:
                    t = self.add(f.read())
                if t and t not in ids:
                    ids.append(t)
            except OSError:
                pass
        return ids


def license_files(directory):
    out = []
    for pat in ("LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "license*", "UNLICENSE*"):
        out += [p for p in glob.glob(os.path.join(directory, pat)) if os.path.isfile(p)]
    return sorted(set(out))


def cargo_packages(target, texts):
    cmd = ["cargo", "metadata", "--format-version", "1", "--filter-platform", target]
    env = dict(os.environ)
    cargo_home = os.path.join(os.path.expanduser("~"), ".cargo", "bin")
    env["PATH"] = cargo_home + os.pathsep + env.get("PATH", "")
    try:
        meta = json.loads(subprocess.run(cmd + ["--offline"], cwd=os.path.join(ROOT, "daemon"), env=env, check=True, capture_output=True).stdout)
    except subprocess.CalledProcessError:
        meta = json.loads(subprocess.run(cmd, cwd=os.path.join(ROOT, "daemon"), env=env, check=True, capture_output=True).stdout)
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    seen = set()
    stack = list(members)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes.get(pid, {}).get("deps", []):
            kinds = [k.get("kind") for k in dep.get("dep_kinds", [])]
            if kinds and all(k == "dev" for k in kinds):
                continue
            stack.append(dep["pkg"])
    out = []
    generic_mit = MIT_ORT.replace("Copyright (c) Microsoft Corporation", "Copyright (c) the authors of {name}")
    for pid in sorted(seen - members, key=lambda i: (packages[i]["name"], packages[i]["version"])):
        p = packages[pid]
        out.append({
            "name": p["name"],
            "version": p["version"],
            "license": p.get("license") or "See license text",
            "kind": "Rust crate",
            "url": p.get("repository") or p.get("homepage") or "",
            "texts": texts.files(license_files(os.path.dirname(p["manifest_path"]))) or ([texts.add(generic_mit.format(name=p["name"]))] if "MIT" in (p.get("license") or "") else []),
        })
    return out


def static_components(texts, qt_version, qt_licenses):
    out = []
    qt_texts = [texts.add(QT_NOTICE)] + texts.files([os.path.join(qt_licenses, n) for n in ("LGPL-3.0-only.txt", "GPL-3.0-only.txt")])
    out.append({"name": "Qt", "version": qt_version, "license": "LGPL-3.0-only", "kind": "Library", "url": "https://www.qt.io", "texts": [t for t in qt_texts if t]})
    out.append({"name": "sherpa-onnx", "version": "2.0.1", "license": "Apache-2.0", "kind": "Library", "url": "https://github.com/k2-fsa/sherpa-onnx", "texts": texts.files([os.path.join(ROOT, "third_party/sherpa-onnx/LICENSE")])})
    out.append({"name": "ONNX Runtime", "version": "", "license": "MIT", "kind": "Library", "url": "https://github.com/microsoft/onnxruntime", "texts": [texts.add(MIT_ORT)]})
    llama = os.environ.get("LLAMA_CPP_DIR", os.path.join(ROOT, "llama.cpp"))
    out.append({"name": "llama.cpp and ggml", "version": "", "license": "MIT", "kind": "Library", "url": "https://github.com/ggml-org/llama.cpp", "texts": texts.files([os.path.join(llama, "LICENSE")])})
    espeak = [texts.add(ESPEAK_NOTICE)] + texts.files(sorted(glob.glob("/usr/share/licenses/espeak-ng/COPYING*")) or [os.path.join(qt_licenses, "GPL-3.0-only.txt")])
    out.append({"name": "espeak-ng", "version": "", "license": "GPL-3.0-or-later", "kind": "Separate program", "url": "https://github.com/espeak-ng/espeak-ng", "texts": [t for t in espeak if t]})
    out.append({"name": "Lucide icons", "version": "", "license": "ISC", "kind": "Icons", "url": "https://lucide.dev", "texts": texts.files([os.path.join(ROOT, "assets/icons/LICENSE")])})
    fonts = {
        "Atkinson Hyperlegible": "OFL-AtkinsonHyperlegible.txt",
        "IBM Plex Sans": "OFL-IBMPlexSans.txt",
        "IBM Plex Mono": "OFL-IBMPlexMono.txt",
        "Noto Sans Devanagari": "OFL-NotoSansDevanagari.txt",
        "Source Serif 4": "OFL-SourceSerif4.txt",
    }
    for name, f in fonts.items():
        out.append({"name": name, "version": "", "license": "OFL-1.1", "kind": "Font", "url": "", "texts": texts.files([os.path.join(ROOT, "assets/fonts", f)])})
    with open(os.path.join(ROOT, "daemon/core/assets/manifest.json"), encoding="utf-8") as f:
        manifest = json.load(f)
    for m in manifest.get("models", []):
        out.append({"name": m["name"], "version": m.get("id", ""), "license": m.get("license", ""), "kind": "Model, downloaded separately", "url": (m.get("urls") or [m.get("url", "")])[0] if (m.get("urls") or m.get("url")) else "", "texts": []})
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--target", default="")
    ap.add_argument("--qt-version", default="6")
    ap.add_argument("--qt-licenses", default="/usr/share/licenses/qt6-qtbase")
    a = ap.parse_args()
    target = a.target
    if not target:
        v = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, env=dict(os.environ, PATH=os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + os.environ.get("PATH", ""))).stdout
        target = next((l.split(":", 1)[1].strip() for l in v.splitlines() if l.startswith("host:")), "x86_64-unknown-linux-gnu")
    texts = Texts()
    components = static_components(texts, a.qt_version, a.qt_licenses) + cargo_packages(target, texts)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    tmp = a.out + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump({"target": target, "components": components, "texts": texts.map}, f, ensure_ascii=False, separators=(",", ":"))
    os.replace(tmp, a.out)
    print(f"{len(components)} components, {len(texts.map)} license texts -> {a.out}")


if __name__ == "__main__":
    sys.exit(main())
