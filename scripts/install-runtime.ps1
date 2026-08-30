param(
    [Parameter(Mandatory = $true)]
    [object[]] $Artifacts
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$SystemSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$AdministratorsSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
$InteractiveUserSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
$WriteMask = [Security.AccessControl.FileSystemRights]::WriteData `
    -bor [Security.AccessControl.FileSystemRights]::AppendData `
    -bor [Security.AccessControl.FileSystemRights]::WriteExtendedAttributes `
    -bor [Security.AccessControl.FileSystemRights]::WriteAttributes `
    -bor [Security.AccessControl.FileSystemRights]::Delete `
    -bor [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles `
    -bor [Security.AccessControl.FileSystemRights]::ChangePermissions `
    -bor [Security.AccessControl.FileSystemRights]::TakeOwnership

function Get-BytesSha256 {
    param([Parameter(Mandatory = $true)][byte[]] $Bytes)

    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '')
    }
    finally {
        $sha.Dispose()
    }
}

function Read-VerifiedArtifact {
    param([Parameter(Mandatory = $true)] $Artifact)

    $bytes = [IO.File]::ReadAllBytes([string] $Artifact.Path)
    $actualHash = Get-BytesSha256 -Bytes $bytes
    $expectedHash = ([string] $Artifact.Hash).ToUpperInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "Artifact changed before elevation: $($Artifact.Name)"
    }
    return [pscustomobject]@{
        Name = [string] $Artifact.Name
        Bytes = $bytes
    }
}

function New-ExactRuntimeAcl {
    param([Parameter(Mandatory = $true)][bool] $Directory)

    if ($Directory) {
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit `
            -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit
    }
    else {
        $acl = [Security.AccessControl.FileSecurity]::new()
        $inheritance = [Security.AccessControl.InheritanceFlags]::None
    }
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner($script:AdministratorsSid)
    $propagation = [Security.AccessControl.PropagationFlags]::None
    $allow = [Security.AccessControl.AccessControlType]::Allow
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $script:SystemSid,
        [Security.AccessControl.FileSystemRights]::FullControl,
        $inheritance,
        $propagation,
        $allow
    ))
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $script:AdministratorsSid,
        [Security.AccessControl.FileSystemRights]::FullControl,
        $inheritance,
        $propagation,
        $allow
    ))
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $script:InteractiveUserSid,
        [Security.AccessControl.FileSystemRights]::ReadAndExecute,
        $inheritance,
        $propagation,
        $allow
    ))
    return $acl
}

function Assert-TrustedRuntimeRoot {
    param([Parameter(Mandatory = $true)][string] $Path)

    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer -or
        (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw "Runtime root is not a plain directory: $Path"
    }
    $acl = Get-Acl -LiteralPath $Path
    if (-not $acl.AreAccessRulesProtected) {
        throw "Runtime root inherits an untrusted DACL: $Path"
    }
    $owner = ([Security.Principal.NTAccount] $acl.Owner).Translate(
        [Security.Principal.SecurityIdentifier]
    ).Value
    if ($owner -notin @($script:SystemSid.Value, $script:AdministratorsSid.Value)) {
        throw "Runtime root owner is not trusted: $owner"
    }

    $systemFull = $false
    $administratorsFull = $false
    foreach ($rule in $acl.GetAccessRules(
        $true,
        $true,
        [Security.Principal.SecurityIdentifier]
    )) {
        if ($rule.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow) {
            throw 'Runtime root contains a deny or unknown ACE'
        }
        $sid = $rule.IdentityReference.Value
        $rights = [Security.AccessControl.FileSystemRights] $rule.FileSystemRights
        if ($sid -eq $script:SystemSid.Value -and
            (($rights -band [Security.AccessControl.FileSystemRights]::FullControl) -eq
                [Security.AccessControl.FileSystemRights]::FullControl)) {
            $systemFull = $true
        }
        elseif ($sid -eq $script:AdministratorsSid.Value -and
            (($rights -band [Security.AccessControl.FileSystemRights]::FullControl) -eq
                [Security.AccessControl.FileSystemRights]::FullControl)) {
            $administratorsFull = $true
        }
        elseif (($rights -band $script:WriteMask) -ne 0) {
            throw "Runtime root grants write access to $sid"
        }
    }
    if (-not ($systemFull -and $administratorsFull)) {
        throw 'Runtime root does not grant SYSTEM and Administrators full control'
    }
}

function Get-SafeRuntimeTree {
    param([Parameter(Mandatory = $true)][string] $Root)

    $items = [Collections.Generic.List[string]]::new()
    $items.Add($Root)
    foreach ($path in [IO.Directory]::EnumerateFileSystemEntries($Root)) {
        $attributes = [IO.File]::GetAttributes($path)
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Reparse points are forbidden in the runtime tree: $path"
        }
        if (($attributes -band [IO.FileAttributes]::Directory) -ne 0) {
            throw "Nested runtime directories are forbidden: $path"
        }
        $items.Add($path)
    }
    return ,$items.ToArray()
}

function Protect-RuntimeTree {
    param([Parameter(Mandatory = $true)][string] $Root)

    foreach ($path in Get-SafeRuntimeTree -Root $Root) {
        $isDirectory = [IO.Directory]::Exists($path)
        Set-Acl -LiteralPath $path -AclObject (New-ExactRuntimeAcl -Directory $isDirectory)
    }
    Assert-TrustedRuntimeRoot -Path $Root
}

function Get-ProgramFilesRoot {
    $properties = Get-ItemProperty -LiteralPath (
        'Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion'
    )
    $programFiles = [IO.Path]::GetFullPath([string] $properties.ProgramFilesDir).TrimEnd('\')
    $environmentPath = [IO.Path]::GetFullPath($env:ProgramFiles).TrimEnd('\')
    if (-not $programFiles.Equals($environmentPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'ProgramFiles environment does not match the machine ProgramFilesDir'
    }
    $item = Get-Item -LiteralPath $programFiles -Force
    if (-not $item.PSIsContainer -or
        (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw 'ProgramFilesDir is not a trusted plain directory'
    }
    return $programFiles
}

function Install-Runtime {
    param([Parameter(Mandatory = $true)][object[]] $ArtifactSpecs)

    $principal = [Security.Principal.WindowsPrincipal]::new(
        [Security.Principal.WindowsIdentity]::GetCurrent()
    )
    if (-not $principal.IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator
    )) {
        throw 'The install payload must run elevated'
    }

    $requiredNames = @(
        'VCamSource.dll',
        'VCamRegistrar.exe',
        'vcam-pump.exe',
        'vcam-verify.exe'
    )
    if ($ArtifactSpecs.Count -ne $requiredNames.Count) {
        throw 'Unexpected artifact count'
    }
    $verified = @{}
    foreach ($artifact in $ArtifactSpecs) {
        $name = [string] $artifact.Name
        if ($name -notin $requiredNames -or $verified.ContainsKey($name) -or
            ([string] $artifact.Hash) -notmatch '^[0-9A-Fa-f]{64}$') {
            throw "Invalid artifact descriptor: $name"
        }
        $verified[$name] = Read-VerifiedArtifact -Artifact $artifact
    }
    foreach ($name in $requiredNames) {
        if (-not $verified.ContainsKey($name)) {
            throw "Missing artifact: $name"
        }
    }

    $programFiles = Get-ProgramFilesRoot
    $runtimeRoot = Join-Path $programFiles 'noperson'
    if ([IO.Directory]::Exists($runtimeRoot)) {
        Assert-TrustedRuntimeRoot -Path $runtimeRoot
    }
    else {
        [IO.Directory]::CreateDirectory($runtimeRoot) | Out-Null
        Set-Acl -LiteralPath $runtimeRoot -AclObject (New-ExactRuntimeAcl -Directory $true)
    }
    Protect-RuntimeTree -Root $runtimeRoot

    $partialPaths = @{}
    try {
        foreach ($name in $requiredNames) {
            $destination = Join-Path $runtimeRoot $name
            if ([IO.Directory]::Exists($destination)) {
                throw "Runtime artifact path is a directory: $destination"
            }
            if ([IO.File]::Exists($destination)) {
                $destinationAttributes = [IO.File]::GetAttributes($destination)
                if (($destinationAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                    throw "Runtime artifact is a reparse point: $destination"
                }
            }
            $partial = Join-Path $runtimeRoot ('.{0}.{1}.partial' -f $name, [Guid]::NewGuid())
            [IO.File]::WriteAllBytes($partial, [byte[]] $verified[$name].Bytes)
            $partialPaths[$name] = $partial
        }

        Protect-RuntimeTree -Root $runtimeRoot
        foreach ($name in $requiredNames) {
            $destination = Join-Path $runtimeRoot $name
            if ([IO.File]::Exists($destination)) {
                [IO.File]::Replace([string] $partialPaths[$name], $destination, $null)
            }
            else {
                [IO.File]::Move([string] $partialPaths[$name], $destination)
            }
            $partialPaths.Remove($name)
        }
        Protect-RuntimeTree -Root $runtimeRoot

        $dllPath = Join-Path $runtimeRoot 'VCamSource.dll'
        $registrarPath = Join-Path $runtimeRoot 'VCamRegistrar.exe'
        $pumpPath = Join-Path $runtimeRoot 'vcam-pump.exe'
        $verifyPath = Join-Path $runtimeRoot 'vcam-verify.exe'
        $regsvr32 = Join-Path $env:SystemRoot 'System32\regsvr32.exe'
        $registration = Start-Process -FilePath $regsvr32 `
            -ArgumentList @('/s', ('"{0}"' -f $dllPath)) `
            -Wait -PassThru
        if ($registration.ExitCode -ne 0) {
            throw "regsvr32 failed with exit $($registration.ExitCode)"
        }

        $registrar = Start-Process -FilePath $registrarPath `
            -ArgumentList @('/headless', '/replace', '1280', '720', '30', '1') `
            -WindowStyle Hidden -PassThru
        Start-Sleep -Seconds 3
        if ($registrar.HasExited) {
            throw "VCamRegistrar exited with code $($registrar.ExitCode)"
        }

        $pumpLog = Join-Path $runtimeRoot 'pump.log'
        $pumpError = Join-Path $runtimeRoot 'pump.err'
        $pump = Start-Process -FilePath $pumpPath `
            -ArgumentList @('10', '1280', '720', '30') `
            -RedirectStandardOutput $pumpLog `
            -RedirectStandardError $pumpError `
            -WindowStyle Hidden -PassThru
        Start-Sleep -Seconds 2
        $verification = Start-Process -FilePath $verifyPath -Wait -PassThru
        $pump.WaitForExit()
        if ($verification.ExitCode -ne 0 -or $pump.ExitCode -ne 0) {
            throw "Media Foundation verification failed: verify=$($verification.ExitCode), pump=$($pump.ExitCode)"
        }
        Protect-RuntimeTree -Root $runtimeRoot
        Write-Host 'PASS: VCam is streaming injected NV12 frames'
    }
    finally {
        foreach ($partial in @($partialPaths.Values)) {
            if ([IO.File]::Exists([string] $partial)) {
                [IO.File]::Delete([string] $partial)
            }
        }
    }
}

Install-Runtime -ArtifactSpecs $Artifacts
