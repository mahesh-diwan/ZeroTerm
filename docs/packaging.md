# Packaging

ZeroTerm builds as a single binary: `cargo build --release -p zeroterm`.

## Artifacts per platform

| Platform | Artifact                                                                                     | Producer                                                                                                           |
| -------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Linux    | `zeroterm-<ver>-linux-<arch>.tar.gz`, `ZeroTerm-<arch>.AppImage`, `.deb`, `.rpm`, `.flatpak` | `scripts/package-zstd.sh`, `scripts/make_appimage.sh`, `scripts/build-deb.sh`, `scripts/build-rpm.sh`, flatpak job |
| macOS    | `ZeroTerm.app` zipped as `zeroterm-<ver>-macos-<arch>.zip`                                   | `scripts/make_macos_app.sh`                                                                                        |
| Windows  | `zeroterm-<ver>-windows-x86_64.zip`, optional `.msi`                                         | `scripts/package_windows.bat` + `scripts/windows_installer.wxs`                                                    |

## Auto-update

`install.sh upgrade` / `install.sh --update` compares the installed version against the
latest GitHub release and reinstalls when newer. The binary supports version reporting
and a self-upgrade entry point:

- `zeroterm --version` / `zeroterm -V` prints the build version from
  `env!("CARGO_PKG_VERSION")` and exits without opening a window.
- `zeroterm upgrade` runs `bash ./install.sh upgrade` when run from the repo checkout,
  or prints the `curl -fsSL <install-url> | bash -s -- upgrade` one-liner otherwise.
  Both routes reuse `install.sh`, which does the version comparison and reinstall.

## macOS

`scripts/make_macos_app.sh` stages `target/release/zeroterm` into `dist/ZeroTerm.app`
(Info.plist: `com.zeroterm.app`, `NSHighResolutionCapable`), ad-hoc codesigns it
(`codesign --force --deep -s -`), and zips it. Optional icon: `assets/zeroterm.icns`.

Notarization is a documented STUB (`notarize()` in the script) — it requires an Apple
Developer account and is not run automatically. Steps: store credentials with
`xcrun notarytool store-credentials`, submit the zip with `notarytool submit --wait`,
then `xcrun stapler staple ZeroTerm.app`.

> **Caveat:** `install.sh` expects macOS release assets named `zeroterm-<ver>-macos-<arch>.zip`
> with `x86_64` / `arm64` arches, matching `scripts/make_macos_app.sh`. Keep the two naming
> schemes in sync when changing release packaging, or the macOS installer download path 404s.

## Windows

ConPTY is handled by `portable-pty` (no code changes needed). Build natively with the MSVC
toolchain (`cargo build --release -p zeroterm`). See `scripts/windows_packaging.md` for
details, the WiX MSI template, and cross-compile caveats.

## Linux

- **AppImage:** `scripts/make_appimage.sh` downloads linuxdeploy + appimagetool (cached in
  `~/.cache/zeroterm-appimage`), stages binary + `.desktop` + icon, outputs
  `dist/ZeroTerm-<arch>.AppImage`. Uses `APPIMAGE_EXTRACT_AND_RUN=1` so FUSE is not needed.
  (`scripts/build-appimage.sh` is the older appimagetool-only variant.)
- **zstd tarball:** `scripts/package-zstd.sh [--rebuild]` builds the release binary if missing
  (or with `--rebuild`) and emits `dist/zeroterm-v<VERSION>-<ARCH>.tar.zst` (binary + README,
  sorted entries + fixed mtime for reproducible builds). Falls back to `.tar.gz` if `zstd` is
  not on PATH. Version read from `crates/zeroterm/Cargo.toml`.
- **deb/rpm/flatpak:** `scripts/build-deb.sh`, `scripts/build-rpm.sh`,
  `scripts/io.github.zeroterm.ZeroTerm.yml`.

## CI release flow (`.github/workflows/release.yml`)

On `v*` tag push, a matrix builds `-p zeroterm` on ubuntu/macos/windows, runs a **50MB
binary-size gate on Linux** (fails the build if exceeded), then packages per OS (AppImage /
`.app` / zip+MSI). `deb`, `rpm`, `flatpak`, and the systemd unit build in separate jobs.
Artifacts are uploaded as `zeroterm-<target>-packages` (use `gh run download` or attach them
to the release manually, or add a `softprops/action-gh-release` step to publish).
