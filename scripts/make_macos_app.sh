#!/usr/bin/env bash
# Build a macOS .app bundle from the release binary, ad-hoc codesign it,
# and zip it for release distribution.
# Usage: bash scripts/make_macos_app.sh [version]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${REPO_ROOT}/target/release/zeroterm"
DIST_DIR="${REPO_ROOT}/dist"
VERSION="${1:-$(git -C "${REPO_ROOT}" describe --tags --always 2>/dev/null || echo 0.2.0)}"
ARCH="$(uname -m)"
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

# Ad-hoc codesign (no identity required); makes the bundle valid locally
if command -v codesign &>/dev/null; then
	echo "Ad-hoc codesigning..."
	codesign --force --deep -s - "${APP_DIR}"
fi

# NOTARIZATION STUB — requires an Apple Developer account; NOT run by default.
# Uncomment and set credentials, then run: bash scripts/make_macos_app.sh
notarize() {
	echo "Notarization requires Apple Developer credentials and is not run automatically."
	echo "Steps (see docs/packaging.md):"
	# shellcheck disable=SC2016 # literal examples, user substitutes real values
	echo '  1. export APPLE_ID=you@example.com APPLE_TEAM_ID=XXXXXXXXXX'
	# shellcheck disable=SC2016 # literal examples, user substitutes real values
	echo '  2. xcrun notarytool store-credentials zeroterm --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID"'
	echo '  3. xcrun notarytool submit "'"${ZIP_FILE}"'" --keychain-profile zeroterm --wait'
	echo '  4. xcrun stapler staple "'"${APP_DIR}"'"'
}

# Zip the .app (ditto preserves permissions/symlinks; zip is the fallback)
echo "Zipping ${APP_DIR}..."
if command -v ditto &>/dev/null; then
	ditto -c -k --keepParent "${APP_DIR}" "${ZIP_FILE}"
else
	zip -rq "${ZIP_FILE}" "${APP_DIR}"
fi

echo "Created ${ZIP_FILE}"
