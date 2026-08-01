#!/usr/bin/env bash
# Build a macOS .app bundle from the release binary, codesign it (ad-hoc unless
# CODESIGN_IDENTITY is set), optionally notarize+staple, and zip for release.
# Usage: bash scripts/make_macos_app.sh [version]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${REPO_ROOT}/target/release/zeroterm"
DIST_DIR="${REPO_ROOT}/dist"
VERSION="${1:-$(git -C "${REPO_ROOT}" describe --tags --always 2>/dev/null || echo 0.2.0)}"
ARCH="$(uname -m)"
# install.sh expects macos-<arch>.zip with x86_64/arm64 (never aarch64)
[[ "${ARCH}" == "aarch64" ]] && ARCH="arm64"
APP_DIR="${DIST_DIR}/ZeroTerm.app"
ZIP_FILE="${DIST_DIR}/zeroterm-${VERSION}-macos-${ARCH}.zip"

if [[ ! -x "${BINARY}" ]]; then
	echo "Binary not found at ${BINARY}. Build first: cargo build --release -p zeroterm" >&2
	exit 1
fi

echo "Building ZeroTerm.app (${VERSION}, ${ARCH})..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS" "${APP_DIR}/Contents/Resources"

cp "${BINARY}" "${APP_DIR}/Contents/MacOS/zeroterm"

cat >"${APP_DIR}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.zeroterm.app</string>
	<key>CFBundleName</key>
	<string>ZeroTerm</string>
	<key>CFBundleDisplayName</key>
	<string>ZeroTerm</string>
	<key>CFBundleExecutable</key>
	<string>zeroterm</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION#v}</string>
	<key>CFBundleVersion</key>
	<string>${VERSION#v}</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>LSMinimumSystemVersion</key>
	<string>10.15</string>
</dict>
</plist>
EOF

# Optional app icon: drop an icns at assets/zeroterm.icns to include it
if [[ -f "${REPO_ROOT}/assets/zeroterm.icns" ]]; then
	cp "${REPO_ROOT}/assets/zeroterm.icns" "${APP_DIR}/Contents/Resources/"
	/usr/libexec/PlistBuddy -c 'Add :CFBundleIconFile string zeroterm' "${APP_DIR}/Contents/Info.plist"
fi

# Codesign. For distribution/notarization set CODESIGN_IDENTITY to a Developer ID
# (signed with hardened runtime + secure timestamp); otherwise ad-hoc (-), which is
# only valid locally and triggers Gatekeeper when shared.
if command -v codesign &>/dev/null; then
	if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
		echo "Codesigning with identity '${CODESIGN_IDENTITY}' (hardened runtime)..."
		codesign --force --deep --options runtime --timestamp -s "${CODESIGN_IDENTITY}" "${APP_DIR}"
	else
		echo "Ad-hoc codesigning (no CODESIGN_IDENTITY set)..."
		codesign --force --deep -s - "${APP_DIR}"
	fi
fi

# Notarize + staple so Gatekeeper trusts the app. Requires an Apple Developer
# account (paid) and a Developer ID signature (CODESIGN_IDENTITY). Env-driven:
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_SPECIFIC_PASSWORD (or APPLE_PASSWORD)
# The password is an app-specific password (2FA-safe), not the account password.
# Skipped with a warning when any credential is absent, so CI without secrets
# still passes. Uses xcrun notarytool (Xcode 13+), not the deprecated altool.
notarize() {
	if [[ -z "${APPLE_ID:-}" || -z "${APPLE_TEAM_ID:-}" ]]; then
		echo "Skipping notarization: set APPLE_ID and APPLE_TEAM_ID to enable." >&2
		return 0
	fi
	local password="${APPLE_APP_SPECIFIC_PASSWORD:-${APPLE_PASSWORD:-}}"
	if [[ -z "${password}" ]]; then
		echo "Skipping notarization: set APPLE_APP_SPECIFIC_PASSWORD (or APPLE_PASSWORD)." >&2
		return 0
	fi
	if ! command -v xcrun &>/dev/null; then
		echo "Skipping notarization: xcrun not found (run on macOS)." >&2
		return 0
	fi
	if [[ -z "${CODESIGN_IDENTITY:-}" ]]; then
		echo "Warning: notarizing an ad-hoc-signed app will be rejected by Apple." >&2
	fi
	echo "Submitting ${ZIP_FILE} to Apple for notarization..."
	xcrun notarytool submit "${ZIP_FILE}" \
		--apple-id "${APPLE_ID}" \
		--password "${password}" \
		--team-id "${APPLE_TEAM_ID}" \
		--wait
	echo "Stapling notarization ticket to ${APP_DIR}..."
	xcrun stapler staple "${APP_DIR}"
	echo "Re-zipping so the released archive contains the stapled ticket..."
	if command -v ditto &>/dev/null; then
		ditto -c -k --keepParent "${APP_DIR}" "${ZIP_FILE}"
	else
		zip -rq "${ZIP_FILE}" "${APP_DIR}"
	fi
}

# Zip the .app (ditto preserves permissions/symlinks; zip is the fallback)
echo "Zipping ${APP_DIR}..."
if command -v ditto &>/dev/null; then
	ditto -c -k --keepParent "${APP_DIR}" "${ZIP_FILE}"
else
	zip -rq "${ZIP_FILE}" "${APP_DIR}"
fi

echo "Created ${ZIP_FILE}"

notarize
