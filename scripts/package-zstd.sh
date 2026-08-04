#!/usr/bin/env bash
# zstd release tarball. Produces dist/zeroterm-v<VERSION>-<ARCH>.tar.zst
# (or .tar.gz when the zstd CLI is unavailable) with the release binary
# plus quick install notes. Deterministic-ish: sorted entries, fixed
# owner/group and mtime (SOURCE_DATE_EPOCH if set, else epoch 0).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP=zeroterm
BINARY="${REPO_ROOT}/target/release/${APP}"
DIST_DIR="${REPO_ROOT}/dist"
ARCH="$(uname -m)"
CARGO="${CARGO:-${HOME}/.cargo/bin/cargo}"

REBUILD=0
for arg in "$@"; do
	case "${arg}" in
	--rebuild) REBUILD=1 ;;
	*)
		echo "Usage: $0 [--rebuild]" >&2
		exit 1
		;;
	esac
done

# Version from the crate manifest (crates/zeroterm/Cargo.toml), fallback to git describe
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "${REPO_ROOT}/crates/${APP}/Cargo.toml" | head -1)"
if [[ -z "${VERSION}" ]]; then
	VERSION="$(git describe --tags --always 2>/dev/null || echo "0.1.0")"
fi
VERSION="${VERSION#v}"

if [[ ! -x "${BINARY}" || "${REBUILD}" -eq 1 ]]; then
	if ! command -v "${CARGO}" &>/dev/null; then
		echo "cargo not found (${CARGO}). Install from: https://rustup.rs" >&2
		exit 1
	fi
	echo "Building release binary..."
	"${CARGO}" build --release -p ${APP}
fi

if ! command -v zstd &>/dev/null; then
	echo "zstd not found; falling back to gzip (.tar.gz)" >&2
	COMPRESS=zlib
	EXT=gz
else
	COMPRESS=zstd
	EXT=zst
fi

OUT="${DIST_DIR}/${APP}-v${VERSION}-${ARCH}.tar.${EXT}"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT
mkdir -p "${STAGE}/${APP}"

cp "${BINARY}" "${STAGE}/${APP}/${APP}"
cat >"${STAGE}/${APP}/README.txt" <<EOF
ZeroTerm ${VERSION} (${ARCH})

GPU-accelerated terminal emulator built with Rust and wgpu.

Install:
  tar -xzf ${APP}-v${VERSION}-${ARCH}.tar.${EXT}   # or .tar.zst
  sudo install -m755 ${APP}/${APP} /usr/local/bin/

Run: zeroterm
EOF

mkdir -p "${DIST_DIR}"
# Deterministic-ish: sorted entries, fixed uid/gid, epoch mtime
TAR_ARGS=(--sort=name --owner=0 --group=0 --mtime="@${SOURCE_DATE_EPOCH:-0}")
if [[ "${COMPRESS}" == zstd ]]; then
	tar -cf - "${TAR_ARGS[@]}" -C "${STAGE}" "${APP}" | zstd -q -f -19 -T0 -o "${OUT}"
else
	tar -czf "${OUT}" "${TAR_ARGS[@]}" -C "${STAGE}" "${APP}"
fi

SIZE="$(du -h "${OUT}" | cut -f1)"
echo "Created ${OUT} (${SIZE})"
