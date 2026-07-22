@echo off
REM ============================================================================
REM VCam: build native runtime, register it, and run the end-to-end check.
REM Run this on the Windows VM after MSVC + Windows 11 SDK are installed
REM (Visual Studio 2022 Build Tools + "Desktop development with C++" workload
REM + the "Windows 11 SDK" individual component is enough).
REM
REM Usage: C:\vcam\install.bat
REM ============================================================================

setlocal enabledelayedexpansion
set ROOT=%~dp0
set SRCDIR=%ROOT%cpp\VCam
set RUNTIME_DIR=C:\vcam
mkdir %RUNTIME_DIR% 2>nul

REM --- 1. Build Rust transport tools ---
where cargo >nul 2>nul
if errorlevel 1 (
    echo ERROR: cargo not found. Install the stable MSVC Rust toolchain.
    exit /b 1
)
pushd "%ROOT%"
cargo build --release --no-default-features --features media-foundation --bin vcam-pump --bin vcam-verify
if errorlevel 1 (
    echo RUST BUILD FAILED
    popd
    exit /b 1
)
popd
copy /Y "%ROOT%target\release\vcam-pump.exe" "%RUNTIME_DIR%\vcam-pump.exe" >nul
if errorlevel 1 exit /b 1
copy /Y "%ROOT%target\release\vcam-verify.exe" "%RUNTIME_DIR%\vcam-verify.exe" >nul
if errorlevel 1 exit /b 1

REM --- 2. Download pinned native packages without MSBuild/NuGet restore ---
call "%ROOT%scripts\bootstrap-nuget.bat"
if errorlevel 1 (
    echo NUGET PACKAGE BOOTSTRAP FAILED
    exit /b 1
)

REM --- 3. Find MSBuild ---
where msbuild >nul 2>nul
if %errorlevel%==0 goto :found_msbuild
REM Common VS 2022 editions. Avoid nested cmd parsing around ProgramFiles(x86).
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Professional\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\Professional\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Enterprise\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\Enterprise\MSBuild\Current\Bin\MSBuild.exe"
if defined MSBUILD goto :found_msbuild
echo ERROR: msbuild not found. Install Visual Studio 2022 Build Tools with the C++ workload.
exit /b 1
:found_msbuild
if "%MSBUILD%"=="" set MSBUILD=msbuild

REM --- 4. Build VCamSource.dll and VCamRegistrar.exe ---
echo === Building VCamSource.dll ===
taskkill /F /IM VCamRegistrar.exe >nul 2>nul
taskkill /F /IM vcam-pump.exe >nul 2>nul
REM Frame Server loads the COM DLL in-process and keeps the file locked after
REM consumers exit. Stop both services before replacing the binary; Windows
REM starts them again automatically when the camera is activated.
net stop FrameServerMonitor /y >nul 2>nul
net stop FrameServer /y >nul 2>nul
pushd "%SRCDIR%"
"%MSBUILD%" VCam.sln /p:Configuration=Release /p:Platform=x64 /m /nologo /v:m
if errorlevel 1 (
    echo BUILD FAILED
    popd
    exit /b 1
)
popd

REM --- 5. Copy runtime files to a Frame Server-readable directory ---
set BUILT=%SRCDIR%\x64\Release\VCamSource.dll
if not exist "%BUILT%" set BUILT=%SRCDIR%\x64\Release\VCamSource\VCamSource.dll
if not exist "%BUILT%" (
    echo ERROR: cannot find built VCamSource.dll under x64\Release\
    dir /s /b "%SRCDIR%\x64\Release\VCamSource.dll" 2>nul
    exit /b 1
)
copy /Y "%BUILT%" "%RUNTIME_DIR%\VCamSource.dll"
if errorlevel 1 (
    echo ERROR: cannot replace VCamSource.dll. Frame Server still owns it.
    exit /b 1
)
set VCAMEXE=%SRCDIR%\x64\Release\VCamRegistrar.exe
if not exist "%VCAMEXE%" (
    echo ERROR: cannot find VCamRegistrar.exe under x64\Release\
    exit /b 1
)
copy /Y "%VCAMEXE%" "%RUNTIME_DIR%\VCamRegistrar.exe"
if errorlevel 1 exit /b 1
echo === Runtime copied to %RUNTIME_DIR% ===

REM --- 6. Register the DLL (HKLM, requires admin) ---
echo === Registering VCamSource.dll ===
regsvr32 /s "%RUNTIME_DIR%\VCamSource.dll"
if errorlevel 1 (
    echo ERROR: regsvr32 failed with exit %errorlevel%
    exit /b 1
)
echo.
echo === Verify CLSID is registered ===
reg query "HKLM\SOFTWARE\Classes\CLSID\{3cad447d-f283-4af4-a3b2-6f5363309f52}\InprocServer32"
if errorlevel 1 exit /b 1

REM --- 7. Start the session-lifetime Media Foundation camera registrar ---
echo.
echo === Starting Media Foundation virtual camera ===
powershell -NoProfile -Command "Start-Process -FilePath '%RUNTIME_DIR%\VCamRegistrar.exe' -ArgumentList '/headless' -WindowStyle Hidden"
powershell -NoProfile -Command "Start-Sleep -Seconds 3"
tasklist /FI "IMAGENAME eq VCamRegistrar.exe" | find /I "VCamRegistrar.exe" >nul
if errorlevel 1 (
    echo ERROR: VCamRegistrar.exe exited; virtual camera registration failed.
    exit /b 1
)

REM --- 8. Run the producer and verify through Media Foundation ---
echo.
echo === Starting 30 FPS green-frame pump ===
if not exist "%RUNTIME_DIR%\vcam-verify.exe" (
    echo ERROR: %RUNTIME_DIR%\vcam-verify.exe is missing.
    exit /b 1
)
taskkill /F /IM vcam-pump.exe >nul 2>nul
powershell -NoProfile -Command "Start-Process -FilePath '%RUNTIME_DIR%\vcam-pump.exe' -RedirectStandardOutput '%RUNTIME_DIR%\pump.log' -RedirectStandardError '%RUNTIME_DIR%\pump.err' -WindowStyle Hidden"
powershell -NoProfile -Command "Start-Sleep -Seconds 2"
"%RUNTIME_DIR%\vcam-verify.exe"
if errorlevel 1 (
    echo ERROR: Media Foundation end-to-end verification failed.
    exit /b 1
)
echo === PASS: VCam is streaming injected NV12 frames ===
endlocal
