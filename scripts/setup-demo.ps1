[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$IsabelleVersion = "Isabelle2025-2"
# The Isabelle site publishes no checksum, so this pins the archive downloaded and approved on 2026-10-18.
$ArchiveBytes = 1069837790
$ArchiveSha256 = "51f47f3259f168de4517cf6e46e90409ab2c45ab4651d5b793509056571379c5"
# isabelle.in.tum.de/dist/ redirects to plain HTTP on this mirror; the mirror also serves the file over HTTPS.
$ArchiveUrl = "https://dist.isabelle.cit.tum.de/dist/$IsabelleVersion.exe?token=Isabelle"

$Root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$IsabelleTooling = Join-Path $Root ".tooling\isabelle"
$DownloadDir = Join-Path $IsabelleTooling "download"
$Archive = Join-Path $DownloadDir "$IsabelleVersion.exe"
$IsabelleHome = Join-Path $IsabelleTooling $IsabelleVersion
$Uninitialized = Join-Path $IsabelleHome "contrib\cygwin\isabelle\uninitialized"

New-Item -ItemType Directory -Force -Path $DownloadDir | Out-Null

if (-not (Test-Path -LiteralPath $IsabelleHome)) {
    if (-not (Test-Path -LiteralPath $Archive)) {
        $partial = "$Archive.part"
        Write-Host "Downloading $IsabelleVersion for Windows (about 1 GB; rerun this script to resume)..."
        & (Join-Path $env:SystemRoot "System32\curl.exe") --fail --location --retry 5 --retry-delay 10 --retry-all-errors --continue-at - --silent --show-error --output $partial $ArchiveUrl
        if ($LASTEXITCODE -ne 0) {
            throw "Downloading $IsabelleVersion failed with exit code $LASTEXITCODE. Rerun this script to resume."
        }
        Move-Item -Force -LiteralPath $partial -Destination $Archive
    }

    $actualBytes = (Get-Item -LiteralPath $Archive).Length
    $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Archive).Hash.ToLowerInvariant()
    if ($actualBytes -ne $ArchiveBytes -or $actualHash -ne $ArchiveSha256) {
        throw "$IsabelleVersion archive mismatch. Expected $ArchiveBytes bytes with SHA-256 $ArchiveSha256, received $actualBytes bytes with $actualHash. Delete $Archive only after investigating."
    }

    # Windows' own tar reads the self-extracting 7-Zip archive without running it,
    # so no desktop shortcut is created and no window opens.
    Write-Host "Unpacking $IsabelleVersion into .tooling\isabelle..."
    & (Join-Path $env:SystemRoot "System32\tar.exe") -xf $Archive -C $IsabelleTooling
    if ($LASTEXITCODE -ne 0) {
        throw "Unpacking $IsabelleVersion failed with exit code $LASTEXITCODE."
    }
}

if (Test-Path -LiteralPath $Uninitialized) {
    Write-Host "Initializing Isabelle's bundled Cygwin..."
    Start-Process -FilePath (Join-Path $IsabelleHome "$IsabelleVersion.exe") -ArgumentList "-init" -WorkingDirectory $IsabelleHome -Wait | Out-Null
    if (Test-Path -LiteralPath $Uninitialized) {
        throw "$IsabelleVersion initialization did not complete."
    }
}

$reportedVersion = & (Join-Path $PSScriptRoot "isabelle.ps1") version | Select-Object -Last 1
if ($LASTEXITCODE -ne 0 -or "$reportedVersion".Trim() -ne $IsabelleVersion) {
    throw "Expected 'isabelle version' to report $IsabelleVersion, received: $reportedVersion"
}

Write-Host ""
Write-Host "Isabelle for the Fabric demo is ready."
Write-Host "  version: $IsabelleVersion"
Write-Host "  location: $IsabelleHome"
Write-Host ""
Write-Host "Run Isabelle tools without changing your global shell:"
Write-Host "  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\isabelle.ps1 version"
