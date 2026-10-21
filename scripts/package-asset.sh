#!/usr/bin/env bash
# Packs the binary and the files of the release into an archive.
#
# The usage is: package-asset.sh TARGET ASSET_NAME ARCHIVE
# The archive is a tarball, or a zip for Windows.

set -euo pipefail

TARGET="$1"
ASSET_NAME="$2"
ARCHIVE="$3"

# The binary of Windows carries the extension of the system.
if [ "$ARCHIVE" = "zip" ]; then
  BINARY="q.exe"
else
  BINARY="q"
fi

mkdir -p dist
cp "target/$TARGET/release/$BINARY" "dist/$BINARY"
cp README.md LICENSE dist/ || true
cp -r man dist/ || true

if [ "$ARCHIVE" = "zip" ]; then
  # The zip of Windows, made with the powershell of the runner.
  if ! powershell -NoProfile -Command \
      "Compress-Archive -Path 'dist/*' -DestinationPath '$ASSET_NAME.zip' -Force"; then
    # The fallback on the tar of the system, that reads the format of the
    # name of the archive.
    "/c/Windows/System32/tar.exe" -a -cf "$ASSET_NAME.zip" -C dist .
  fi
else
  tar -czvf "$ASSET_NAME.tar.gz" -C dist .
fi
