@echo off
if exist "%~dp0raytrace-gui.exe" (
    start "" /D "%~dp0" "%~dp0raytrace-gui.exe"
) else (
    start "" /D "%~dp0" "%~dp0target\release\raytrace-gui.exe"
)
