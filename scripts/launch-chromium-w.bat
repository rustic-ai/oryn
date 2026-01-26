@echo off
REM Launch Chromium with extension-w loaded in development mode (Windows)

setlocal enabledelayedexpansion

set SCRIPT_DIR=%~dp0
set PROJECT_ROOT=%SCRIPT_DIR%..
set EXTENSION_DIR=%PROJECT_ROOT%\extension-w

echo === Oryn-W Extension Dev Launcher (Windows) ===
echo.

REM Check if WASM module exists
if not exist "%EXTENSION_DIR%\wasm\oryn_core_bg.wasm" (
    echo ERROR: WASM module not found!
    echo.
    echo Please build the WASM module first:
    echo   scripts\build-wasm.bat
    echo Or manually:
    echo   cd crates\oryn-core
    echo   wasm-pack build --target web --out-dir ..\..\extension-w\wasm --release
    echo.
    pause
    exit /b 1
)

REM Find Chromium binary (Chrome doesn't support unpacked extensions via CLI)
set CHROME_BIN=

if exist "%ProgramFiles%\Chromium\Application\chrome.exe" (
    set CHROME_BIN=%ProgramFiles%\Chromium\Application\chrome.exe
) else if exist "%ProgramFiles(x86)%\Chromium\Application\chrome.exe" (
    set CHROME_BIN=%ProgramFiles(x86)%\Chromium\Application\chrome.exe
) else if exist "%LocalAppData%\Chromium\Application\chrome.exe" (
    set CHROME_BIN=%LocalAppData%\Chromium\Application\chrome.exe
) else if exist "%ProgramFiles%\Google\Chrome\Application\chrome.exe" (
    echo WARNING: Found Chrome but it may not support unpacked extensions via CLI
    echo Consider installing Chromium for development
    echo.
    set CHROME_BIN=%ProgramFiles%\Google\Chrome\Application\chrome.exe
) else if exist "%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe" (
    echo WARNING: Found Chrome but it may not support unpacked extensions via CLI
    echo Consider installing Chromium for development
    echo.
    set CHROME_BIN=%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe
) else if exist "%LocalAppData%\Google\Chrome\Application\chrome.exe" (
    echo WARNING: Found Chrome but it may not support unpacked extensions via CLI
    echo Consider installing Chromium for development
    echo.
    set CHROME_BIN=%LocalAppData%\Google\Chrome\Application\chrome.exe
) else (
    echo ERROR: Chromium not found!
    echo.
    echo Please install Chromium (Chrome doesn't support unpacked extensions via CLI):
    echo   Download from: https://www.chromium.org/getting-involved/download-chromium/
    echo   Or use Chocolatey: choco install chromium
    echo.
    pause
    exit /b 1
)

echo [OK] Found Chromium: %CHROME_BIN%
echo [OK] Extension directory: %EXTENSION_DIR%
echo.

REM Create temporary user data directory
set USER_DATA_DIR=%TEMP%\oryn-w-dev-%RANDOM%
mkdir "%USER_DATA_DIR%\Default"

REM Create preferences to pin the extension to toolbar
echo {> "%USER_DATA_DIR%\Default\Preferences"
echo   "browser": {>> "%USER_DATA_DIR%\Default\Preferences"
echo     "show_toolbar_bookmarks_button": false>> "%USER_DATA_DIR%\Default\Preferences"
echo   },>> "%USER_DATA_DIR%\Default\Preferences"
echo   "extensions": {>> "%USER_DATA_DIR%\Default\Preferences"
echo     "ui": {>> "%USER_DATA_DIR%\Default\Preferences"
echo       "developer_mode": true>> "%USER_DATA_DIR%\Default\Preferences"
echo     }>> "%USER_DATA_DIR%\Default\Preferences"
echo   }>> "%USER_DATA_DIR%\Default\Preferences"
echo }>> "%USER_DATA_DIR%\Default\Preferences"

echo Launching Chromium with extension...
echo.
echo Extension loaded from: %EXTENSION_DIR%
echo User data directory: %USER_DATA_DIR%
echo.
echo Press Ctrl+C to stop Chromium
echo.

REM Launch Chromium with extension
REM --disable-features=ExtensionsToolbarMenu keeps extension icons in toolbar (not hidden in menu)
"%CHROME_BIN%" ^
    --user-data-dir="%USER_DATA_DIR%" ^
    --disable-extensions-except="%EXTENSION_DIR%" ^
    --load-extension="%EXTENSION_DIR%" ^
    --no-first-run ^
    --no-default-browser-check ^
    --disable-features=ExtensionsToolbarMenu ^
    --show-component-extension-options ^
    "https://example.com"

REM Cleanup
echo.
echo Cleaning up temporary user data...
rmdir /s /q "%USER_DATA_DIR%"
echo Done!
pause
