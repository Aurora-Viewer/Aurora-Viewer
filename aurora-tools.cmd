@echo off
setlocal DisableDelayedExpansion
rem Aurora Tools entry point: keyboard menus, PR tracking and release management.
rem Load scripts\tools\aurora-tools.ps1 from the repository or the project folder;
rem those scripts reopen the menu automatically after a tools update.
rem Alone in a folder, download the installer and install the project there.
rem (goto rather than ( ) blocks: a path with a parenthesis would break a block)
title Aurora Tools
cd /d "%~dp0"
if exist "%~dp0scripts\tools\aurora-tools.ps1" goto repo
if exist "%~dp0aurora-viewer\scripts\tools\aurora-tools.ps1" goto project
powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"
set "AuroraToolsExitCode=%errorlevel%"
pause
exit /b %AuroraToolsExitCode%
:repo
set "AuroraToolsScript=%~dp0scripts\tools\aurora-tools.ps1"
goto launch
:project
set "AuroraToolsScript=%~dp0aurora-viewer\scripts\tools\aurora-tools.ps1"
:launch
powershell -NoProfile -ExecutionPolicy Bypass -File "%AuroraToolsScript%" %*
exit /b %errorlevel%
