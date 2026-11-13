#!/usr/bin/env python3
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

LGPL = {
    "Core", "Gui", "Qml", "QmlModels", "QmlMeta", "QmlWorkerScript", "Quick", "QuickControls2", "QuickControls2Impl",
    "QuickControls2Basic", "QuickControls2BasicStyleImpl", "QuickTemplates2", "QuickLayouts", "QuickDialogs2",
    "QuickDialogs2QuickImpl", "QuickDialogs2Utils", "LabsPlatform", "Widgets", "Network", "Svg", "DBus", "OpenGL",
    "XcbQpa", "WaylandClient", "WlShellIntegration", "LayerShellQtInterface", "QuickShapes", "Concurrent", "QmlCore",
}
IMPORTS = {
    "QtQuick": "Quick", "QtQuick.Layouts": "QuickLayouts", "QtQuick.Templates": "QuickTemplates2", "QtQuick.Controls": "QuickControls2",
    "QtQuick.Dialogs": "QuickDialogs2", "QtQuick.Window": "Quick", "QtQml": "Qml", "QtQml.Models": "QmlModels", "Qt.labs.platform": "LabsPlatform",
    "QtQuick.Shapes": "QuickShapes", "QtTest": "QuickTest",
}
FORBIDDEN = {"Charts", "DataVisualization", "VirtualKeyboard", "Graphs", "Quick3DPhysics", "Pdf", "Lottie", "ShaderTools", "Quick3D", "WebEngineCore"}


def linked(binary):
    tool = shutil.which("ldd") if sys.platform.startswith("linux") else shutil.which("otool")
    if not tool:
        return set()
    args = [tool, binary] if tool.endswith("ldd") else [tool, "-L", binary]
    out = subprocess.run(args, capture_output=True, text=True).stdout
    return {a or b for a, b in re.findall(r"libQt6([A-Za-z0-9]+)\.so|Qt([A-Za-z0-9]+)\.framework", out)}


def imported():
    mods = {}
    for d, _, files in os.walk(os.path.join(ROOT, "qml")):
        for f in files:
            if f.endswith(".qml"):
                for m in re.findall(r"(?m)^import\s+((?:Qt|QtQuick|QtQml)[A-Za-z0-9.]*)", open(os.path.join(d, f), encoding="utf-8").read()):
                    mods.setdefault(m, os.path.relpath(os.path.join(d, f), ROOT))
    return mods


def main():
    build = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "build", "qt")
    bad = []
    seen = set()
    for exe in ("setu-shell", "setu-ui"):
        path = os.path.join(build, exe)
        if os.path.exists(path):
            for m in linked(path):
                seen.add(m)
                if m in FORBIDDEN or m not in LGPL:
                    bad.append(f"{exe} links Qt {m}, which is not on the LGPL allow-list")
    for imp, where in sorted(imported().items()):
        mod = IMPORTS.get(imp)
        if mod is None:
            bad.append(f"{where} imports {imp}, which is not on the allow-list")
        else:
            seen.add(mod)
    print("Qt modules in use: " + ", ".join(sorted(seen)))
    for b in bad:
        print(b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
