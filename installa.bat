@echo off
cd /d "%~dp0"
cargo build --release || (pause & exit /b 1)
if not exist config.json copy config.example.json config.json
echo.
echo Periferiche trovate:
target\release\logi-edge-switch.exe --list
pause
