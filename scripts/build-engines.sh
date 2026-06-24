#!/usr/bin/env bash
# Build/fetch the DPI engines bundled with DPI-Bypass.
#
#   Linux:   nfqws (the zapret core) is compiled from source.
#   Windows: GoodbyeDPI + WinDivert are downloaded (no-op on Linux unless
#            --windows is passed).
#
# Output goes to src-tauri/binaries/ so Tauri can ship them as sidecars and so
# install.sh can drop them into /usr/lib/dpi-bypass/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/src-tauri/binaries"
BUILD="$ROOT/build"
mkdir -p "$OUT" "$BUILD"

ZAPRET_REF="${ZAPRET_REF:-v71.2}"
GOODBYEDPI_VER="${GOODBYEDPI_VER:-0.2.3rc3}"

# Rust target triple, for Tauri sidecar naming (binary-<triple>).
TRIPLE="$(rustc -vV 2>/dev/null | awk '/host:/ {print $2}')"
TRIPLE="${TRIPLE:-x86_64-unknown-linux-gnu}"

build_nfqws() {
  echo "==> Building nfqws (zapret $ZAPRET_REF)"
  if ! pkg-config --exists libnetfilter_queue; then
    echo "!! Missing libnetfilter_queue dev headers." >&2
    echo "   Debian/Ubuntu: apt-get install -y libnetfilter-queue-dev libnfnetlink-dev libmnl-dev libcap-dev zlib1g-dev" >&2
    exit 1
  fi
  local src="$BUILD/zapret"
  if [ ! -d "$src/.git" ]; then
    git clone --depth 1 --branch "$ZAPRET_REF" https://github.com/bol-van/zapret "$src"
  fi
  make -C "$src/nfq"
  install -m 0755 "$src/nfq/nfqws" "$OUT/nfqws"
  # Tauri sidecar copy (must be named with the target triple).
  cp "$OUT/nfqws" "$OUT/nfqws-$TRIPLE"
  echo "   -> $OUT/nfqws"
  echo "   -> $OUT/nfqws-$TRIPLE"
  "$OUT/nfqws" --help >/dev/null 2>&1 && echo "   nfqws runs OK" || echo "   (nfqws built; --help returned nonzero, which some versions do)"
}

fetch_goodbyedpi() {
  echo "==> Fetching GoodbyeDPI $GOODBYEDPI_VER (Windows)"
  local url="https://github.com/ValdikSS/GoodbyeDPI/releases/download/${GOODBYEDPI_VER}/GoodbyeDPI-${GOODBYEDPI_VER}.zip"
  local zip="$BUILD/goodbyedpi.zip"
  curl -fsSL "$url" -o "$zip"
  ( cd "$BUILD" && unzip -o "$zip" -d goodbyedpi >/dev/null )
  # Layout varies by release; copy the x86_64 build if present.
  find "$BUILD/goodbyedpi" -iname 'goodbyedpi.exe' -path '*x86_64*' -exec cp {} "$OUT/goodbyedpi.exe" \; || true
  find "$BUILD/goodbyedpi" -iname 'WinDivert*.dll' -path '*x86_64*' -exec cp {} "$OUT/" \; || true
  find "$BUILD/goodbyedpi" -iname 'WinDivert*.sys' -path '*x86_64*' -exec cp {} "$OUT/" \; || true
  echo "   -> $OUT/goodbyedpi.exe (+ WinDivert)"
}

WINDOWS=0
for a in "$@"; do [ "$a" = "--windows" ] && WINDOWS=1; done

case "$(uname -s)" in
  Linux) build_nfqws; [ "$WINDOWS" = 1 ] && fetch_goodbyedpi ;;
  *) fetch_goodbyedpi ;;
esac

echo "Engines ready in $OUT"
