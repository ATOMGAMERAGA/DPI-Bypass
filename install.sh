#!/usr/bin/env bash
# DPI-Bypass — Linux installer.
#
# Detects the distro/package manager, installs runtime dependencies, places the
# GUI + privileged helper + nfqws engine, registers the desktop entry, polkit
# rule and (disabled) systemd service.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ATOMGAMERAGA/DPI-Bypass/main/install.sh | sudo bash
#   sudo ./install.sh --from-build      # install from a local checkout you built
set -euo pipefail

PREFIX="/usr"
LIBDIR="$PREFIX/lib/dpi-bypass"
BINDIR="$PREFIX/bin"
ICONDIR="$PREFIX/share/icons/hicolor"
DESKTOPDIR="$PREFIX/share/applications"
POLKITDIR="/etc/polkit-1/rules.d"

log()  { printf '\033[36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31mxx\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "Please run as root (sudo)."

# ---- Detect package manager from /etc/os-release ----
detect_pm() {
  local ids=""
  [ -r /etc/os-release ] && ids="$(. /etc/os-release; echo "$ID $ID_LIKE")"
  for tok in $ids; do
    case "$tok" in
      ubuntu|debian|mint|pop|zorin) echo apt; return;;
      fedora|rhel|centos)           echo dnf; return;;
      arch|manjaro|endeavouros|cachyos) echo pacman; return;;
      opensuse*|suse)               echo zypper; return;;
      void)                         echo xbps; return;;
      alpine)                       echo apk; return;;
    esac
  done
  # Fall back to whatever binary exists.
  for pm in apt-get dnf pacman zypper xbps-install apk; do
    command -v "$pm" >/dev/null 2>&1 && { echo "$pm" | sed 's/apt-get/apt/;s/xbps-install/xbps/'; return; }
  done
  echo unknown
}

install_deps() {
  local pm="$1"
  log "Installing dependencies via: $pm"
  case "$pm" in
    apt)    apt-get update -y && apt-get install -y nftables libnetfilter-queue1 libnfnetlink0 libmnl0 libwebkit2gtk-4.1-0 libayatana-appindicator3-1 policykit-1 curl ;;
    dnf)    dnf install -y nftables libnetfilter_queue webkit2gtk4.1 libayatana-appindicator-gtk3 polkit curl ;;
    pacman) pacman -Sy --noconfirm nftables libnetfilter_queue webkit2gtk-4.1 libayatana-appindicator polkit curl ;;
    zypper) zypper install -y nftables libnetfilter_queue1 webkit2gtk3 libayatana-appindicator3-1 polkit curl ;;
    xbps)   xbps-install -y nftables libnetfilter_queue webkit2gtk libayatana-appindicator polkit curl ;;
    apk)    apk add nftables libnetfilter_queue webkit2gtk libayatana-appindicator polkit curl ;;
    *) warn "Unknown package manager. Install manually: nftables, libnetfilter_queue, webkit2gtk-4.1, libayatana-appindicator, polkit, curl" ;;
  esac
}

# ---- Locate artefacts: a local build, an extracted bundle, or fetch a release ----
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd || echo "$PWD")"
GUI_BIN=""; HELPER_BIN=""; NFQWS_BIN=""

detect_artifacts() {
  GUI_BIN=""; HELPER_BIN=""; NFQWS_BIN=""
  local d
  for d in "$SCRIPT_DIR/target/release" "$SCRIPT_DIR/target/debug" "$SCRIPT_DIR"; do
    [ -z "$GUI_BIN"    ] && [ -x "$d/dpi-bypass" ]        && GUI_BIN="$d/dpi-bypass"
    [ -z "$HELPER_BIN" ] && [ -x "$d/dpi-bypass-helper" ] && HELPER_BIN="$d/dpi-bypass-helper"
  done
  [ -x "$SCRIPT_DIR/src-tauri/binaries/nfqws" ] && NFQWS_BIN="$SCRIPT_DIR/src-tauri/binaries/nfqws"
}

# Download + extract the latest published release tarball; point SCRIPT_DIR at it.
bootstrap_from_release() {
  command -v curl >/dev/null 2>&1 || die "curl gerekli."
  command -v tar  >/dev/null 2>&1 || die "tar gerekli."
  log "En son sürüm indiriliyor…"
  local api="https://api.github.com/repos/ATOMGAMERAGA/DPI-Bypass/releases/latest"
  local url
  url="$(curl -fsSL "$api" \
    | grep -oE '"browser_download_url"[^,]*linux-x86_64\.tar\.gz"' \
    | sed -E 's/.*"(https[^"]+)"/\1/' | head -1)"
  [ -n "$url" ] || die "Sürümde Linux arşivi bulunamadı."
  local tmp; tmp="$(mktemp -d)"
  curl -fsSL "$url" -o "$tmp/bundle.tar.gz" || die "İndirme başarısız: $url"
  tar -xzf "$tmp/bundle.tar.gz" -C "$tmp" || die "Arşiv açılamadı."
  local dir; dir="$(find "$tmp" -maxdepth 1 -type d -name 'dpi-bypass-*' | head -1)"
  [ -n "$dir" ] || die "Arşiv içeriği beklenmedik."
  SCRIPT_DIR="$dir"
}

detect_artifacts

install_files() {
  log "Installing files into $LIBDIR"
  install -d "$LIBDIR" "$BINDIR" "$DESKTOPDIR" "$POLKITDIR" "$ICONDIR/scalable/apps"

  [ -n "$GUI_BIN" ]    || die "GUI binary not found — build first: cargo build --release"
  [ -n "$HELPER_BIN" ] || die "Helper binary not found — build first: cargo build --release"
  [ -n "$NFQWS_BIN" ]  || warn "nfqws engine not found — run scripts/build-engines.sh (bypass won't work until present)"

  install -m 0755 "$GUI_BIN"    "$LIBDIR/dpi-bypass"
  install -m 0755 "$HELPER_BIN" "$LIBDIR/dpi-bypass-helper"
  [ -n "$NFQWS_BIN" ] && install -m 0755 "$NFQWS_BIN" "$LIBDIR/nfqws"
  ln -sf "$LIBDIR/dpi-bypass" "$BINDIR/dpi-bypass"

  install -m 0644 "$SCRIPT_DIR/packaging/dpi-bypass.desktop" "$DESKTOPDIR/dpi-bypass.desktop"
  install -m 0644 "$SCRIPT_DIR/logo.svg" "$ICONDIR/scalable/apps/dpi-bypass.svg"
  install -m 0644 "$SCRIPT_DIR/packaging/polkit/dpi-bypass.rules" "$POLKITDIR/49-dpi-bypass.rules"
  install -m 0644 "$SCRIPT_DIR/systemd/dpi-bypass.service" "/etc/systemd/system/dpi-bypass.service"

  systemctl daemon-reload || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$ICONDIR" || true
  log "systemd service installed but left DISABLED (enable 'Always On' from the app)."
}

FROM_BUILD=0
for a in "$@"; do [ "$a" = "--from-build" ] && FROM_BUILD=1; done

# Launched via `curl | sudo bash` with no local binaries → fetch the latest
# published release and install from it.
if { [ -z "$GUI_BIN" ] || [ -z "$HELPER_BIN" ]; } && [ "$FROM_BUILD" -eq 0 ]; then
  bootstrap_from_release
  detect_artifacts
fi

PM="$(detect_pm)"
install_deps "$PM"
install_files
log "Bitti. Başlatmak için:  dpi-bypass   (veya uygulama menüsünden)"
