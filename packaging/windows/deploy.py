#!/usr/bin/env python3
import argparse
import os
import re
import shutil
import subprocess
import sys

SYSTEM = re.compile(r"^(api-ms-win-.*|ext-ms-.*|bcryptprimitives|kernel32|user32|gdi32|advapi32|ole32|oleaut32|shell32|shlwapi|ws2_32|bcrypt|ntdll|userenv|secur32|crypt32|dwmapi|uxtheme|winmm|imm32|version|setupapi|dbghelp|dxgi|d3d\d+|d3dcompiler_\d+|dwrite|d2d1|propsys|powrprof|mf\w*|ksuser|avrt|iphlpapi|netapi32|wtsapi32|comdlg32|comctl32|winspool|mpr|authz|credui|uiautomationcore|cfgmgr32|rpcrt4|combase|pdh|psapi|opengl32|glu32|dnsapi|ncrypt|wintrust|msvcrt|ucrtbase|shcore|wininet|winhttp|dbgeng|hid|ntdsapi|normaliz|virtdisk|esent|mmdevapi|oleacc|textinputframework|coremessaging|wldap32|cryptbase|sspicli|fwpuclnt|vulkan-1|nvcuda)\.dll$", re.I)

QML_DIRS = [
    "QtQml",
    "QtQuick/Window",
    "QtQuick/Layouts",
    "QtQuick/Templates",
    "QtQuick/Controls/Basic",
    "QtQuick/Controls/impl",
    "QtQuick/Dialogs",
    "Qt/labs/platform",
]
QML_FILES = ["QtQuick/qmldir", "QtQuick/qtquick2plugin.dll", "QtQuick/plugins.qmltypes", "QtQuick/Controls/qmldir", "QtQuick/Controls/qtquickcontrols2plugin.dll", "QtQuick/Controls/plugins.qmltypes"]
PLUGINS = ["platforms/qwindows.dll", "imageformats/qsvg.dll", "iconengines/qsvgicon.dll"]


def imports(objdump, path):
    out = subprocess.run([objdump, "-p", path], capture_output=True, text=True).stdout
    return re.findall(r"DLL Name: (\S+)", out)


def copy(src, dst):
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(src, dst)


def main():
    ap = argparse.ArgumentParser(description="Assemble a runnable Windows folder from a cross build")
    ap.add_argument("--out", required=True)
    ap.add_argument("--qt", required=True, help="Qt for Windows prefix, for example .../6.10.2/llvm-mingw_64")
    ap.add_argument("--objdump", required=True)
    ap.add_argument("--exe", action="append", default=[], help="executables to ship")
    ap.add_argument("--dll-dir", action="append", default=[], help="folders searched for dependent DLLs, in order")
    ap.add_argument("--extra-dll", action="append", default=[], help="DLLs loaded at run time that no import table names")
    ap.add_argument("--espeak", help="espeak-ng.exe")
    ap.add_argument("--espeak-data", help="folder named espeak-ng-data")
    ap.add_argument("--models", required=True, help="bundled models folder")
    ap.add_argument("--benchmark", required=True)
    ap.add_argument("--license", action="append", default=[], help="NAME=PATH licence files for the licences folder")
    a = ap.parse_args()

    out = os.path.abspath(a.out)
    if os.path.exists(out):
        shutil.rmtree(out)
    os.makedirs(out)
    for e in a.exe:
        copy(e, os.path.join(out, os.path.basename(e)))
    for d in QML_DIRS:
        src = os.path.join(a.qt, "qml", d)
        if os.path.isdir(src):
            shutil.copytree(src, os.path.join(out, "qml", d), ignore=shutil.ignore_patterns("*.pdb", "*.debug", "*.qmltypes", "XmlListModel", "StateMachine"), dirs_exist_ok=True)
    for f in QML_FILES:
        src = os.path.join(a.qt, "qml", f)
        if os.path.exists(src):
            copy(src, os.path.join(out, "qml", f))
    for p in PLUGINS:
        copy(os.path.join(a.qt, "plugins", p), os.path.join(out, "plugins", p))
    for d in a.extra_dll:
        copy(d, os.path.join(out, os.path.basename(d)))
    search = [os.path.join(a.qt, "bin")] + a.dll_dir
    seen = set()
    missing = set()
    queue = [os.path.join(dp, f) for dp, _, fs in os.walk(out) for f in fs if f.lower().endswith((".exe", ".dll"))]
    while queue:
        path = queue.pop()
        for dll in imports(a.objdump, path):
            key = dll.lower()
            if key in seen or SYSTEM.match(dll):
                continue
            seen.add(key)
            if os.path.exists(os.path.join(out, dll)):
                continue
            for d in a.dll_dir + search:
                cand = os.path.join(d, dll)
                if os.path.exists(cand):
                    dst = os.path.join(out, dll)
                    copy(cand, dst)
                    queue.append(dst)
                    break
            else:
                missing.add(dll)
    with open(os.path.join(out, "qt.conf"), "w") as f:
        f.write("[Paths]\nPrefix = .\nPlugins = plugins\nQmlImports = qml\n")
    shutil.copytree(a.models, os.path.join(out, "share", "models"), ignore=shutil.ignore_patterns(".stamp"))
    copy(a.benchmark, os.path.join(out, "share", "benchmark.wav"))
    if a.espeak:
        copy(a.espeak, os.path.join(out, "espeak-ng.exe"))
    if a.espeak_data:
        shutil.copytree(a.espeak_data, os.path.join(out, "espeak-ng-data"))
    for item in a.license:
        name, path = item.split("=", 1)
        copy(path, os.path.join(out, "licenses", name))
    total = sum(os.path.getsize(os.path.join(dp, f)) for dp, _, fs in os.walk(out) for f in fs)
    print(f"{out}: {total / 1e6:.1f} MB")
    if missing:
        print("not found (check they are system DLLs): " + ", ".join(sorted(missing)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
