@echo off
setlocal enabledelayedexpansion

:: ============================================================
:: Loopbox Windows Release Build
:: ============================================================
:: Prerequisites: Rust toolchain, Dioxus CLI, Git
:: Run from x64 Native Tools Command Prompt for VS
::
:: Usage:
::   scripts\release-windows.bat [version]
::   scripts\release-windows.bat v0.3.0
:: ============================================================

set "SCRIPT_DIR=%~dp0"
set "PROJECT_DIR=%SCRIPT_DIR%.."

:: Version from argument or Cargo.toml
set "VERSION=%~1"
if "%VERSION%"=="" (
    for /f "tokens=3 delims= " %%a in ('findstr /r "^version" "%PROJECT_DIR%\Cargo.toml"') do (
        set "VERSION=%%~a"
        goto :got_version
    )
)
:got_version
set "VERSION=%VERSION:v=%"
set "RELEASE_TAG=v%VERSION%"

echo.
echo ============================================================
echo  Loopbox Windows Release: %RELEASE_TAG%
echo ============================================================
echo  Source: %PROJECT_DIR%
echo ============================================================
echo.

if "%DIOXUS_CLI_BIN%"=="" (
    set "DX_BIN=dx"
) else (
    set "DX_BIN=%DIOXUS_CLI_BIN%"
)
"%DX_BIN%" --version >nul 2>&1 || (
    echo ERROR: Dioxus CLI not found. Install with: cargo install dioxus-cli
    echo        If another dx is earlier in PATH, set DIOXUS_CLI_BIN to the Dioxus CLI path.
    exit /b 1
)
where cargo >nul 2>&1 || (
    echo ERROR: Rust toolchain not found. Install from https://rustup.rs
    exit /b 1
)
if not exist "%PROJECT_DIR%\Cargo.toml" (
    echo ERROR: Cargo.toml not found at %PROJECT_DIR%
    exit /b 1
)

for /f "tokens=3 delims= " %%a in ('findstr /r "^version" "%PROJECT_DIR%\Cargo.toml"') do (
    set "CURRENT=%%~a"
)
if not "%CURRENT%"=="%VERSION%" (
    echo    Updating Cargo.toml version: %CURRENT% -^> %VERSION%
    powershell -Command "(Get-Content '%PROJECT_DIR%\Cargo.toml') -replace '^version = \".*\"', 'version = \"%VERSION%\"' | Set-Content '%PROJECT_DIR%\Cargo.toml'"
)

echo [1/3] Building and bundling release...
cd /d "%PROJECT_DIR%"
"%DX_BIN%" bundle --platform desktop --release --package-types nsis
if errorlevel 1 (
    echo ERROR: Build/Bundle failed.
    exit /b 1
)

echo [2/3] Locating installer...
set "DIST_DIR=%PROJECT_DIR%\dist"
set "RELEASE_DIR=%PROJECT_DIR%\release-artifacts"
if not exist "%RELEASE_DIR%" mkdir "%RELEASE_DIR%"

set "INSTALLER_SRC="
for %%f in ("%DIST_DIR%\*setup*.exe" "%DIST_DIR%\*_x64*.exe") do (
    if exist "%%f" set "INSTALLER_SRC=%%f"
)
for /r "%DIST_DIR%\bundle" %%f in (*setup*.exe *_x64*.exe) do (
    set "INSTALLER_SRC=%%f"
)

set "RELEASE_NAME=Loopbox-%RELEASE_TAG%-windows-x64-setup.exe"

if defined INSTALLER_SRC (
    copy "!INSTALLER_SRC!" "%RELEASE_DIR%\%RELEASE_NAME%" >nul
    echo    Installer: %RELEASE_DIR%\%RELEASE_NAME%
) else (
    if exist "%DIST_DIR%\Loopbox.exe" (
        set "RELEASE_NAME=Loopbox-%RELEASE_TAG%-windows-x64.exe"
        copy "%DIST_DIR%\Loopbox.exe" "%RELEASE_DIR%\!RELEASE_NAME!" >nul
        echo    Binary: %RELEASE_DIR%\!RELEASE_NAME!
    ) else (
        echo ERROR: No build artifact found in %DIST_DIR%
        exit /b 1
    )
)

echo [3/3] Uploading to GitHub release...
where gh >nul 2>&1 || (
    echo NOTE: GitHub CLI not found. Upload manually:
    echo       %RELEASE_DIR%\%RELEASE_NAME%
    goto :done
)

gh release view %RELEASE_TAG% >nul 2>&1
if errorlevel 1 (
    echo    Creating release %RELEASE_TAG%...
    gh release create %RELEASE_TAG% "%RELEASE_DIR%\%RELEASE_NAME%" --title "%RELEASE_TAG%" --notes "Windows release %RELEASE_TAG%"
) else (
    echo    Uploading to existing release %RELEASE_TAG%...
    gh release upload %RELEASE_TAG% "%RELEASE_DIR%\%RELEASE_NAME%" --clobber
)

:done
echo.
echo ============================================================
echo  Release complete: %RELEASE_TAG%
echo  Artifact: %RELEASE_DIR%\%RELEASE_NAME%
echo ============================================================
