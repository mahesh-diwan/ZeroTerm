#!/usr/bin/env bash
# ZeroTerm Installer
set -euo pipefail

# Detect platform
OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
Darwin)
	if command -v brew &>/dev/null; then
		echo "Install via Homebrew not yet published. Building from source..."
		echo "Run: cargo install --git https://github.com/zeroterm/zeroterm"
	fi
	;;
Linux)
	echo "Installing ZeroTerm for Linux..."
	if command -v apt &>/dev/null; then
		echo "APT install not yet published."
	fi
	;;
*)
	echo "Unsupported OS: $OS"
	exit 1
	;;
esac

# Fallback: build from source
if ! command -v cargo &>/dev/null; then
	echo "Rust not installed. Install from: https://rustup.rs"
	exit 1
fi

echo "Building ZeroTerm from source (this may take a few minutes)..."
cargo build --release
echo "Binary at: target/release/zeroterm"
