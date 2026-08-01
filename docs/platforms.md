# Platform Notes

ZeroTerm renders with wgpu 23. The renderer creates its instance with
`wgpu::Backends::PRIMARY` (`crates/zeroterm-render/src/renderer.rs:437`), which selects
**all native desktop backends — Vulkan, Metal, and DX12 — at runtime**. It is not forced
to a single backend, so Metal (macOS) and DX12 (Windows) work out of the box with no
per-OS build flags. One build config serves every OS.

## macOS

- **GPU:** Metal via wgpu automatically (`PRIMARY` includes Metal). No code changes.
- **Build + .app:** `cargo build --release -p zeroterm`, then
  `bash scripts/make_macos_app.sh [version]`. Produces `dist/ZeroTerm.app` and
  `dist/zeroterm-<ver>-macos-<arch>.zip` with `arch` = `x86_64` or `arm64` (matches the
  asset names `install.sh` downloads; the script maps `aarch64` → `arm64` defensively).
- **Codesign:** the script uses `CODESIGN_IDENTITY` when set (signed with
  `--options runtime --timestamp` — hardened runtime, required for notarization) and
  falls back to ad-hoc `-` otherwise. Ad-hoc is fine for local use; a shared ad-hoc app
  triggers Gatekeeper ("unidentified developer").
- **Notarization + stapling** (makes Gatekeeper trust downloaded apps):
  - Requires a **paid Apple Developer account** ($99/yr), a Developer ID Application
    certificate, and 2FA. There is no free tier.
  - `--password` must be an **app-specific password** (create at
    https://appleid.apple.com → Sign-In & Security → App-Specific Passwords), never the
    account password, because of 2FA.
  - Env-driven: `APPLE_ID`, `APPLE_TEAM_ID`, and `APPLE_APP_SPECIFIC_PASSWORD`
    (or `APPLE_PASSWORD`). The script skips notarization with a warning when any is
    unset, so CI without secrets still passes.
  - Exact commands (what `make_macos_app.sh` runs when credentials are present):

    ```sh
    export CODESIGN_IDENTITY="Developer ID Application: Name (TEAMID)"
    export APPLE_ID=you@example.com APPLE_TEAM_ID=XXXXXXXXXX
    export APPLE_APP_SPECIFIC_PASSWORD=xxxx-xxxx-xxxx-xxxx
    bash scripts/make_macos_app.sh
    # inside: xcrun notarytool submit dist/zeroterm-<ver>-macos-<arch>.zip \
    #     --apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" \
    #     --team-id "$APPLE_TEAM_ID" --wait
    # then:   xcrun stapler staple dist/ZeroTerm.app
    # then re-zip so the released archive contains the stapled ticket
    ```

  - `notarytool` is the current tool (Xcode 13+); the deprecated `altool` is not used.
  - Alternative to env vars: `xcrun notarytool store-credentials zeroterm --apple-id … --team-id …`
    then submit with `--keychain-profile zeroterm` instead of `--apple-id/--password/--team-id`.
  - **Gatekeeper:** a notarized, stapled app opens normally. Without it, users must
    right-click → Open, or run `xattr -dr com.apple.quarantine /Applications/ZeroTerm.app`.
  - Verify a submitted archive with `xcrun notarytool log <submission-id>` or
    `spctl --assess -vv dist/ZeroTerm.app`.

## Windows

- **ConPTY:** `portable-pty` (used by the core crate) wraps the ConPTY API with a
  winpty fallback — no code changes needed. Full notes in
  `scripts/windows_packaging.md`.
- **GPU:** DX12 via wgpu automatically (`PRIMARY` includes DX12). Vulkan is the fallback
  if a DX12 adapter is unavailable.
- **Build:** native MSVC toolchain (`rustup` default on Windows),
  `cargo build --release -p zeroterm` → `target\release\zeroterm.exe`. Cross-compiling
  from Linux/macOS is not recommended for MSVC (needs the MSVC runtime libs); use a
  native Windows host or a `windows-latest` CI runner.
- **Packaging:** `scripts/package_windows.bat` (run in `cmd.exe` or Git Bash) produces
  `dist\zeroterm-<ver>-windows-x86_64.zip` (exe + README via PowerShell
  `Compress-Archive`), and — if WiX `candle`/`light` are on PATH — an MSI.
- **MSI via WiX** (`scripts/windows_installer.wxs`, WiX Toolset v3):
  - Before the first build, replace `UpgradeCode="PUT-YOUR-GUID-HERE"` with a fixed GUID
    (generate once with `guidgen`/`uuidgen`, never change it) and set
    `Product/@Version` to a numeric value (e.g. `0.2.0`, no leading `v`).
  - Install WiX: `winget install --id WiXToolset.WiXToolset.3`.
  - Build by hand: `candle scripts\windows_installer.wxs && light -o dist\zeroterm.msi dist\wix\*.wixobj`.
  - The wxs structure: single `Product` (MajorUpgrade, embedded cab) with a
    `ComponentGroup AppFiles` installing `zeroterm.exe` + `README.md` to
    `ProgramFilesFolder\ZeroTerm`.
  - The exe lands in `C:\Program Files\ZeroTerm`.
- **Code signing:** `signtool sign /fd SHA256 /a target\release\zeroterm.exe` (sign the
  zip contents and the MSI too). Requires a code-signing certificate; EV certs avoid
  SmartScreen "Unknown publisher" warnings. Unsigned binaries still run but SmartScreen
  warns on first launch.

## Linux

- **GPU:** Vulkan via wgpu automatically. Requires a Vulkan driver (mesa `vulkan-*`,
  NVIDIA driver, etc.); without one the adapter request fails.
- **Packaging** already exists; see `docs/packaging.md` for the full table:
  - AppImage: `scripts/make_appimage.sh`
  - `.deb`: `scripts/build-deb.sh`
  - `.rpm`: `scripts/build-rpm.sh`
  - Flatpak: `scripts/io.github.zeroterm.ZeroTerm.yml`
  - tarball used by `install.sh`: `zeroterm-<ver>-linux-<arch>.tar.gz`
