#!/usr/bin/env python3
"""Prune a CycloneDX SBOM to the crates actually in the build graph.

cargo-cyclonedx emits optional dependencies of transitive crates even when
their feature is never enabled (for example ratatui's termwiz backend pulls
in terminfo). Those phantom components make Trivy report findings for crates
that are not shipped. This script drops any component whose (name, version)
is not present in ``cargo tree --all-features``.

The script is best-effort: if cargo tree cannot run, the SBOM is left
untouched and a warning is printed, so a transient tooling failure never
blocks the security scan.
"""

import json
import os
import re
import subprocess
import sys


def repo_root():
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def real_graph():
    root = repo_root()
    out = subprocess.check_output(
        ["cargo", "tree", "--all-features", "--prefix", "none", "--format", "{p}"],
        text=True,
        stderr=subprocess.DEVNULL,
        cwd=root,
    )
    pairs = set()
    for line in out.splitlines():
        line = line.strip()
        match = re.match(r"^(\S+)\s+(v?\S+)", line)
        if match:
            pairs.add((match.group(1), match.group(2).lstrip("v")))
    return pairs


def main(path):
    with open(path) as f:
        sbom = json.load(f)

    try:
        graph = real_graph()
    except (subprocess.CalledProcessError, FileNotFoundError, OSError) as exc:
        print(f"WARNING: could not compute build graph ({exc}); SBOM left unpruned", file=sys.stderr)
        return 0

    components = sbom.get("components", [])
    kept = [c for c in components if (c.get("name"), c.get("version")) in graph]
    dropped = [c for c in components if (c.get("name"), c.get("version")) not in graph]
    sbom["components"] = kept

    with open(path, "w") as f:
        json.dump(sbom, f, indent=2)
        f.write("\n")

    print(f"pruned {len(dropped)} phantom components, kept {len(kept)}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("usage: prune-sbom.py <sbom.json>", file=sys.stderr)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
