#!/usr/bin/env python3
import argparse
import datetime
import hashlib
import json
import os
import re
import sys


def spdx_id(name, version, i):
    return "SPDXRef-" + re.sub(r"[^A-Za-z0-9.-]", "-", f"{name}-{version}") + f"-{i}"


def main():
    ap = argparse.ArgumentParser(description="Write an SPDX 2.3 SBOM from the generated third-party license list")
    ap.add_argument("--licenses", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    with open(a.licenses, encoding="utf-8") as f:
        data = json.load(f)
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0")) or int(datetime.datetime.now(datetime.timezone.utc).timestamp())
    created = datetime.datetime.fromtimestamp(epoch, datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    root = "SPDXRef-Package-Setu"
    packages = [{
        "SPDXID": root,
        "name": "Setu",
        "versionInfo": a.version,
        "downloadLocation": "NOASSERTION",
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": "NOASSERTION",
        "copyrightText": "NOASSERTION",
        "supplier": "NOASSERTION",
    }]
    relationships = [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": root}]
    for i, c in enumerate(data["components"]):
        sid = spdx_id(c["name"], c.get("version", ""), i)
        lic = c.get("license") or "NOASSERTION"
        lic = lic.replace("/", " OR ")
        if not re.fullmatch(r"[A-Za-z0-9.+\- ()]+", lic) or "WITH" in lic and "LLVM" not in lic:
            lic = "NOASSERTION"
        pkg = {
            "SPDXID": sid,
            "name": c["name"],
            "versionInfo": c.get("version") or "NOASSERTION",
            "downloadLocation": c.get("url") or "NOASSERTION",
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": lic,
            "copyrightText": "NOASSERTION",
            "comment": c.get("kind", ""),
        }
        if c.get("kind") == "Rust crate":
            pkg["externalRefs"] = [{"referenceCategory": "PACKAGE-MANAGER", "referenceType": "purl", "referenceLocator": f"pkg:cargo/{c['name']}@{c['version']}"}]
        packages.append(pkg)
        rel = "DEPENDS_ON" if not c.get("kind", "").startswith("Model") else "OTHER"
        relationships.append({"spdxElementId": root, "relationshipType": rel, "relatedSpdxElement": sid})
    doc = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"setu-{a.version}",
        "documentNamespace": "https://setu.invalid/spdx/" + hashlib.sha256(f"{a.version}-{created}".encode()).hexdigest(),
        "creationInfo": {"created": created, "creators": ["Tool: setu-sbom"]},
        "packages": packages,
        "relationships": relationships,
    }
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(doc, f, indent=1)
    print(f"{len(packages)} packages -> {a.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
