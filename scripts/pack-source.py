#!/usr/bin/env python3
import argparse
import gzip
import io
import os
import re
import tarfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STAMP = 1767225600
TREES = ["build-macos.sh", "CMakeLists.txt", "assets", "daemon", "i18n", "packaging/macos", "qml", "scripts", "src"]
LLAMA = ["CMakeLists.txt", "CMakePresets.json", "LICENSE", "licenses", "cmake", "include", "src", "ggml", "vendor", "common", "tools/mtmd"]
SKIP_DIRS = {"daemon/target", "daemon/fuzz", "__pycache__"}


def version():
    with open(os.path.join(ROOT, "CMakeLists.txt"), encoding="utf-8") as f:
        return re.search(r"project\(Setu VERSION ([0-9.]+)", f.read()).group(1)


def wanted(rel):
    name = os.path.basename(rel)
    return not (name.startswith(".") or name.endswith((".md", ".pyc")) or name in SKIP_DIRS or rel in SKIP_DIRS)


def walk(rel):
    path = os.path.join(ROOT, rel)
    if not wanted(rel):
        return
    if os.path.isfile(path):
        yield rel
        return
    for name in sorted(os.listdir(path)):
        yield from walk(os.path.join(rel, name))


def main():
    ap = argparse.ArgumentParser(description="Pack the source needed to build the macOS disk image")
    ap.add_argument("--out", help="archive to write")
    a = ap.parse_args()
    ver = version()
    out = a.out or os.path.join(ROOT, "dist", f"Setu-{ver}-macos-source.tar.gz")
    top = f"setu-{ver}"
    files = [f for t in TREES for f in walk(t)] + [f for t in LLAMA for f in walk(os.path.join("llama.cpp", t))]
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.PAX_FORMAT) as tar:
        for rel in files:
            path = os.path.join(ROOT, rel)
            info = tarfile.TarInfo(f"{top}/{rel}")
            info.size = os.path.getsize(path)
            info.mtime = STAMP
            info.mode = 0o755 if os.access(path, os.X_OK) else 0o644
            info.uname = info.gname = ""
            with open(path, "rb") as f:
                tar.addfile(info, f)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    tmp = out + ".tmp"
    with open(tmp, "wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=9) as gz:
        gz.write(buf.getvalue())
    os.replace(tmp, out)
    print(f"{len(files)} files -> {out}")


if __name__ == "__main__":
    main()
