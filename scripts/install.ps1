# ZeroTerm Windows Installer
Write-Host "ZeroTerm Windows Installer" -ForegroundColor Cyan
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "Rust not installed. Install from: https://rustup.rs" -ForegroundColor Red
    exit 1
}
Write-Host "Building ZeroTerm from source..." -ForegroundColor Yellow
cargo build --release
Write-Host "Binary at: target/release/zeroterm.exe" -ForegroundColor Green
