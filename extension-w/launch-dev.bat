@echo off
REM Launch Chrome with extension-w loaded in development mode (Windows)

setlocal enabledelayedexpansion

set SCRIPT_DIR=%~dp0
set EXTENSION_DIR=%SCRIPT_DIR%

echo === Oryn-W Extension Dev Launcher (Windows) ===
echo.

REM Check if WASM module exists
if not exist "%EXTENSION_DIR%wasm\oryn_core_bg.wasm" (
    echo ERROR: WASM module not found!
    echo.
    echo Please build the WASM module first:
    echo   cd ..\crates\oryn-core
    echo   wasm-pack build --target web --out-dir ..\..\extension-w\wasm --release
    echo.
    pause
    exit /b 1
)

REM Find Chrome binary
set CHROME_BIN=

if exist "%ProgramFiles%\Google\Chrome\Application\chrome.exe" (
    set CHROME_BIN=%ProgramFiles%\Google\Chrome\Application\chrome.exe
) else if exist "%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe" (
    set CHROME_BIN=%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe
) else if exist "%LocalAppData%\Google\Chrome\Application\chrome.exe" (
    set CHROME_BIN=%LocalAppData%\Google\Chrome\Application\chrome.exe
) else if exist "%ProgramFiles%\Chromium\Application\chrome.exe" (
    set CHROME_BIN=%ProgramFiles%\Chromium\Application\chrome.exe
) else (
    echo ERROR: Chrome not found!
    echo.
    echo Please install Google Chrome from: https://www.google.com/chrome/
    echo.
    pause
    exit /b 1
)

echo [OK] Found Chrome: %CHROME_BIN%
echo [OK] Extension directory: %EXTENSION_DIR%
echo.

REM Create temporary user data directory
set USER_DATA_DIR=%TEMP%\oryn-w-dev-%RANDOM%
mkdir "%USER_DATA_DIR%"

echo Launching Chrome with extension...
echo.
echo Extension loaded from: %EXTENSION_DIR%
echo User data directory: %USER_DATA_DIR%
echo.
echo Press Ctrl+C to stop Chrome
echo.

REM Launch Chrome with extension
"%CHROME_BIN%" ^
    --user-data-dir="%USER_DATA_DIR%" ^
    --disable-extensions-except="%EXTENSION_DIR%" ^
    --load-extension="%EXTENSION_DIR%" ^
    --no-first-run ^
    --no-default-browser-check ^
    --disable-features=ExtensionsToolbarMenu ^
    "https://example.com"

REM Cleanup
echo.
echo Cleaning up temporary user data...
rmdir /s /q "%USER_DATA_DIR%"
echo Done!
pause
