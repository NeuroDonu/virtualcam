@echo off
setlocal
set OUTDIR=C:\vcam
taskkill /F /IM vcam-pump.exe >nul 2>nul
taskkill /F /IM VCamRegistrar.exe >nul 2>nul
if exist "%OUTDIR%\VCamSource.dll" (
    regsvr32 /u /s "%OUTDIR%\VCamSource.dll"
    if errorlevel 1 exit /b 1
)
echo VCam unregistered. Runtime files remain in %OUTDIR%.
endlocal
