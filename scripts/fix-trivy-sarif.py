#!/usr/bin/env python3
"""Post-process a Trivy SBOM-mode SARIF so GitHub Code Scanning accepts it.

Trivy's `sbom` scanner emits results with an empty artifactLocation.uri and a
dangling uriBaseId ("ROOTPATH") that is never defined in originalUriBaseIds.
GitHub rejects the whole file with "locationFromSarifResult: expected artifact
location".

This script points every result at Cargo.lock (the lock file in which the
dependency is declared, matching trivy fs output) and drops the dangling
uriBaseId so each artifactLocation carries a concrete uri.
"""
import json
import sys


def main(path):
    with open(path) as f:
        sarif = json.load(f)

    fixed = 0
    for run in sarif.get("runs", []):
        for result in run.get("results", []):
            for loc in result.get("locations", []):
                phys = loc.get("physicalLocation") or {}
                al = phys.get("artifactLocation") or {}
                if not al.get("uri"):
                    al["uri"] = "Cargo.lock"
                al.pop("uriBaseId", None)
                phys["artifactLocation"] = al
                loc["physicalLocation"] = phys
                fixed += 1

    with open(path, "w") as f:
        json.dump(sarif, f, indent=2)
    print(f"fixed {fixed} locations in {path}")


if __name__ == "__main__":
    main(sys.argv[1])
