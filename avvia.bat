@echo off
cd /d "%~dp0"
start "" target\release\logi-edge-switch.exe -c "%~dp0config.json" %*
