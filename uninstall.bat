@echo off
setlocal EnableExtensions
set "POWERSHELL_EXE=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
set "FSUTIL_EXE=%SystemRoot%\System32\fsutil.exe"
set "REG_EXE=%SystemRoot%\System32\reg.exe"
set "REGSVR32_EXE=%SystemRoot%\System32\regsvr32.exe"

"%POWERSHELL_EXE%" -NoProfile -NonInteractive -Command "$identity=[Security.Principal.WindowsIdentity]::GetCurrent(); $principal=[Security.Principal.WindowsPrincipal]::new($identity); if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { exit 0 }; exit 1"
if errorlevel 1 (
    echo ERROR: uninstall.bat requires an elevated Command Prompt.
    exit /b 1
)

set "PROGRAM_FILES_ROOT="
for /f "tokens=1,2,*" %%A in ('"%REG_EXE%" query "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion" /v ProgramFilesDir /reg:64 2^>nul') do if /I "%%A"=="ProgramFilesDir" set "PROGRAM_FILES_ROOT=%%C"
if not defined PROGRAM_FILES_ROOT goto :untrusted_runtime
if /I not "%PROGRAM_FILES_ROOT%"=="%ProgramFiles%" goto :untrusted_runtime
set "OUTDIR=%ProgramFiles%\noperson"
if not exist "%OUTDIR%\VCamRegistrar.exe" (
    echo ERROR: trusted VCamRegistrar.exe is missing from %OUTDIR%.
    echo Repair the installation before uninstalling the camera registration.
    exit /b 1
)

call :verify_runtime_tree
if errorlevel 1 goto :untrusted_runtime
call :verify_runtime_execution_files
if errorlevel 1 goto :untrusted_runtime

"%OUTDIR%\VCamRegistrar.exe" /remove
if errorlevel 1 (
    echo ERROR: the owned VCam registration could not be removed.
    echo Close camera clients and retry. If Windows retains the camera,
    echo restart Windows and rerun uninstall.bat.
    exit /b 1
)
if exist "%OUTDIR%\VCamSource.dll" (
    "%REGSVR32_EXE%" /u /s "%OUTDIR%\VCamSource.dll"
    if errorlevel 1 exit /b 1
)
echo VCam unregistered. Runtime files remain in %OUTDIR%.
endlocal
exit /b 0

:verify_runtime_tree
"%POWERSHELL_EXE%" -NoProfile -NonInteractive -Command "$ErrorActionPreference='Stop'; try { $full=[Security.AccessControl.FileSystemRights]::FullControl; $write=[Security.AccessControl.FileSystemRights]::WriteData -bor [Security.AccessControl.FileSystemRights]::AppendData -bor [Security.AccessControl.FileSystemRights]::WriteExtendedAttributes -bor [Security.AccessControl.FileSystemRights]::WriteAttributes -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership; $system='S-1-5-18'; $admins='S-1-5-32-544'; $paths=@($env:OUTDIR,(Join-Path $env:OUTDIR 'VCamRegistrar.exe')); $dll=Join-Path $env:OUTDIR 'VCamSource.dll'; if ([IO.File]::Exists($dll)) { $paths += $dll }; foreach ($path in $paths) { $item=Get-Item -LiteralPath $path -Force; if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { exit 10 }; if ($path -ne $env:OUTDIR -and $item.PSIsContainer) { exit 11 }; $acl=Get-Acl -LiteralPath $path; if (-not $acl.AreAccessRulesProtected) { exit 12 }; $owner=([Security.Principal.NTAccount]$acl.Owner).Translate([Security.Principal.SecurityIdentifier]).Value; if ($owner -notin @($system,$admins)) { exit 13 }; $systemFull=$false; $adminFull=$false; foreach ($rule in $acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])) { if ($rule.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) { exit 14 }; $sid=$rule.IdentityReference.Value; $rights=[Security.AccessControl.FileSystemRights]$rule.FileSystemRights; if ($sid -eq $system -and (($rights -band $full) -eq $full)) { $systemFull=$true } elseif ($sid -eq $admins -and (($rights -band $full) -eq $full)) { $adminFull=$true } elseif (($rights -band $write) -ne 0) { exit 15 } }; if (-not ($systemFull -and $adminFull)) { exit 16 } }; exit 0 } catch { exit 17 }"
exit /b %errorlevel%

:verify_runtime_execution_files
"%POWERSHELL_EXE%" -NoProfile -NonInteractive -Command "$ErrorActionPreference='Stop'; try { $links=@(& $env:FSUTIL_EXE hardlink list (Join-Path $env:OUTDIR 'VCamRegistrar.exe')); if ($LASTEXITCODE -ne 0 -or $links.Count -ne 1) { exit 20 }; exit 0 } catch { exit 21 }"
exit /b %errorlevel%

:untrusted_runtime
echo ERROR: refusing to execute an untrusted, writable, linked, or redirected runtime helper.
exit /b 1
