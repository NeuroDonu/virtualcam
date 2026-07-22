[CmdletBinding()]
param(
    [string]$PackagesDirectory,
    [string[]]$SourceTemplates = @(
        "https://www.nuget.org/api/v2/package/{id}/{version}",
        "https://api.nuget.org/v3-flatcontainer/{id-lower}/{version-lower}/{id-lower}.{version-lower}.nupkg"
    ),
    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ([string]::IsNullOrWhiteSpace($PackagesDirectory)) {
    $PackagesDirectory = Join-Path $PSScriptRoot "..\cpp\VCam\packages"
}

# Deterministic package bootstrap, independent from MSBuild/NuGet restore.
$Packages = @(
    @{
        Id = "Microsoft.Windows.CppWinRT"
        Version = "3.0.260520.1"
        Sha256 = "d22e2e26133d63217ae26e91b1685fb024b03a508a78af645f8347a3126c8435"
        RequiredFiles = @(
            "bin\cppwinrt.exe",
            "build\native\Microsoft.Windows.CppWinRT.props",
            "build\native\Microsoft.Windows.CppWinRT.targets"
        )
    },
    @{
        Id = "Microsoft.Windows.ImplementationLibrary"
        Version = "1.0.260126.7"
        Sha256 = "7d67bc71dd0edc342db44ea0334ac9b6a1776514f81694b0b5b3c7ac9d608e42"
        RequiredFiles = @(
            "include\wil\result.h",
            "build\native\Microsoft.Windows.ImplementationLibrary.targets"
        )
    }
)

function Test-PackageLayout {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][string[]]$RequiredFiles
    )

    foreach ($RelativePath in $RequiredFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $Directory $RelativePath) -PathType Leaf)) {
            return $false
        }
    }
    return $true
}

function Assert-ProjectManifests {
    $ManifestPaths = @(
        (Join-Path $PSScriptRoot "..\cpp\VCam\Registrar\packages.config"),
        (Join-Path $PSScriptRoot "..\cpp\VCam\MediaSource\packages.config")
    )

    foreach ($ManifestPath in $ManifestPaths) {
        if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
            throw "Package manifest not found: $ManifestPath"
        }

        [xml]$Manifest = Get-Content -LiteralPath $ManifestPath -Raw
        $Actual = @($Manifest.packages.package | ForEach-Object { "$($_.id)@$($_.version)" } | Sort-Object)
        $Expected = @($Packages | ForEach-Object { "$($_.Id)@$($_.Version)" } | Sort-Object)
        if (($Actual -join "|") -ne ($Expected -join "|")) {
            throw "Pinned packages do not match $ManifestPath. Expected: $($Expected -join ', '); found: $($Actual -join ', ')"
        }
    }
}

function Download-File {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    $Curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($null -eq $Curl) {
        throw "curl.exe is required. It is included with supported Windows 10 and Windows 11 releases."
    }

    & $Curl.Source -4 --fail --location --retry 3 --retry-all-errors `
        --connect-timeout 15 --max-time 300 --silent --show-error `
        --user-agent "VCam-NuGet-Bootstrap/1.0" --output $Destination $Uri
    if ($LASTEXITCODE -ne 0) {
        throw "curl.exe failed with exit code $LASTEXITCODE while downloading $Uri"
    }
}

function Download-Package {
    param(
        [Parameter(Mandatory = $true)][hashtable]$Package,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    $IdLower = $Package.Id.ToLowerInvariant()
    $VersionLower = $Package.Version.ToLowerInvariant()
    foreach ($Template in $SourceTemplates) {
        $Uri = $Template.Replace("{id-lower}", $IdLower).
            Replace("{version-lower}", $VersionLower).
            Replace("{id}", $Package.Id).
            Replace("{version}", $Package.Version)
        Write-Host "[download] $Uri"
        try {
            Download-File -Uri $Uri -Destination $Destination
            return
        }
        catch {
            Write-Warning $_.Exception.Message
            Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
        }
    }

    throw "All package sources failed for $($Package.Id) $($Package.Version)"
}

Assert-ProjectManifests
$PackagesDirectory = [System.IO.Path]::GetFullPath($PackagesDirectory)
[System.IO.Directory]::CreateDirectory($PackagesDirectory) | Out-Null
$TemporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("vcam-nuget-" + [Guid]::NewGuid().ToString("N"))
[System.IO.Directory]::CreateDirectory($TemporaryRoot) | Out-Null

try {
    foreach ($Package in $Packages) {
        $FolderName = "$($Package.Id).$($Package.Version)"
        $TargetDirectory = Join-Path $PackagesDirectory $FolderName
        if (-not $Force -and (Test-PackageLayout -Directory $TargetDirectory -RequiredFiles $Package.RequiredFiles)) {
            Write-Host "[cached] $FolderName"
            continue
        }

        $IdLower = $Package.Id.ToLowerInvariant()
        $VersionLower = $Package.Version.ToLowerInvariant()
        # Expand-Archive in Windows PowerShell 5.1 only accepts a .zip suffix.
        # A NuGet .nupkg is a ZIP archive, so retain the bytes under that suffix.
        $PackageFile = Join-Path $TemporaryRoot "$IdLower.$VersionLower.zip"
        $ExtractedDirectory = Join-Path $TemporaryRoot $FolderName
        Download-Package -Package $Package -Destination $PackageFile
        $ActualHash = (Get-FileHash -LiteralPath $PackageFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($ActualHash -ne $Package.Sha256) {
            throw "SHA-256 mismatch for $FolderName. Expected $($Package.Sha256), got $ActualHash"
        }

        Expand-Archive -LiteralPath $PackageFile -DestinationPath $ExtractedDirectory -Force
        if (-not (Test-PackageLayout -Directory $ExtractedDirectory -RequiredFiles $Package.RequiredFiles)) {
            throw "Package $FolderName does not contain the expected native build files"
        }

        if (Test-Path -LiteralPath $TargetDirectory) {
            Remove-Item -LiteralPath $TargetDirectory -Recurse -Force
        }
        Move-Item -LiteralPath $ExtractedDirectory -Destination $TargetDirectory
        Write-Host "[installed] $TargetDirectory"
    }
}
finally {
    if (Test-Path -LiteralPath $TemporaryRoot) {
        Remove-Item -LiteralPath $TemporaryRoot -Recurse -Force
    }
}

Write-Host "NuGet package bootstrap completed."
