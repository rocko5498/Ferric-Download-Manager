@echo off
rem Double-click to install Ferric for the current user (no administrator rights needed).
powershell.exe -ExecutionPolicy Bypass -File "%~dp0Install-Ferric.ps1"
pause
