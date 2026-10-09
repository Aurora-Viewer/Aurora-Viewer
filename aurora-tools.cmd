@echo off
rem Aurora Viewer development tools (double-click): scripts\tools\aurora-tools.ps1
title Aurora Tools
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\tools\aurora-tools.ps1" %*
