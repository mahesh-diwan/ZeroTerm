#!/usr/bin/env bash
# Build release binaries for all platforms
# Run on CI or locally to create release artifacts

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${REPO_ROOT}/target/release"
DIST_DIR="${REPO_ROOT}/dist"
VERSION="${1:-$(git describe --tags --always --dirty)}"

echo "Building ZeroTerm ${VERSION} for all platforms..."

mkdir -p "${DIST_DIR}"

# Build for current platform
echo "Building for host platform..."
cargo build --release -p zeroterm

# Package for current platform
HOST_PLATFORM=""
case "$(uname -s)" in
Linux*) HOST_PLATFORM="linux-$(uname -m)" ;;
Darwin*) HOST_PLATFORM="darwin-$(uname -m)" ;;
CYGWIN* | MINGW* | MSYS*) HOST_PLATFORM="windows-$(uname -m)" ;;
esac

package_binary() {
	local platform="$1"
	local binary_name="$2"
	local archive_name="zeroterm-${VERSION}-${platform}"

	echo "Packaging ${archive_name}..."

	local temp_dir
	temp_dir=$(mktemp -d)
	cp "${BUILD_DIR}/${binary_name}" "${temp_dir}/"

	# Include README and LICENSE
	cp "${REPO_ROOT}/README.md" "${temp_dir}/"
	if [[ -f "${REPO_ROOT}/LICENSE" ]]; then
		cp "${REPO_ROOT}/LICENSE" "${temp_dir}/"
	fi

	case "$platform" in
	windows-*)
		cd "${temp_dir}"
		zip -r "${DIST_DIR}/${archive_name}.zip" .
		;;
	*)
		tar -czf "${DIST_DIR}/${archive_name}.tar.gz" -C "${temp_dir}" .
		;;
	esac

	rm -rf "${temp_dir}"
	echo "Created ${DIST_DIR}/${archive_name}.*"
}

case "$HOST_PLATFORM" in
linux-x86_64 | linux-aarch64)
	package_binary "${HOST_PLATFORM}" "zeroterm"
	;;
darwin-x86_64 | darwin-aarch64)
	package_binary "${HOST_PLATFORM}" "zeroterm"
	;;
windows-x86_64)
	package_binary "${HOST_PLATFORM}" "zeroterm.exe"
	;;
*)
	echo "Unknown platform: ${HOST_PLATFORM}"
	exit 1
	;;
esac

# Best-effort platform extras; skipped when the packager or its tools are missing
case "$HOST_PLATFORM" in
linux-*)
	if [[ -f "${REPO_ROOT}/scripts/make_appimage.sh" ]]; then
		echo "==> Building AppImage..."
		"${REPO_ROOT}/scripts/make_appimage.sh" || echo "Warning: AppImage build skipped (script exited non-zero)"
	fi
	;;
darwin-*)
	if [[ -f "${REPO_ROOT}/scripts/make_macos_app.sh" ]]; then
		echo "==> Building macOS .app bundle..."
		"${REPO_ROOT}/scripts/make_macos_app.sh" || echo "Warning: .app build skipped (script exited non-zero)"
	fi
	;;
esac

echo ""
echo "Build complete! Artifacts in ${DIST_DIR}:"
ls -la "${DIST_DIR}/"
