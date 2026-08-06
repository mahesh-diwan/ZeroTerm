#!/usr/bin/env bash
set -euo pipefail

APP=zeroterm
VERSION="$(git describe --tags --always 2>/dev/null || echo "0.3.0")"
VERSION="${VERSION#v}"
VERSION="${VERSION//-/_}"

if ! command -v cargo &>/dev/null; then
	echo "Rust not installed. Install from: https://rustup.rs"
	exit 1
fi

if ! command -v rpmbuild &>/dev/null; then
	echo "rpmbuild not found. Install with: sudo apt install rpm (or: sudo dnf install rpm-build)"
	echo "RPM build tree skipped."
	exit 1
fi

echo "Building release binary..."
cargo build --release

RPM_DIR="${HOME}/rpmbuild"
STAGE="${RPM_DIR}/SOURCES/zeroterm-${VERSION}"
rm -rf "${RPM_DIR}"
mkdir -p "${RPM_DIR}"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
mkdir -p "${STAGE}/usr/bin"
mkdir -p "${STAGE}/usr/share/applications"
mkdir -p "${STAGE}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${STAGE}/usr/share/metainfo"

cp target/release/"${APP}" "${STAGE}/usr/bin/"

cat >"${STAGE}/usr/share/applications/${APP}.desktop" <<EOF
[Desktop Entry]
Name=ZeroTerm
Comment=GPU-accelerated terminal emulator
Exec=${APP}
Icon=${APP}
Terminal=false
Type=Application
Categories=TerminalEmulator;
StartupNotify=true
EOF

cat >"${STAGE}/usr/share/icons/hicolor/scalable/apps/${APP}.svg" <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">
  <rect width="48" height="48" rx="8" fill="#1a1a2e"/>
  <text x="24" y="32" font-family="monospace" font-size="28" fill="#00ff88" text-anchor="middle" font-weight="bold">&gt;_</text>
</svg>
SVG

cat >"${STAGE}/usr/share/metainfo/io.github.zeroterm.ZeroTerm.metainfo.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop">
  <id>io.github.zeroterm.ZeroTerm</id>
  <name>ZeroTerm</name>
  <summary>GPU-accelerated terminal emulator</summary>
  <description>
    <p>ZeroTerm is a modern, GPU-accelerated terminal emulator built with Rust and wgpu.</p>
  </description>
  <categories>
    <category>TerminalEmulator</category>
  </categories>
  <url type="homepage">https://github.com/mahesh-diwan/ZeroTerm</url>
  <project_license>MIT</project_license>
</component>
XML

cat >"${RPM_DIR}/SPECS/${APP}.spec" <<EOF
Name: ${APP}
Version: ${VERSION}
Release: 1
Summary: GPU-accelerated terminal emulator
License: MIT
URL: https://github.com/mahesh-diwan/ZeroTerm
Requires: glibc, libX11
BuildArch: x86_64

%description
ZeroTerm is a modern, GPU-accelerated terminal emulator built with Rust and
wgpu. It provides fast rendering, multi-tab support, and Lua configuration.

%install
install -Dm755 %{_sourcedir}/zeroterm-%{version}/usr/bin/zeroterm %{buildroot}%{_bindir}/zeroterm
install -Dm644 %{_sourcedir}/zeroterm-%{version}/usr/share/applications/zeroterm.desktop %{buildroot}%{_datadir}/applications/zeroterm.desktop
install -Dm644 %{_sourcedir}/zeroterm-%{version}/usr/share/icons/hicolor/scalable/apps/zeroterm.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/zeroterm.svg
install -Dm644 %{_sourcedir}/zeroterm-%{version}/usr/share/metainfo/io.github.zeroterm.ZeroTerm.metainfo.xml %{buildroot}%{_datadir}/metainfo/io.github.zeroterm.ZeroTerm.metainfo.xml

%files
%{_bindir}/zeroterm
%{_datadir}/applications/zeroterm.desktop
%{_datadir}/icons/hicolor/scalable/apps/zeroterm.svg
%{_datadir}/metainfo/io.github.zeroterm.ZeroTerm.metainfo.xml

%changelog
* $(date '+%a %b %d %Y') ZeroTerm Developers <dev@zeroterm.dev>
- Initial RPM packaging
EOF

rpmbuild -bb --define "_topdir ${RPM_DIR}" "${RPM_DIR}/SPECS/${APP}.spec"

cp "${RPM_DIR}"/RPMS/*/*.rpm .
echo "RPM package created: ${APP}-${VERSION}-1.x86_64.rpm"
