#!/usr/bin/env bash
# Checks that the tag of a release goes with the version of the tool.
#
# The usage is: check-release-tag.sh TAG
# The script prints the version of the tag when everything is in order, and
# stops with a message when the tag and the version do not match.
#
# The paths of the files can be changed for the tests with the environment
# variables Q_MANIFEST and Q_MAN_PAGE.

set -euo pipefail

TAG="${1:-}"

if [ -z "$TAG" ]; then
  echo "the tag of the release is missing" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="${Q_MANIFEST:-$ROOT/Cargo.toml}"
MAN_PAGE="${Q_MAN_PAGE:-$ROOT/man/q.1}"

# The tag carries the version with a v in front, and may carry a suffix of
# the pre-release, such as v1.0.0-rc.1.
if ! printf '%s' "$TAG" | grep -qE '^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
  echo "the tag $TAG is not vMAJOR.MINOR.PATCH" >&2
  exit 1
fi

VERSION=${TAG#v}

# The manifest carries the version of the tool.
MANIFEST_VERSION=$(grep -E '^version[[:space:]]*=' "$MANIFEST" | head -n 1 | sed 's/.*"\(.*\)".*/\1/')
if [ "$MANIFEST_VERSION" != "$VERSION" ]; then
  echo "the tag $TAG does not go with the version $MANIFEST_VERSION of $MANIFEST" >&2
  exit 1
fi

# The man page carries the same version in its header. The releases before the
# man page came have none, so a missing file is not a fault.
if [ -f "$MAN_PAGE" ]; then
  if ! grep -qE "^\.TH Q 1 .* \"q $VERSION\" " "$MAN_PAGE"; then
    echo "the man page $MAN_PAGE does not carry the version $VERSION" >&2
    exit 1
  fi
else
  echo "the release carries no man page" >&2
fi

echo "$VERSION"
