#!/usr/bin/env python3
"""Set the release fields of a Homebrew formula.

The formula carries a download for every platform, so the script takes the
hash of each of them.
"""

import re
import sys

# The line that holds the version of the formula.
VERSION = re.compile(r'^(\s*)version "(.*)"$')

# The line that holds a download.
URL = re.compile(r'^(\s*)url "(.*)"$')

# The line that holds the hash of a download.
SHA256 = re.compile(r'^(\s*)sha256 "(.*)"$')


def update(text, version, hashes):
    """Rewrites the version, the downloads and the hashes of the formula."""
    out = []
    pending = None

    for line in text.split("\n"):
        match = VERSION.match(line)
        if match:
            out.append('%sversion "%s"' % (match.group(1), version))
            continue

        match = URL.match(line)
        if match:
            indent, url = match.group(1), match.group(2)
            asset = url.rsplit("/", 1)[-1]
            if asset not in hashes:
                sys.exit("the formula has no download named %s" % asset)
            pending = asset
            out.append(
                '%surl "https://github.com/jarpex/q/releases/download/v%s/%s"'
                % (indent, version, asset)
            )
            continue

        match = SHA256.match(line)
        if match:
            if pending is None:
                sys.exit("the formula has a hash without a download")
            out.append('%ssha256 "%s"' % (match.group(1), hashes[pending]))
            pending = None
            continue

        out.append(line)

    if pending is not None:
        sys.exit("the download %s has no hash" % pending)

    return "\n".join(out)


def main():
    if len(sys.argv) < 3:
        sys.exit("usage: update-formula.py FORMULA VERSION ASSET=SHA256 [ASSET=SHA256 ...]")
    path, version = sys.argv[1:3]
    hashes = {}
    for pair in sys.argv[3:]:
        asset, _, digest = pair.partition("=")
        if not digest:
            sys.exit("the hash of %s is missing" % asset)
        hashes[asset] = digest

    with open(path) as handle:
        text = handle.read()
    text = update(text, version, hashes)
    with open(path, "w") as handle:
        handle.write(text)


if __name__ == "__main__":
    main()
