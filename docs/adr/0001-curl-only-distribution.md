# ADR-0001: Curl-only distribution — install.sh is the single install source

- **Status:** Accepted
- **Date:** 2026-08-11
- **Deciders:** Project owner

## Context and Problem Statement

ZeroTerm must have exactly one way to be installed. The owner's standing
instruction: *"there is only one source of installation and that is through
curl script — don't push anything else."* Early attempts at publishing
package assets (AppImage, `.deb`, `.rpm`) onto GitHub Releases were unstable
(the release pipeline repeatedly failed), and maintaining a matrix of package
formats across platforms consumed effort that did not improve the product.

## Decision Drivers

- **Single source of truth** — one installer means one place to fix, verify,
  and document.
- **Always-latest** — `install.sh` resolves the latest release tag (with a
  fallback to `main`) so installs track the newest version without the user
  chasing releases.
- **CI robustness** — no release-job dependency on packaging toolchains
  (linuxdeploy, dpkg, rpmbuild) that fail in fresh environments.
- **Operational simplicity** — the team prefers fixing the installer over
  debugging three packaging pipelines.

## Considered Options

1. **Publish all package formats** (AppImage + `.deb` + `.rpm` + zips) on
   GitHub Releases, with the installer just downloading the right asset.
   - Rejected: fragile CI, per-format maintenance, and the owner's explicit
     instruction to push nothing but the curl path.
2. **Curl-only installer** with a source fallback.
   - Accepted: `scripts/install.sh` downloads a prebuilt asset when the
     current tag has one (Linux AppImage/raw binary, macOS zip, Windows zip)
     and otherwise builds from source at the tag. Logging goes to stderr;
   the script refuses to run as root; `main` serves as the tag-resolution
   fallback.

## Decision Outcome

`scripts/install.sh` is the single, documented install path:

```sh
curl -fsSL https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/scripts/install.sh | bash
```

No AppImage/`.deb`/`.rpm` assets are published to Releases. The installer is
the only thing the distribution pipeline is responsible for keeping correct.

### Consequences

- **Positive:** one installer to maintain and test; installs always resolve
  the latest tag; release process is just `git tag` + push (no asset
  pipeline); CI stays green.
- **Negative:** users without a Rust toolchain need a prebuilt asset present
  on the tag to avoid a source build; no distro package-manager presence;
  users on exotic platforms rely on the source fallback.
- **Follow-up:** the installer stays the continuous-improvement target; any
  future packaging work must first change this ADR.
