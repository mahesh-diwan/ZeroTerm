<# ZeroTerm Universal Installer for Windows (PowerShell)
# Installs ZeroTerm to $env:USERPROFILE\.local\bin without admin rights
# Usage: irm https://raw.githubusercontent.com/mahesh-diwan/ZeroTerm/main/install.ps1 | iex
#>

[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$Purge
)

# Configuration
$Repo = "mahesh-diwan/ZeroTerm"
$BinaryName = "zeroterm.exe"
$InstallDir = Join-Path $env:USERPROFILE ".local\bin"
$GitHubApi = "https://api.github.com/repos/$Repo"
$GitHubReleases = "https://github.com/$Repo/releases"
$StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\ZeroTerm"
$ConfigDir = Join-Path $env:USERPROFILE ".config\zeroterm"

function Write-Log {
    param([string]$Message, [string]$Level = "INFO")
    $color = switch ($Level) {
        "ERROR" { "Red" }
        "WARN"  { "Yellow" }
        "OK"    { "Green" }
        default { "Cyan" }
    }
    Write-Host "[$Level] $Message" -ForegroundColor $color
}

function Get-Platform {
    $os = if ($IsLinux) { "linux" } elseif ($IsMacOS) { "darwin" } else { "windows" }
    $arch = switch ($env:PROCESSOR_ARCHITECTURE) {
        "AMD64" { "x86_64" }
        "ARM64" { "aarch64" }
        default { throw "Unsupported architecture: $env:PROCESSOR_ARCHITECTURE" }
    }
    return "$os-$arch"
}

function Get-LatestVersion {
    try {
        $response = Invoke-RestMethod -Uri "$GitHubApi/releases/latest" -ErrorAction Stop
        return $response.tag_name
    } catch {
        Write-Log "Could not fetch latest release, trying fallback..." "WARN"
        try {
            $releases = Invoke-RestMethod -Uri "$GitHubApi/releases" -ErrorAction Stop
            return $releases[0].tag_name
        } catch {
            return $null
        }
    }
}

function Install-Binary {
    param([string]$Platform, [string]$Version)
    
    $assetName = switch ($Platform) {
        "linux-x86_64"     { "zeroterm-$Version-linux-x86_64.tar.gz" }
        "linux-aarch64"    { "zeroterm-$Version-linux-aarch64.tar.gz" }
        "darwin-x86_64"    { "zeroterm-$Version-macos-x86_64.tar.gz" }
        "darwin-aarch64"   { "zeroterm-$Version-macos-aarch64.tar.gz" }
        "windows-x86_64"   { "zeroterm-$Version-windows-x86_64.zip" }
        default { throw "No prebuilt binary for platform: $Platform" }
    }

    $downloadUrl = "$GitHubReleases/download/$Version/$assetName"
    Write-Log "Downloading $assetName..."
    
    $tempDir = [System.IO.Path]::GetTempPath()
    $archivePath = Join-Path $tempDir $assetName
    
    try {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath -ErrorAction Stop
    } catch {
        Write-Log "Failed to download $downloadUrl" "ERROR"
        Write-Log "Building from source instead..." "WARN"
        Build-FromSource
        return
    }

    Write-Log "Extracting..."
    $extractDir = Join-Path $tempDir "zeroterm_extract"
    if (Test-Path $extractDir) { Remove-Item $extractDir -Recurse -Force }
    New-Item -ItemType Directory -Path $extractDir | Out-Null

    if ($assetName.EndsWith(".zip")) {
        Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force
    } else {
        tar -xzf $archivePath -C $extractDir
    }

    $binaryPath = Get-ChildItem -Path $extractDir -Filter $BinaryName -Recurse | Select-Object -First 1
    if (-not $binaryPath) {
        Write-Log "Binary not found in archive" "ERROR"
        exit 1
    }

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }

    Copy-Item $binaryPath.FullName -Destination (Join-Path $InstallDir $BinaryName) -Force
    Write-Log "Installed to $InstallDir\$BinaryName" "OK"
}

function Build-FromSource {
    Write-Log "Building from source..." "INFO"
    
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Log "Rust not found. Install from https://rustup.rs/" "ERROR"
        exit 1
    }

    $tempDir = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), "zeroterm_build_$(Get-Random)")
    if (Test-Path $tempDir) { Remove-Item $tempDir -Recurse -Force }
    
    Write-Log "Cloning repository..."
    git clone --depth 1 "https://github.com/$Repo.git" $tempDir

    Write-Log "Building release binary..."
    Set-Location "$tempDir"
    cargo build --release -p zeroterm

    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }
    Copy-Item "target/release/$BinaryName" -Destination (Join-Path $InstallDir $BinaryName) -Force
    Write-Log "Built and installed from source" "OK"
}

function Setup-Path {
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($currentPath -notlike "*$InstallDir*") {
        Write-Log "Adding $InstallDir to user PATH..." "INFO"
        $newPath = "$InstallDir;$currentPath"
        [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        Write-Log "Added to PATH. Restart your terminal." "WARN"
    } else {
        Write-Log "$InstallDir already in PATH" "OK"
    }
}

function Verify-Install {
    $binaryPath = Join-Path $InstallDir $BinaryName
    if (Test-Path $binaryPath) {
        try {
            & $binaryPath --version 2>$null
            Write-Log "Installation verified!" "OK"
        } catch {
            Write-Log "Binary installed but --version not implemented yet" "WARN"
        }
    } else {
        Write-Log "Binary not found at $binaryPath" "ERROR"
    }
}

function Install-StartMenuShortcut {
    $shortcutPath = Join-Path $StartMenuDir "ZeroTerm.lnk"
    if (-not (Test-Path $StartMenuDir)) {
        New-Item -ItemType Directory -Path $StartMenuDir -Force | Out-Null
    }
    $wshShell = New-Object -ComObject WScript.Shell
    $shortcut = $wshShell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = Join-Path $InstallDir $BinaryName
    $shortcut.WorkingDirectory = $InstallDir
    $shortcut.Description = "Zero latency, zero bloat, zero config terminal emulator"
    $shortcut.Save()
    Write-Log "Start Menu shortcut created" "OK"
}

function Uninstall-ZeroTerm {
    Write-Log "Uninstalling ZeroTerm..." "INFO"
    $binaryPath = Join-Path $InstallDir $BinaryName
    if (Test-Path $binaryPath) {
        Remove-Item $binaryPath -Force
        Write-Log "Removed: $binaryPath" "INFO"
    }
    $shortcutPath = Join-Path $StartMenuDir "ZeroTerm.lnk"
    if (Test-Path $shortcutPath) {
        Remove-Item $shortcutPath -Force
        Write-Log "Removed: $shortcutPath" "INFO"
    }
    if (Test-Path $StartMenuDir -and -not (Get-ChildItem $StartMenuDir)) {
        Remove-Item $StartMenuDir -Force
        Write-Log "Removed: $StartMenuDir" "INFO"
    }
    if ($Purge -and (Test-Path $ConfigDir)) {
        Remove-Item $ConfigDir -Recurse -Force
        Write-Log "Config removed: $ConfigDir" "INFO"
    }
    Write-Log "ZeroTerm uninstalled" "OK"
}

# Main
if ($Uninstall) {
    Uninstall-ZeroTerm
    return
}

Write-Host "╔══════════════════════════════════════════╗"
Write-Host "║     ZeroTerm Universal Installer         ║"
Write-Host "║  Zero latency, zero bloat, zero config   ║"
Write-Host "╚══════════════════════════════════════════╝"
Write-Host ""

$platform = Get-Platform
Write-Log "Detected platform: $platform"

$version = Get-LatestVersion
if (-not $version) {
    Write-Log "No releases found, building from source..." "WARN"
    Build-FromSource
} else {
    Write-Log "Latest version: $version"
    Install-Binary -Platform $platform -Version $version
}

Setup-Path
Verify-Install

if ($platform.StartsWith("windows")) {
    Install-StartMenuShortcut
}

Write-Host ""
Write-Log "ZeroTerm installed successfully!" "OK"
Write-Host ""
Write-Host "Run: $BinaryName"
Write-Host "Config: $env:USERPROFILE\.config\zeroterm\config.toml"