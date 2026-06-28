#!/usr/bin/env bash

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "This script must run on an Apple Silicon Mac." >&2
  exit 1
fi

for tool in brew dylibbundler curl otool file; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Missing required build tool: $tool" >&2
    exit 1
  fi
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT_DIR/src-tauri/binaries"
SHARE_DIR="$ROOT_DIR/src-tauri/share"
TESSERACT_DIR="$ROOT_DIR/src-tauri/tesseract"

clean_generated_dir() {
  local dir="$1"
  mkdir -p "$dir"
  find "$dir" -mindepth 1 ! -name ".gitkeep" -exec rm -rf {} +
}

clean_generated_dir "$BIN_DIR"
clean_generated_dir "$SHARE_DIR"
clean_generated_dir "$TESSERACT_DIR"

install_binary() {
  local name="$1"
  local destination="$2"
  local source
  source="$(command -v "$name")"
  cp -L "$source" "$destination"
  chmod 755 "$destination"
}

install_binary qpdf "$BIN_DIR/qpdf"
install_binary pdftoppm "$BIN_DIR/pdftoppm"
install_binary pdftotext "$BIN_DIR/pdftotext"
install_binary tesseract "$TESSERACT_DIR/tesseract"

cd "$ROOT_DIR"

# Homebrew executables reference versioned dylibs outside the app. Copy the
# complete dependency closure and rewrite load commands to portable paths.
dylibbundler \
  -od \
  -b \
  -x "$BIN_DIR/qpdf" \
  -x "$BIN_DIR/pdftoppm" \
  -x "$BIN_DIR/pdftotext" \
  -d "$BIN_DIR/lib" \
  -p "@executable_path/lib/" \
  -s "$(brew --prefix qpdf)/lib" \
  -s "$(brew --prefix poppler)/lib"

dylibbundler \
  -od \
  -b \
  -x "$TESSERACT_DIR/tesseract" \
  -d "$TESSERACT_DIR/lib" \
  -p "@executable_path/lib/" \
  -s "$(brew --prefix tesseract)/lib"

mkdir -p "$SHARE_DIR/poppler" "$SHARE_DIR/fontconfig"
rsync -aL "$(brew --prefix poppler)/share/poppler/" "$SHARE_DIR/poppler/"
rsync -aL "$(brew --prefix)/etc/fonts/" "$SHARE_DIR/fontconfig/"

mkdir -p "$TESSERACT_DIR/tessdata"
rsync -aL "$(brew --prefix tesseract)/share/tessdata/" "$TESSERACT_DIR/tessdata/"

for lang in eng tur deu fra osd; do
  curl \
    --fail \
    --location \
    --retry 3 \
    --silent \
    --show-error \
    "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/main/${lang}.traineddata" \
    --output "$TESSERACT_DIR/tessdata/${lang}.traineddata"
done

check_portability() {
  local target="$1"
  local unresolved
  unresolved="$(otool -L "$target" | grep -E '/opt/homebrew|/usr/local' || true)"
  if [[ -n "$unresolved" ]]; then
    echo "Non-portable library references remain in $target:" >&2
    echo "$unresolved" >&2
    exit 1
  fi
}

while IFS= read -r -d '' candidate; do
  if file "$candidate" | grep -q "Mach-O"; then
    check_portability "$candidate"
  fi
done < <(find "$BIN_DIR" "$TESSERACT_DIR" -type f -print0)

"$BIN_DIR/qpdf" --version
"$BIN_DIR/pdftoppm" -v
"$BIN_DIR/pdftotext" -v
TESSDATA_PREFIX="$TESSERACT_DIR/tessdata" "$TESSERACT_DIR/tesseract" --list-langs

echo "Prepared portable Apple Silicon PDF engines."
