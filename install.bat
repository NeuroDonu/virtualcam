@echo off
REM ============================================================================
REM VCam source installer: build without elevation, then pass immutable hashes
REM and a read-once PowerShell payload through one fixed encoded UAC bootstrap.
REM ============================================================================

setlocal EnableExtensions EnableDelayedExpansion
set "ROOT=%~dp0"
set "SRCDIR=%ROOT%cpp\VCam"
set "INSTALL_PAYLOAD=%ROOT%scripts\install-runtime.ps1"
set "POWERSHELL_EXE=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"

if not "%~1"=="" goto :usage
if not exist "%POWERSHELL_EXE%" (
    echo ERROR: Windows PowerShell is missing from System32.
    exit /b 1
)
if not exist "%INSTALL_PAYLOAD%" (
    echo ERROR: trusted install payload is missing: %INSTALL_PAYLOAD%
    exit /b 1
)

call :is_elevated
set "ELEVATION_STATE=%errorlevel%"
if "%ELEVATION_STATE%"=="0" (
    echo ERROR: refusing to build from an elevated shell.
    echo Open a normal Command Prompt and run install.bat.
    exit /b 1
)
if not "%ELEVATION_STATE%"=="1" (
    echo ERROR: could not determine whether the build shell is elevated.
    exit /b 1
)

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

call "%ROOT%scripts\bootstrap-nuget.bat"
if errorlevel 1 (
    echo NUGET PACKAGE BOOTSTRAP FAILED
    exit /b 1
)

set "MSBUILD="
where msbuild >nul 2>nul
if %errorlevel%==0 set "MSBUILD=msbuild"
if defined MSBUILD goto :found_msbuild
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if exist "%VSWHERE%" (
    for /f "usebackq tokens=*" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.Component.MSBuild -find MSBuild\**\Bin\MSBuild.exe`) do if not defined MSBUILD set "MSBUILD=%%I"
)
if defined MSBUILD goto :found_msbuild
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Professional\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\Professional\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Enterprise\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles%\Microsoft Visual Studio\2022\Enterprise\MSBuild\Current\Bin\MSBuild.exe"
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\MSBuild.exe" set "MSBUILD=%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\MSBuild.exe"
if defined MSBUILD goto :found_msbuild
echo ERROR: msbuild not found. Install Visual Studio 2022 Build Tools with the C++ workload.
exit /b 1

:found_msbuild
echo === Building VCamSource.dll and VCamRegistrar.exe ===
pushd "%SRCDIR%"
"%MSBUILD%" VCam.sln /p:Configuration=Release /p:Platform=x64 /m /nologo /v:m
if errorlevel 1 (
    echo BUILD FAILED
    popd
    exit /b 1
)
popd

set "ARTIFACT_SOURCE_DLL=%SRCDIR%\x64\Release\VCamSource.dll"
if not exist "%ARTIFACT_SOURCE_DLL%" set "ARTIFACT_SOURCE_DLL=%SRCDIR%\x64\Release\VCamSource\VCamSource.dll"
set "ARTIFACT_REGISTRAR=%SRCDIR%\x64\Release\VCamRegistrar.exe"
set "ARTIFACT_PUMP=%ROOT%target\release\vcam-pump.exe"
set "ARTIFACT_VERIFY=%ROOT%target\release\vcam-verify.exe"
for %%F in ("%ARTIFACT_SOURCE_DLL%" "%ARTIFACT_REGISTRAR%" "%ARTIFACT_PUMP%" "%ARTIFACT_VERIFY%") do if not exist "%%~F" (
    echo ERROR: expected build artifact is missing: %%~F
    exit /b 1
)

echo === Build complete; requesting one verified elevated install ===
"%POWERSHELL_EXE%" -NoProfile -NonInteractive -Command "$ErrorActionPreference='Stop'; try { $payloadBytes=[IO.File]::ReadAllBytes($env:INSTALL_PAYLOAD); $sha=[Security.Cryptography.SHA256]::Create(); try { $payloadHash=([BitConverter]::ToString($sha.ComputeHash($payloadBytes))).Replace('-','') } finally { $sha.Dispose() }; $artifacts=@([pscustomobject]@{Name='VCamSource.dll';Path=$env:ARTIFACT_SOURCE_DLL;Hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $env:ARTIFACT_SOURCE_DLL).Hash.ToUpperInvariant()},[pscustomobject]@{Name='VCamRegistrar.exe';Path=$env:ARTIFACT_REGISTRAR;Hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $env:ARTIFACT_REGISTRAR).Hash.ToUpperInvariant()},[pscustomobject]@{Name='vcam-pump.exe';Path=$env:ARTIFACT_PUMP;Hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $env:ARTIFACT_PUMP).Hash.ToUpperInvariant()},[pscustomobject]@{Name='vcam-verify.exe';Path=$env:ARTIFACT_VERIFY;Hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $env:ARTIFACT_VERIFY).Hash.ToUpperInvariant()}); $context=[pscustomobject]@{PayloadPath=$env:INSTALL_PAYLOAD;PayloadHash=$payloadHash;Artifacts=$artifacts}; $contextJson=$context | ConvertTo-Json -Compress -Depth 4; $contextB64=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($contextJson)); $bootstrap='$ErrorActionPreference=''Stop'';$contextB64=''__CONTEXT__'';$contextJson=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($contextB64));$context=$contextJson|ConvertFrom-Json;$payloadBytes=[IO.File]::ReadAllBytes([string]$context.PayloadPath);$sha=[Security.Cryptography.SHA256]::Create();try{$payloadHash=([BitConverter]::ToString($sha.ComputeHash($payloadBytes))).Replace(''-'','''')}finally{$sha.Dispose()};if($payloadHash -ne [string]$context.PayloadHash){throw ''install payload changed before elevation''};$payloadText=([Text.UTF8Encoding]::new($false,$true)).GetString($payloadBytes);$scriptBlock=[ScriptBlock]::Create($payloadText);& $scriptBlock -Artifacts $context.Artifacts;exit 0'; $bootstrap=$bootstrap.Replace('__CONTEXT__',$contextB64); $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($bootstrap)); $process=Start-Process -FilePath $env:POWERSHELL_EXE -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -Verb RunAs -Wait -PassThru -ErrorAction Stop; exit $process.ExitCode } catch { Write-Error $_; exit 1 }"
set "INSTALL_EXIT=%errorlevel%"
endlocal
exit /b %INSTALL_EXIT%

:is_elevated
"%POWERSHELL_EXE%" -NoProfile -NonInteractive -Command "$identity=[Security.Principal.WindowsIdentity]::GetCurrent(); $principal=[Security.Principal.WindowsPrincipal]::new($identity); if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { exit 0 }; exit 1"
exit /b %errorlevel%

:usage
echo Usage: install.bat
exit /b 2
