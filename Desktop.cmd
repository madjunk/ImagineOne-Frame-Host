@echo off
rem Double-click to open the ImagineOne desktop app menu.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0Desktop.ps1" %*
echo.
pause
