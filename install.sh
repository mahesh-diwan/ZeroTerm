#!/usr/bin/env bash
# ZeroTerm Universal Installer
# Installs ZeroTerm to ~/.local/bin without root privileges
# Usage: curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/install.sh | bash

set -euo pipefail

# Configuration
REPO="mahesh-diwan/ZeroTerm"
BINARY_NAME="zeroterm"
INSTALL_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
APP_BUNDLE_DIR="${HOME}/Applications"
CONFIG_DIR="${HOME}/.config/zeroterm"
GITHUB_API="https://api.github.com/repos/${REPO}"
GITHUB_RELEASES="https://github.com/${REPO}/releases"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $*" >&2; }
log_success() { echo -e "${GREEN}[OK]${NC} $*" >&2; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*" >&2; }
log_error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# Detect OS and architecture
detect_platform() {
	local os arch
	case "$(uname -s)" in
	Linux*) os="linux" ;;
	Darwin*) os="darwin" ;;
	CYGWIN* | MINGW* | MSYS*) os="windows" ;;
	*)
		log_error "Unsupported OS: $(uname -s)"
		exit 1
		;;
	esac

	case "$(uname -m)" in
	x86_64 | amd64) arch="x86_64" ;;
	aarch64 | arm64) arch="aarch64" ;;
	*)
		log_error "Unsupported architecture: $(uname -m)"
		exit 1
		;;
	esac

	echo "${os}-${arch}"
}

# Get latest release version
get_latest_version() {
	local version
	version=$(curl -fsSL --retry 3 --retry-delay 2 --retry-connrefused "${GITHUB_API}/releases/latest" 2>/dev/null | grep -o '"tag_name": "[^"]*' | cut -d'"' -f4)
	if [[ -z "$version" ]]; then
		log_warn "Could not fetch latest release, trying fallback..."
		version=$(curl -fsSL --retry 3 --retry-delay 2 --retry-connrefused "${GITHUB_API}/releases" 2>/dev/null | grep -o '"tag_name": "[^"]*' | head -1 | cut -d'"' -f4)
	fi
	echo "$version"
}

# Download with pacman-style progress animation (stderr must be a TTY)
show_progress() {
	local dest="$1" url="$2"
	local pid i
	local -a spinner=('ᗧ·······' '·ᗧ······' '··ᗧ·····' '···ᗧ····' '····ᗧ···' '·····ᗧ··' '······ᗧ·' '·······ᗧ')

	if [[ ! -t 2 ]]; then
		curl -fL --retry 3 --retry-delay 2 --retry-connrefused -fsSL -o "$dest" "$url"
		return
	fi

	curl -fL --retry 3 --retry-delay 2 --retry-connrefused -fsSL -o "$dest" "$url" &
	pid=$!
	i=0
	while kill -0 "$pid" 2>/dev/null; do
		printf '\r  %s  ' "${spinner[$((i % 8))]}" >&2
		i=$((i + 1))
		sleep 0.12
	done
	printf '\r\033[K' >&2
	wait "$pid"
}

# Best-effort SHA-256 verification; skipped when no checksum is published
verify_checksum() {
	local file="$1" url="$2"
	local sum_cmd expected actual sum_file="${file}.sha256"

	if ! command -v sha256sum &>/dev/null && ! command -v shasum &>/dev/null; then
		log_warn "No sha256 tool found, skipping checksum verification"
		return 0
	fi

	if ! curl -fL --retry 3 --retry-delay 2 --retry-connrefused -fsSL -o "$sum_file" "${url}.sha256" 2>/dev/null; then
		log_warn "No checksum published for this release, skipping verification"
		return 0
	fi

	if command -v sha256sum &>/dev/null; then
		sum_cmd="sha256sum"
	else
		sum_cmd="shasum -a 256"
	fi
	actual=$($sum_cmd "$file" | awk '{print $1}')
	expected=$(awk '{print $1}' "$sum_file")

	if [[ "$actual" != "$expected" ]]; then
		log_error "Checksum mismatch! Expected ${expected}, got ${actual}"
		rm -f "$file"
		return 1
	fi
	log_success "Checksum verified"
}

# Download and install binary
install_binary() {
	local platform="$1"
	local version="$2"
	local asset_name
	local download_url
	local temp_dir

	# Determine asset name based on platform
	case "$platform" in
	linux-x86_64) asset_name="${BINARY_NAME}-${version}-linux-x86_64.tar.gz" ;;
	linux-aarch64) asset_name="${BINARY_NAME}-${version}-linux-aarch64.tar.gz" ;;
	darwin-x86_64) asset_name="${BINARY_NAME}-${version}-macos-x86_64.zip" ;;
	darwin-aarch64) asset_name="${BINARY_NAME}-${version}-macos-arm64.zip" ;;
	windows-x86_64) asset_name="${BINARY_NAME}-${version}-windows-x86_64.zip" ;;
	*)
		log_error "No prebuilt binary for platform: $platform"
		exit 1
		;;
	esac

	download_url="${GITHUB_RELEASES}/download/${version}/${asset_name}"

	log_info "Downloading ${asset_name}..."
	temp_dir=$(mktemp -d)
	trap 'rm -rf "${temp_dir:-}"' EXIT

	if ! show_progress "${temp_dir}/${asset_name}" "${download_url}"; then
		log_error "Failed to download ${download_url}"
		log_info "Building from source instead..."
		build_from_source
		return
	fi

	if ! verify_checksum "${temp_dir}/${asset_name}" "${download_url}"; then
		log_warn "Checksum verification failed, building from source instead..."
		build_from_source
		return
	fi

	log_info "Extracting..."
	case "$asset_name" in
	*.tar.gz)
		command -v tar &>/dev/null || {
			log_error "tar not found, cannot extract"
			exit 1
		}
		tar -xzf "${temp_dir}/${asset_name}" -C "${temp_dir}"
		;;
	*.zip)
		command -v unzip &>/dev/null || {
			log_error "unzip not found, cannot extract"
			exit 1
		}
		unzip -q "${temp_dir}/${asset_name}" -d "${temp_dir}"
		;;
	esac

	# Find the binary
	local binary_path
	binary_path=$(find "${temp_dir}" -name "${BINARY_NAME}" -type f 2>/dev/null | head -1)
	if [[ -z "$binary_path" ]]; then
		# Try with .exe extension for Windows
		binary_path=$(find "${temp_dir}" -name "${BINARY_NAME}.exe" -type f 2>/dev/null | head -1)
	fi

	if [[ -z "$binary_path" ]]; then
		log_error "Binary not found in archive"
		exit 1
	fi

	# Install
	mkdir -p "${INSTALL_DIR}"
	cp "${binary_path}" "${INSTALL_DIR}/${BINARY_NAME}"
	chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

	log_success "Installed to ${INSTALL_DIR}/${BINARY_NAME}"
}

# Build from source as fallback
build_from_source() {
	log_info "Building from source..."

	# Check for Rust
	if ! command -v cargo &>/dev/null; then
		log_error "Rust not found. Please install from https://rustup.rs/"
		exit 1
	fi

	local temp_dir
	temp_dir=$(mktemp -d)
	trap 'rm -rf "${temp_dir:-}"' EXIT

	log_info "Cloning repository..."
	git clone --depth 1 "https://github.com/${REPO}.git" "${temp_dir}/ZeroTerm"

	log_info "Building release binary (this may take a few minutes)..."
	cd "${temp_dir}/ZeroTerm"
	cargo build --release -p zeroterm

	mkdir -p "${INSTALL_DIR}"
	cp "target/release/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
	chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

	log_success "Built and installed from source"
}

# Add to PATH if needed
setup_path() {
	local shell_rc=""
	case "${SHELL##*/}" in
	bash) shell_rc="${HOME}/.bashrc" ;;
	zsh) shell_rc="${HOME}/.zshrc" ;;
	fish) shell_rc="${HOME}/.config/fish/config.fish" ;;
	*) shell_rc="${HOME}/.profile" ;;
	esac

	if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
		log_info "Adding ${INSTALL_DIR} to PATH in ${shell_rc}..."
		mkdir -p "$(dirname "${shell_rc}")"

		case "${SHELL##*/}" in
		fish)
			echo "set -gx PATH ${INSTALL_DIR} \$PATH" >>"${shell_rc}"
			;;
		*)
			echo "export PATH=\"${INSTALL_DIR}:\$PATH\"" >>"${shell_rc}"
			;;
		esac
		log_warn "Restart your shell or run: source ${shell_rc}"
	else
		log_success "${INSTALL_DIR} already in PATH"
	fi
}

# Verify installation
verify_install() {
	if "${INSTALL_DIR}/${BINARY_NAME}" --version 2>/dev/null || "${INSTALL_DIR}/${BINARY_NAME}" -V 2>/dev/null; then
		log_success "Installation verified!"
	else
		log_warn "Could not verify binary (may need --version flag implementation)"
	fi
}

# Install Linux .desktop entry
install_desktop_entry() {
	mkdir -p "${DESKTOP_DIR}"
	cat >"${DESKTOP_DIR}/zeroterm.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=ZeroTerm
Comment=Zero latency, zero bloat, zero config terminal emulator
Exec=${INSTALL_DIR}/zeroterm
Icon=utilities-terminal
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;console;shell;zeroterm;
EOF
	log_success "Desktop entry installed"
}

# Create macOS .app bundle wrapper
install_app_bundle() {
	local app_dir="${APP_BUNDLE_DIR}/ZeroTerm.app"
	mkdir -p "${app_dir}/Contents/MacOS"
	ln -sf "${INSTALL_DIR}/zeroterm" "${app_dir}/Contents/MacOS/zeroterm"
	cat >"${app_dir}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>zeroterm</string>
	<key>CFBundleIdentifier</key>
	<string>com.zeroterm.terminal</string>
	<key>CFBundleName</key>
	<string>ZeroTerm</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
</dict>
</plist>
EOF
	log_success "App bundle created: ${app_dir}"
}

# Detect the currently installed version (best-effort; zeroterm --version is a Rust-side TODO)
get_installed_version() {
	local bin out ver
	bin="$(command -v "${BINARY_NAME}" 2>/dev/null)" || return 1
	out=$("${bin}" --version 2>/dev/null || "${bin}" -V 2>/dev/null) || return 1
	ver=$(printf '%s\n' "${out}" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
	[[ -n "${ver}" ]] || return 1
	printf '%s\n' "${ver}"
}

# Returns 0 when $1 is a newer version than $2 (dotted numeric compare, portable)
version_gt() {
	local a b
	a="${1#v}"
	b="${2#v}"
	[[ "$a" == "$b" ]] && return 1
	IFS='.' read -ra a_parts <<<"$a"
	IFS='.' read -ra b_parts <<<"$b"
	local i
	for i in "${!a_parts[@]}"; do
		local ai bi
		ai="${a_parts[$i]}"
		bi="${b_parts[$i]:-0}"
		if ((ai > bi)); then
			return 0
		elif ((ai < bi)); then
			return 1
		fi
	done
	((${#a_parts[@]} > ${#b_parts[@]}))
}

# Auto-update: compare installed vs latest release, reinstall when newer
upgrade_zeroterm() {
	local installed latest platform
	installed=""
	if installed=$(get_installed_version); then
		log_info "Installed version: ${installed}"
	else
		log_warn "Could not detect installed version (zeroterm --version is a Rust-side TODO)"
		log_info "Proceeding with a fresh install of the latest release"
	fi

	latest=$(get_latest_version || true)
	if [[ -z "$latest" ]]; then
		log_error "Could not determine the latest release version"
		exit 1
	fi
	log_info "Latest version: ${latest}"

	if [[ -n "$installed" ]] && ! version_gt "$latest" "$installed"; then
		log_success "ZeroTerm is already up to date (${installed})"
		return 0
	fi

	platform=$(detect_platform)
	install_binary "$platform" "$latest"
	setup_path
	verify_install

	case "$platform" in
	linux-*) install_desktop_entry ;;
	darwin-*) install_app_bundle ;;
	esac

	log_success "ZeroTerm updated to ${latest}"
}

# Remove the PATH line this installer added
remove_path_line() {
	local shell_rc=""
	case "${SHELL##*/}" in
	bash) shell_rc="${HOME}/.bashrc" ;;
	zsh) shell_rc="${HOME}/.zshrc" ;;
	fish) shell_rc="${HOME}/.config/fish/config.fish" ;;
	*) shell_rc="${HOME}/.profile" ;;
	esac

	[[ -f "$shell_rc" ]] || return 0

	if [[ "${SHELL##*/}" == fish ]]; then
		sed -i.bak "\|set -gx PATH ${INSTALL_DIR} \\\$PATH|d" "$shell_rc"
	else
		sed -i.bak "\|export PATH=\"${INSTALL_DIR}:\\\$PATH\"|d" "$shell_rc"
	fi
	rm -f "${shell_rc}.bak"
	log_success "Removed PATH line from ${shell_rc}"
}

# Uninstall
uninstall() {
	log_info "Uninstalling ZeroTerm..."
	rm -f "${INSTALL_DIR}/zeroterm"
	rm -f "${DESKTOP_DIR}/zeroterm.desktop"
	rm -rf "${APP_BUNDLE_DIR}/ZeroTerm.app"
	remove_path_line
	if [ "$PURGE" = true ]; then
		rm -rf "${CONFIG_DIR}"
		log_info "Config removed: ${CONFIG_DIR}"
	fi
	log_success "ZeroTerm uninstalled"
}

# Main
main() {
	local UNINSTALL=false
	local PURGE=false
	local UPGRADE=false
	for arg in "$@"; do
		case "$arg" in
		--uninstall | -u) UNINSTALL=true ;;
		--purge) PURGE=true ;;
		--update) UPGRADE=true ;;
		upgrade) UPGRADE=true ;;
		--verbose) set -x ;;
		esac
	done

	if [ "$UNINSTALL" = true ]; then
		uninstall
		exit 0
	fi

	if [ "$UPGRADE" = true ]; then
		upgrade_zeroterm
		exit 0
	fi

	echo "╔══════════════════════════════════════════╗"
	echo "║        ZeroTerm Universal Installer      ║"
	echo "║     Zero latency, zero bloat, zero config  ║"
	echo "╚══════════════════════════════════════════╝"
	echo

	local platform version
	platform=$(detect_platform)
	log_info "Detected platform: ${platform}"

	version=$(get_latest_version)
	if [[ -z "$version" ]]; then
		log_warn "No releases found, building from source..."
		build_from_source
	else
		log_info "Latest version: ${version}"
		install_binary "${platform}" "${version}"
	fi

	setup_path
	verify_install

	case "$platform" in
	linux-*) install_desktop_entry ;;
	darwin-*) install_app_bundle ;;
	esac

	echo
	log_success "ZeroTerm installed successfully!"
	echo
	echo "Run: ${BINARY_NAME}"
	echo "Config: ~/.config/zeroterm/config.toml"
	echo
}

main "$@"
