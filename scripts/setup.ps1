[CmdletBinding()]
param(
    [switch]$SkipVerify
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-External {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter()][string[]]$ArgumentList = @()
    )

    & $FilePath @ArgumentList
    if ($LASTEXITCODE -ne 0) {
        throw ("Command failed with exit code {0}: {1} {2}" -f $LASTEXITCODE, $FilePath, ($ArgumentList -join " "))
    }
}

function Get-ToolchainChannel {
    param([Parameter(Mandatory = $true)][string]$ToolchainFile)

    $contents = Get-Content -Raw -LiteralPath $ToolchainFile
    $match = [regex]::Match($contents, '(?m)^\s*channel\s*=\s*"([^"]+)"\s*$')
    if (-not $match.Success) {
        throw "Could not read a single channel value from $ToolchainFile"
    }
    return $match.Groups[1].Value
}

function Test-WindowsSdk {
    param([Parameter(Mandatory = $true)][string]$Architecture)

    $sdkArchitecture = switch ($Architecture) {
        "X64" { "x64" }
        "Arm64" { "arm64" }
        default { throw "Unsupported Windows SDK architecture '$Architecture'." }
    }

    $sdkRoots = @()
    if (-not [string]::IsNullOrWhiteSpace($env:WindowsSdkDir)) {
        $sdkRoots += $env:WindowsSdkDir
    }

    foreach ($registryPath in @(
        "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows Kits\Installed Roots",
        "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots"
    )) {
        $property = Get-ItemProperty -LiteralPath $registryPath -Name KitsRoot10 -ErrorAction SilentlyContinue
        if ($null -ne $property -and -not [string]::IsNullOrWhiteSpace($property.KitsRoot10)) {
            $sdkRoots += $property.KitsRoot10
        }
    }

    foreach ($sdkRoot in @($sdkRoots | Select-Object -Unique)) {
        $libRoot = Join-Path $sdkRoot "Lib"
        if (-not (Test-Path -LiteralPath $libRoot)) {
            continue
        }

        foreach ($versionDirectory in @(Get-ChildItem -LiteralPath $libRoot -Directory -ErrorAction SilentlyContinue)) {
            $version = $versionDirectory.Name
            $requiredPaths = @(
                (Join-Path $versionDirectory.FullName "um\$sdkArchitecture"),
                (Join-Path $versionDirectory.FullName "ucrt\$sdkArchitecture"),
                (Join-Path $sdkRoot "Include\$version\um"),
                (Join-Path $sdkRoot "Include\$version\ucrt")
            )
            $missingPath = $requiredPaths | Where-Object { -not (Test-Path -LiteralPath $_) } | Select-Object -First 1
            if ($null -eq $missingPath) {
                return $true
            }
        }
    }

    return $false
}

function Test-WindowsBuildTools {
    param([Parameter(Mandatory = $true)][string]$Architecture)

    $vcComponent = switch ($Architecture) {
        "X64" { "Microsoft.VisualStudio.Component.VC.Tools.x86.x64" }
        "Arm64" { "Microsoft.VisualStudio.Component.VC.Tools.ARM64" }
        default { throw "Unsupported Visual C++ tools architecture '$Architecture'." }
    }

    $hasCompiler = $null -ne (Get-Command link.exe -ErrorAction SilentlyContinue)

    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    $vswhereCandidates = @()
    if (-not [string]::IsNullOrWhiteSpace($programFilesX86)) {
        $vswhereCandidates += Join-Path $programFilesX86 "Microsoft Visual Studio\Installer\vswhere.exe"
    }
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $vswhereCandidates += Join-Path $env:ProgramFiles "Microsoft Visual Studio\Installer\vswhere.exe"
    }

    if (-not $hasCompiler) {
        foreach ($candidate in $vswhereCandidates) {
            if (-not (Test-Path -LiteralPath $candidate)) {
                continue
            }

            $installation = & $candidate -latest -products * -requires $vcComponent -property installationPath
            if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installation)) {
                $hasCompiler = $true
                break
            }
        }
    }

    $hasSdk = Test-WindowsSdk -Architecture $Architecture
    if (-not $hasCompiler -or -not $hasSdk) {
        $missing = @()
        if (-not $hasCompiler) { $missing += "the $vcComponent toolset" }
        if (-not $hasSdk) { $missing += "a Windows SDK with $($Architecture.ToLowerInvariant()) libraries" }
        throw "Required native Windows build prerequisites were not detected: $($missing -join ', '). Install Visual Studio Build Tools with Desktop development with C++ and a Windows SDK."
    }
}

$Root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$Tooling = Join-Path $Root ".tooling"
$CargoHome = Join-Path $Tooling "cargo"
$RustupHome = Join-Path $Tooling "rustup"
$Bootstrap = Join-Path $Tooling "bootstrap"
$TargetDir = Join-Path $Root "target"

New-Item -ItemType Directory -Force -Path $CargoHome, $RustupHome, $Bootstrap, $TargetDir | Out-Null

$env:CARGO_HOME = $CargoHome
$env:RUSTUP_HOME = $RustupHome
$env:CARGO_TARGET_DIR = $TargetDir
$env:PATH = "$(Join-Path $CargoHome 'bin');$env:PATH"
$env:RUSTUP_INIT_SKIP_PATH_CHECK = "yes"
$env:RUSTUP_DIST_SERVER = "https://static.rust-lang.org"
$env:RUSTUP_UPDATE_ROOT = "https://static.rust-lang.org/rustup"
Remove-Item Env:RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue
foreach ($name in @(
    "RUSTC",
    "RUSTDOC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTDOC"
)) {
    [Environment]::SetEnvironmentVariable($name, $null, "Process")
}

$RustupVersion = "1.29.0"
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($architecture) {
    "X64" {
        $hostTarget = "x86_64-pc-windows-msvc"
        $expectedHash = "86478e53f769379d7f0ebfa7c9aa97cb76ca92233f79aa2cc0dbee2efaac73c7"
    }
    "Arm64" {
        $hostTarget = "aarch64-pc-windows-msvc"
        $expectedHash = "3af309e6c3062aa11df0e932954f69d13b734d8a431e593812f3ecd9ff9e6ef6"
    }
    default {
        throw "Unsupported Windows architecture '$architecture'. Supported: x86_64 and arm64."
    }
}

Test-WindowsBuildTools -Architecture $architecture

$Rustup = Join-Path $CargoHome "bin\rustup.exe"
if (-not (Test-Path -LiteralPath $Rustup)) {
    $RustupInit = Join-Path $Bootstrap "rustup-init-$RustupVersion-$hostTarget.exe"
    if (-not (Test-Path -LiteralPath $RustupInit)) {
        $download = "$RustupInit.download"
        $url = "https://static.rust-lang.org/rustup/archive/$RustupVersion/$hostTarget/rustup-init.exe"
        Write-Host "Downloading pinned rustup $RustupVersion for $hostTarget..."
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $download
        Move-Item -Force -LiteralPath $download -Destination $RustupInit
    }

    $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $RustupInit).Hash.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "rustup-init checksum mismatch. Expected $expectedHash, received $actualHash. Delete $RustupInit and retry only after investigating."
    }

    Invoke-External -FilePath $RustupInit -ArgumentList @(
        "-y",
        "--no-modify-path",
        "--default-toolchain", "none",
        "--profile", "minimal"
    )
}

$env:RUSTUP_AUTO_INSTALL = "0"
$rustupVersionOutput = & $Rustup --version 2>$null | Where-Object { $_ -match "^rustup " } | Select-Object -First 1
if ([string]::IsNullOrWhiteSpace($rustupVersionOutput) -or $rustupVersionOutput -notmatch "rustup $([regex]::Escape($RustupVersion))") {
    throw "Expected project-local rustup $RustupVersion, received: $rustupVersionOutput"
}

$channel = Get-ToolchainChannel -ToolchainFile (Join-Path $Root "rust-toolchain.toml")
Invoke-External -FilePath $Rustup -ArgumentList @(
    "toolchain", "install", $channel,
    "--profile", "minimal",
    "--component", "clippy",
    "--component", "rustfmt",
    "--component", "rust-analyzer",
    "--component", "rust-src"
)

$env:RUSTUP_TOOLCHAIN = $channel

$Cargo = Join-Path $CargoHome "bin\cargo.exe"
$Rustc = Join-Path $CargoHome "bin\rustc.exe"
$env:RUSTC = $Rustc
$env:RUSTDOC = Join-Path $CargoHome "bin\rustdoc.exe"

$reportedHome = (& $Rustup show home | Select-Object -Last 1).Trim()
if ($LASTEXITCODE -ne 0 -or -not $reportedHome.Equals($RustupHome, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Isolation check failed: rustup home is '$reportedHome', expected '$RustupHome'."
}

$sysroot = (& $Rustc --print sysroot | Select-Object -Last 1).Trim()
if ($LASTEXITCODE -ne 0 -or -not $sysroot.StartsWith($RustupHome, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Isolation check failed: rustc sysroot '$sysroot' is outside '$RustupHome'."
}

Push-Location $Root
try {
    if (-not (Test-Path -LiteralPath (Join-Path $Root "Cargo.lock"))) {
        Invoke-External -FilePath $Cargo -ArgumentList @("generate-lockfile")
    }

    Invoke-External -FilePath $Cargo -ArgumentList @("fetch", "--locked")

    if (-not $SkipVerify) {
        Invoke-External -FilePath $Cargo -ArgumentList @("fmt", "--all", "--", "--check")
        Invoke-External -FilePath $Cargo -ArgumentList @("check", "--workspace", "--all-targets", "--all-features", "--locked")
        Invoke-External -FilePath $Cargo -ArgumentList @("clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings")
        Invoke-External -FilePath $Cargo -ArgumentList @("test", "--workspace", "--all-targets", "--all-features", "--locked")
        Invoke-External -FilePath $Cargo -ArgumentList @("doc", "--workspace", "--no-deps", "--locked")
    }
}
finally {
    Pop-Location
}

Write-Host ""
Write-Host "Fabric Rust environment is ready."
Write-Host "  rustup: $rustupVersionOutput"
Write-Host "  channel: $channel"
Write-Host "  RUSTUP_HOME: $RustupHome"
Write-Host "  CARGO_HOME: $CargoHome"
Write-Host ""
Write-Host "Run commands without changing your global shell:"
Write-Host "  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo test --workspace --locked"
