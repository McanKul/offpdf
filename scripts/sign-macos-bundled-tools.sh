#!/usr/bin/env bash

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS." >&2
  exit 1
fi

SIGNING_IDENTITY="${1:-${APPLE_SIGNING_IDENTITY:-}}"
if [[ -z "$SIGNING_IDENTITY" ]]; then
  echo "A Developer ID signing identity is required." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT_DIR/src-tauri/binaries"
TESSERACT_DIR="$ROOT_DIR/src-tauri/tesseract"

xattr -cr "$BIN_DIR" "$TESSERACT_DIR"

sign_macho_files() {
  local root="$1"

  while IFS= read -r -d '' candidate; do
    if file "$candidate" | grep -q "Mach-O"; then
      codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$SIGNING_IDENTITY" \
        "$candidate"
      codesign --verify --strict --verbose=2 "$candidate"
    fi
  done < <(find "$root" -type f -print0)
}

# Sign nested libraries before their executables, then Tauri signs the outer
# application bundle after copying these resources.
sign_macho_files "$BIN_DIR/lib"
sign_macho_files "$TESSERACT_DIR/lib"

for executable in \
  "$BIN_DIR/qpdf" \
  "$BIN_DIR/pdftoppm" \
  "$BIN_DIR/pdftotext" \
  "$TESSERACT_DIR/tesseract"; do
  codesign \
    --force \
    --options runtime \
    --timestamp \
    --sign "$SIGNING_IDENTITY" \
    "$executable"
  codesign --verify --strict --verbose=2 "$executable"
done

echo "Signed bundled Apple Silicon PDF engines."
