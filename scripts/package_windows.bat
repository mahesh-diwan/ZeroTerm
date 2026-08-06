@echo off
REM ZeroTerm Windows packaging: build release, zip, and (if WiX present) an MSI.
REM Run from the repo root on Windows (cmd.exe or Git Bash/MSYS).
setlocal

if not defined VERSION (
  for /f "delims=" %%i in ('git describe --tags --always 2^>nul') do set "VERSION=%%i"
)
if not defined VERSION set "VERSION=0.3.0"

REM uname guard: present under Git Bash/MSYS, absent in plain cmd.exe.
REM Used only to print a cross-compile hint; packaging works either way.
set "HAVE_UNAME=0"
where uname >nul 2>nul && set "HAVE_UNAME=1"
if "%HAVE_UNAME%"=="1" (
  echo [INFO] Detected unix-like shell (Git Bash/MSYS).
)

where cargo >nul 2>nul || (
  echo [ERROR] Rust not installed. Install from: https://rustup.rs
  exit /b 1
)

if not exist "target\release\zeroterm.exe" (
  echo [INFO] Building release binary...
  cargo build --release -p zeroterm
  if errorlevel 1 (
    echo [ERROR] Build failed.
    exit /b 1
  )
)

if not exist "dist" mkdir dist

echo [INFO] Packaging zip...
powershell -NoProfile -Command "Compress-Archive -Force -Path target\release\zeroterm.exe,README.md -DestinationPath 'dist\zeroterm-%VERSION%-windows-x86_64.zip'"
if errorlevel 1 (
  echo [ERROR] Zip packaging failed.
  exit /b 1
)
echo [OK] Created dist\zeroterm-%VERSION%-windows-x86_64.zip

REM Optional: WiX MSI (only when candle + light are on PATH)
where candle >nul 2>nul || goto :done
where light >nul 2>nul || goto :done
echo [INFO] Building MSI with WiX...
if not exist "dist\wix" mkdir "dist\wix"
candle -o "dist\wix\" "scripts\windows_installer.wxs"
if errorlevel 1 (
  echo [WARN] candle failed; skipping MSI. Ensure scripts\windows_installer.wxs has a real UpgradeCode.
  goto :done
)
light -o "dist\zeroterm-%VERSION%-x86_64.msi" "dist\wix\*.wixobj"
if not errorlevel 1 echo [OK] Created dist\zeroterm-%VERSION%-x86_64.msi

:done
echo [OK] Packaging complete. Artifacts in dist\
endlocal
