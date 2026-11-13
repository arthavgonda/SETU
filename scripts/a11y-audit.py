#!/usr/bin/env python3
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
QML = [os.path.join(d, f) for d, _, fs in os.walk(os.path.join(ROOT, "qml")) for f in fs if f.endswith(".qml")]
CPP = [os.path.join(d, f) for d, _, fs in os.walk(os.path.join(ROOT, "src")) for f in fs if f.endswith((".cpp", ".h"))]

problems = []


def report(path, line, msg):
    problems.append(f"{os.path.relpath(path, ROOT)}:{line}: {msg}")


def blocks(src, type_name):
    for m in re.finditer(r"(?m)^(\s*)" + re.escape(type_name) + r"\s*\{", src):
        indent = m.group(1)
        start = m.end()
        end = re.search(r"(?m)^" + re.escape(indent) + r"\}", src[start:])
        body = src[start:start + end.start()] if end else src[start:]
        yield src.count("\n", 0, m.start()) + 1, body


def own_props(body):
    depth = 0
    out = []
    for line in body.splitlines():
        if depth == 0:
            out.append(line)
        depth += line.count("{") - line.count("}")
    return "\n".join(out)


for path in QML:
    src = open(path, encoding="utf-8").read()
    rel = os.path.relpath(path, ROOT)
    for n, line in enumerate(src.splitlines(), 1):
        if re.search(r"Text\.(RichText|StyledText|MarkdownText)|TextEdit\.(RichText|MarkdownText)", line):
            report(path, n, "text from the daemon must be rendered as plain text")
        if "theme/Theme.qml" not in rel and re.search(r'"#[0-9A-Fa-f]{6,8}"', line):
            report(path, n, "colour literal outside Theme.qml; use a token")
        m = re.search(r'\b(text|title|label|placeholder|description|secondary|actionText|emptyText|tip|consequence|actionLabel)\s*:\s*"([^"]*[A-Za-z][^"]*)"', line)
        if m and "tests/" not in rel:
            report(path, n, f'user-visible string "{m.group(2)}" is not wrapped in qsTr')
        if re.search(r"push.to.talk|hold.to.talk", line, re.I) and "no push-to-talk" not in line.lower():
            report(path, n, "push-to-talk must not exist")
        if "XMLHttpRequest" in line:
            report(path, n, "the UI makes no network calls")
    for n, body in blocks(src, "IconButton"):
        if not re.search(r"^\s*label\s*:", own_props(body), re.M):
            report(path, n, "IconButton without a label has no accessible name")
    for n, body in blocks(src, "Button"):
        props = own_props(body)
        if not re.search(r"^\s*(text|Accessible\.name)\s*:", props, re.M):
            report(path, n, "Button without text or Accessible.name")
    for n, body in blocks(src, "MouseArea"):
        props = own_props(body)
        if re.search(r"onClicked|onPressed", props) and not re.search(r"Keys\.|Accessible\.onPressAction|activeFocusOnTab|focusPolicy", src):
            report(path, n, "mouse-only interaction without a keyboard or accessibility action")
    for n, body in blocks(src, "Image"):
        props = own_props(body)
        if not re.search(r"Accessible\.(name|ignored)", props):
            report(path, n, "Image needs Accessible.name, or Accessible.ignored when decorative")

for path in CPP:
    src = open(path, encoding="utf-8").read()
    for n, line in enumerate(src.splitlines(), 1):
        if re.search(r"QNetworkAccessManager|QNetworkRequest|QTcpSocket|QUdpSocket|QWebSocket", line):
            report(path, n, "the UI makes no network calls; the daemon owns the network")

for p in problems:
    print(p)
print(f"{len(problems)} problems in {len(QML)} QML and {len(CPP)} C++ files")
sys.exit(1 if problems else 0)
