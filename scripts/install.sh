#!/usr/bin/env bash
# ZeroTerm installer
#
# Resolves the latest tagged release of ZeroTerm, then installs it:
#   1. downloads the prebuilt package for your OS/architecture when the
#      release has one, or
#   2. falls back to building from source at that tag.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash
#   ./install.sh upgrade                  # (re)install the latest release
#   ZEROTERM_VERSION=v0.3.0 ./install.sh  # pin a specific release tag
#
# Overrides:
#   ZEROTERM_VERSION      release tag to install (default: latest published)
#   ZEROTERM_INSTALL_DIR  install destination (default: ~/.local/bin for
#                         binaries, ~/Applications for the macOS app)
# The script uses bash features (`[[ ]]`, `pipefail`, `local`); detect a POSIX
# sh early so `curl ... | sh` on dash gives a clear message instead of a
# cryptic failure.
if [ -z "${BASH_VERSION:-}" ]; then
    echo "This installer requires bash (bash-specific features are used)." >&2
    echo "Run it with: curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash" >&2
    exit 1
fi

set -euo pipefail

REPO="mahesh-diwan/ZeroTerm"
REPO_URL="https://github.com/${REPO}"
API_URL="https://api.github.com/repos/${REPO}"

# All logging goes to stderr so that stdout carries only the functions' actual
# return values (e.g. `version="$(resolve_version)"` must not pick up log lines).
log()  { printf '\033[32m[ZeroTerm]\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[33m[ZeroTerm]\033[0m warning: %s\n' "$*" >&2; }
err()  { printf '\033[31m[ZeroTerm]\033[0m error: %s\n' "$*" >&2; exit 1; }

need_cmd() { command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"; }

# --- version resolution ------------------------------------------------------

resolve_version() {
    if [[ -n "${ZEROTERM_VERSION:-}" ]]; then
        log "installing pinned version ${ZEROTERM_VERSION}"
        printf '%s\n' "${ZEROTERM_VERSION}"
        return
    fi
    local json v=""
    # Prefer the latest *published* release; its tag is what we install.
    json="$(curl -fsSL --retry 3 --retry-delay 2 --max-time 20 "${API_URL}/releases/latest" 2>/dev/null || true)"
    if [[ -n "$json" ]]; then
        v="$(printf '%s' "$json" \
            | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 \
            | sed 's/.*"\([^"]*\)"$/\1/')"
    fi
    if [[ -z "$v" ]]; then
        # No published release yet: use the newest tag, else main.
        json="$(curl -fsSL --retry 3 --retry-delay 2 --max-time 20 "${API_URL}/tags?per_page=100" 2>/dev/null || true)"
        if [[ -n "$json" ]]; then
            v="$(printf '%s' "$json" \
                | grep -o '"name"[[:space:]]*:[[:space:]]*"v[^"]*"' | head -1 \
                | sed 's/.*"v/v/; s/"$//')"
        fi
    fi
    if [[ -z "$v" ]]; then
        v="main"
        warn "no tagged release found; installing from the 'main' branch"
    fi
    log "resolved latest version: ${v}"
    printf '%s\n' "$v"
}

# --- platform detection ------------------------------------------------------

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "${os}" in
        Linux)
            case "${arch}" in
                x86_64|amd64)  printf 'linux-x86_64\n' ;;
                aarch64|arm64) printf 'linux-aarch64\n' ;;
                *) err "unsupported CPU architecture: ${arch}" ;;
            esac
            ;;
        Darwin)
            case "${arch}" in
                arm64|aarch64) printf 'macos-arm64\n' ;;
                x86_64|amd64)  printf 'macos-x86_64\n' ;;
                *) err "unsupported CPU architecture: ${arch}" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            warn "Windows builds are experimental and untested; installing the zip"
            printf 'windows-x86_64\n'
            ;;
        *)
            err "unsupported operating system: ${os} (Linux and macOS are supported)"
            ;;
    esac
}

# --- release asset discovery -------------------------------------------------

fetch_release_json() {
    local version="$1" json
    json="$(curl -fsSL --retry 3 --retry-delay 2 --max-time 20 "${API_URL}/releases/tags/${version}" 2>/dev/null || true)"
    if [[ -z "$json" ]]; then
        json="$(curl -fsSL --retry 3 --retry-delay 2 --max-time 20 "${API_URL}/releases/latest" 2>/dev/null || true)"
    fi
    printf '%s' "$json"
}

asset_urls() {
    local json="$1"
    printf '%s' "$json" \
        | grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' \
        | sed 's/.*"\([^"]*\)"$/\1/'
}

# Echo the download URL of the first asset matching our platform, or nothing.
pick_asset() {
    local platform="$1" u
    shift
    case "${platform}" in
        linux-x86_64)
            for u in "$@"; do
                case "$u" in *ZeroTerm-x86_64.AppImage) printf '%s\n' "$u"; return ;; esac
            done
            for u in "$@"; do
                case "$u" in */zeroterm) printf '%s\n' "$u"; return ;; esac
            done
            ;;
        linux-aarch64)
            for u in "$@"; do
                case "$u" in *ZeroTerm-aarch64.AppImage) printf '%s\n' "$u"; return ;; esac
            done
            for u in "$@"; do
                case "$u" in */zeroterm) printf '%s\n' "$u"; return ;; esac
            done
            ;;
        macos-*)
            local want="macos-${platform#macos-}"
            for u in "$@"; do
                case "$u" in
                    *"${want}.zip") printf '%s\n' "$u"; return ;;
                esac
            done
            ;;
        windows-x86_64)
            for u in "$@"; do
                case "$u" in
                    *windows-x86_64.zip) printf '%s\n' "$u"; return ;;
                esac
            done
            ;;
    esac
    return 1
}

# --- download / install ------------------------------------------------------

download_file() {
    local url="$1" dest="$2"
    log "downloading ${url##*/} ..."
    if command -v curl &>/dev/null; then
        curl -fL --retry 3 --retry-delay 2 --progress-bar -o "${dest}" "${url}"
    elif command -v wget &>/dev/null; then
        wget -q -O "${dest}" "${url}"
    else
        err "curl or wget is required to download the release"
    fi
}

install_linux() {
    local url="$1" version="$2"
    local dir="${ZEROTERM_INSTALL_DIR:-${HOME}/.local/bin}"
    mkdir -p "${dir}"
    if [[ "$url" == *.AppImage ]]; then
        local appimage="${dir}/ZeroTerm-${version}.AppImage"
        # Drop stale AppImages from previous versions so upgrades don't orphan them.
        rm -f "${dir}"/ZeroTerm-*.AppImage
        download_file "$url" "$appimage"
        chmod +x "$appimage"
        ln -sf "$appimage" "${dir}/zeroterm"
        log "installed ${dir}/zeroterm -> ${appimage}"
    else
        download_file "$url" "${dir}/zeroterm"
        chmod +x "${dir}/zeroterm"
        log "installed ${dir}/zeroterm"
    fi
    hint_path "${dir}"
    printf '%s\n' "${dir}/zeroterm"
}

install_macos() {
    local url="$1"
    local apps="${ZEROTERM_INSTALL_DIR:-${HOME}/Applications}"
    local tmp app
    need_cmd unzip
    mkdir -p "$apps"
    tmp="$(mktemp -d)"
    download_file "$url" "${tmp}/zeroterm.zip"
    log "unpacking ZeroTerm.app ..."
    (cd "$tmp" && unzip -q zeroterm.zip)
    app="$(find "$tmp" -maxdepth 2 -name '*.app' -type d | head -1)"
    [[ -n "$app" ]] || err "the downloaded archive did not contain a .app bundle"
    rm -rf "${apps}/$(basename "$app")"
    cp -R "$app" "${apps}/"
    chmod +x "${apps}/$(basename "$app")/Contents/MacOS/zeroterm"
    rm -rf "$tmp"
    log "installed ${apps}/$(basename "$app")"
    printf '%s\n' "${apps}/$(basename "$app")"
}

install_windows() {
    local url="$1"
    local dir="${ZEROTERM_INSTALL_DIR:-${LOCALAPPDATA:-$HOME/AppData/Local}/Programs/ZeroTerm}"
    local tmp
    need_cmd unzip
    mkdir -p "$dir"
    tmp="$(mktemp -d)"
    download_file "$url" "${tmp}/zeroterm.zip"
    (cd "$tmp" && unzip -q zeroterm.zip)
    cp -R "$tmp"/. "$dir"/
    rm -rf "$tmp"
    log "installed ${dir}/zeroterm.exe (add this directory to your PATH)"
    printf '%s\n' "${dir}/zeroterm.exe"
}

build_from_source() {
    local version="$1"
    local src dir bin tarball
    need_cmd cargo || err "no prebuilt binary is published for this platform and Rust (cargo) is not installed. Install Rust from https://rustup.rs and re-run."
    need_cmd tar
    log "no prebuilt binary for this platform; building from source at ${version} ..."
    src="$(mktemp -d)"
    if [[ "${version}" == "main" ]]; then
        tarball="${REPO_URL}/archive/refs/heads/main.tar.gz"
    else
        tarball="${REPO_URL}/archive/refs/tags/${version}.tar.gz"
    fi
    download_file "$tarball" "${src}/src.tar.gz"
    tar -xzf "${src}/src.tar.gz" -C "$src" --strip-components=1
    log "building the release binary (this may take a few minutes) ..."
    if ! (cd "$src" && cargo build --release -p zeroterm); then
        err "source build failed. ZeroTerm links against Lua 5.4 (via mlua) and the wgpu system libraries; on Debian/Ubuntu install: sudo apt install liblua5.4-dev libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl-dev libssl-dev pkg-config (macOS: brew install lua cmake pkg-config)"
    fi
    bin="${src}/target/release/zeroterm"
    [[ -x "$bin" ]] || err "the source build did not produce target/release/zeroterm"
    dir="${ZEROTERM_INSTALL_DIR:-${HOME}/.local/bin}"
    mkdir -p "$dir"
    cp "$bin" "${dir}/zeroterm"
    chmod +x "${dir}/zeroterm"
    rm -rf "$src"
    log "installed ${dir}/zeroterm (built from source at ${version})"
    hint_path "${dir}"
    printf '%s\n' "${dir}/zeroterm"
}

verify_install() {
    local bin="$1"
    local reported
    if [[ -x "$bin" ]]; then
        reported="$("$bin" --version 2>/dev/null || true)"
        if [[ -n "$reported" ]]; then
            log "verified: ${reported}"
        else
            warn "installed, but could not run '${bin} --version' to verify"
        fi
    fi
}

hint_path() {
    local dir="$1"
    case ":${PATH}:" in
        *":${dir}:"*) ;; # already on PATH
        *)
            warn "${dir} is not on your PATH; add it with:"
            warn "  export PATH=\"${dir}:\$PATH\"  # add to ~/.bashrc or ~/.zshrc"
            ;;
    esac
}

# --- main --------------------------------------------------------------------

main() {
    local mode="${1:-install}" version platform json urls asset bin
    case "$mode" in
        install|upgrade) ;;
        -h|--help|help)
            cat <<'HELP'
ZeroTerm installer

Resolves the latest tagged release of ZeroTerm, then installs it:
  1. downloads the prebuilt package for your OS/architecture when the
     release has one, or
  2. falls back to building from source at that tag.

Usage:
  curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash
  bash scripts/install.sh upgrade          # (re)install the latest release
  ZEROTERM_VERSION=v0.3.0 bash scripts/install.sh   # pin a specific tag

Overrides:
  ZEROTERM_VERSION      release tag to install (default: latest published)
  ZEROTERM_INSTALL_DIR  install destination (default: ~/.local/bin for
                        binaries, ~/Applications for the macOS app)
HELP
            exit 0
            ;;
        *) err "unknown argument: ${mode} (expected 'install' or 'upgrade')" ;;
    esac

    if ! command -v curl &>/dev/null && ! command -v wget &>/dev/null; then
        err "curl or wget is required to download releases"
    fi
    version="$(resolve_version)"
    platform="$(detect_platform)"

    if [[ "${version}" != "main" ]]; then
        json="$(fetch_release_json "$version")"
        if [[ -n "$json" ]]; then
            asset="$(pick_asset "$platform" $(asset_urls "$json") || true)"
            if [[ -n "$asset" ]]; then
                case "$platform" in
                    linux-*)        bin="$(install_linux "$asset" "$version")" ;;
                    macos-*)        bin="$(install_macos "$asset")" ;;
                    windows-x86_64) bin="$(install_windows "$asset")" ;;
                esac
                verify_install "$bin"
                return 0
            fi
            warn "no release asset matches ${platform} in ${version}; building from source"
        fi
    fi

    bin="$(build_from_source "$version")"
    verify_install "$bin"
}

# BASH_SOURCE[0] is unset when the script is piped to bash (curl ... | bash);
# run main in that case or when executed as a file, but not when `source`d.
if [[ -z "${BASH_SOURCE[0]:-}" || "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
