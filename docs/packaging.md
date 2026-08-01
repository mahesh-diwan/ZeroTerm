# Packaging

ZeroTerm builds as a single binary: `cargo build --release -p zeroterm`.

## Artifacts per platform

| Platform | Artifact                                                                                     | Producer                                                                                                    |
| -------- | -------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Linux    | `zeroterm-<ver>-linux-<arch>.tar.gz`, `ZeroTerm-<arch>.AppImage`, `.deb`, `.rpm`, `.flatpak` | `build-release.sh`, `scripts/make_appimage.sh`, `scripts/build-deb.sh`, `scripts/build-rpm.sh`, flatpak job |
| macOS    | `ZeroTerm.app` zipped as `zeroterm-<ver>-macos-<arch>.zip`                                   | `scripts/make_macos_app.sh`                                                                                 |
| Windows  | `zeroterm-<ver>-windows-x86_64.zip`, optional `.msi`                                         | `scripts/package_windows.bat` + `scripts/windows_installer.wxs`                                             |

## Auto-update

`install.sh upgrade` / `install.sh --update` compares the installed version against the
latest GitHub release and reinstalls when newer. If the installed binary cannot report a
version it falls back to a fresh install of the latest release.

> **Rust-side TODO:** the binary does not implement `--version`/`-V` yet, so upgrade
> detection always falls back to reinstalling. Add `env!("CARGO_PKG_VERSION")` handling
> in `crates/zeroterm/src/main.rs` to enable real version comparison.

## macOS

`scripts/make_macos_app.sh` stages `target/release/zeroterm` into `dist/ZeroTerm.app`
(Info.plist: `com.zeroterm.app`, `NSHighResolutionCapable`), ad-hoc codesigns it
(`codesign --force --deep -s -`), and zips it. Optional icon: `assets/zeroterm.icns`.

Notarization is a documented STUB (`notarize()` in the script) — it requires an Apple
Developer account and is not run automatically. Steps: store credentials with
`xcrun notarytool store-credentials`, submit the zip with `notarytool submit --wait`,
then `xcrun stapler staple ZeroTerm.app`.

> **Caveat:** `install.sh` expects macOS release assets named `zeroterm-<ver>-macos-<arch>.tar.gz`,
> while `build-release.sh` currently emits `darwin` in the name. Align the naming before the
> first tagged release, or the macOS installer download path will 404.

## Windows

ConPTY is handled by `portable-pty` (no code changes needed). Build natively with the MSVC
toolchain (`cargo build --release -p zeroterm`). See `scripts/windows_packaging.md` for
details, the WiX MSI template, and cross-compile caveats.

## Linux

- **AppImage:** `scripts/make_appimage.sh` downloads linuxdeploy + appimagetool (cached in
  `~/.cache/zeroterm-appimage`), stages binary + `.desktop` + icon, outputs
  `dist/ZeroTerm-<arch>.AppImage`. Uses `APPIMAGE_EXTRACT_AND_RUN=1` so FUSE is not needed.
  (`scripts/build-appimage.sh` is the older appimagetool-only variant.)
- **deb/rpm/flatpak:** `scripts/build-deb.sh`, `scripts/build-rpm.sh`,
  `scripts/io.github.zeroterm.ZeroTerm.yml`.

## CI release flow (`.github/workflows/release.yml`)

On `v*` tag push, a matrix builds `-p zeroterm` on ubuntu/macos/windows, runs a **50MB
binary-size gate on Linux** (fails the build if exceeded), then packages per OS (AppImage /
`.app` / zip+MSI). `deb`, `rpm`, `flatpak`, and the systemd unit build in separate jobs.
Artifacts are uploaded as `zeroterm-<target>-packages` (use `gh run download` or attach them
to the release manually, or add a `softprops/action-gh-release` step to publish).
