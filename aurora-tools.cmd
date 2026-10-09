@echo off
rem Aurora Viewer development tools (double-click). In the repository or in the
rem project folder next to aurora-viewer\, it opens scripts\tools\aurora-tools.ps1;
rem alone in a folder, it downloads the installer and installs the project there.
rem (goto rather than ( ) blocks: a path with a parenthesis would break a block)
title Aurora Tools
cd /d "%~dp0"
if exist "%~dp0scripts\tools\aurora-tools.ps1" goto repo
if exist "%~dp0aurora-viewer\scripts\tools\aurora-tools.ps1" goto project
powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"
pause
goto :eof
:repo
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\tools\aurora-tools.ps1" %*
goto :eof
:project
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0aurora-viewer\scripts\tools\aurora-tools.ps1" %*
