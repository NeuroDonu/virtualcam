@echo off
setlocal
powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%~dp0bootstrap-nuget.ps1" %*
exit /b %errorlevel%
