Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Command = @($args)

$environmentNames = @(
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "PATH",
    "RUSTC",
    "RUSTDOC",
    "RUSTUP_AUTO_INSTALL",
    "RUSTUP_DIST_SERVER",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "RUSTUP_UPDATE_ROOT",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTDOC"
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
    $value = [Environment]::GetEnvironmentVariable($name, "Process")
    $previousEnvironment[$name] = @{
        Exists = $null -ne $value
        Value = $value
    }
}

try {
    $Root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
    $Tooling = Join-Path $Root ".tooling"
    $env:CARGO_HOME = Join-Path $Tooling "cargo"
    $env:RUSTUP_HOME = Join-Path $Tooling "rustup"
    $env:CARGO_TARGET_DIR = Join-Path $Root "target"
    $env:PATH = "$(Join-Path $env:CARGO_HOME 'bin');$env:PATH"
    $env:RUSTC = Join-Path $env:CARGO_HOME "bin\rustc.exe"
    $env:RUSTDOC = Join-Path $env:CARGO_HOME "bin\rustdoc.exe"
    $env:RUSTUP_AUTO_INSTALL = "0"
    $env:RUSTUP_DIST_SERVER = "https://static.rust-lang.org"
    $env:RUSTUP_UPDATE_ROOT = "https://static.rust-lang.org/rustup"

    foreach ($name in @(
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTC",
        "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTDOC"
    )) {
        [Environment]::SetEnvironmentVariable($name, $null, "Process")
    }

    $channelText = Get-Content -Raw -LiteralPath (Join-Path $Root "rust-toolchain.toml")
    $match = [regex]::Match($channelText, '(?m)^\s*channel\s*=\s*"([^"]+)"\s*$')
    if (-not $match.Success) {
        throw "Could not read the pinned channel from rust-toolchain.toml."
    }
    $env:RUSTUP_TOOLCHAIN = $match.Groups[1].Value

    $Cargo = Join-Path $env:CARGO_HOME "bin\cargo.exe"
    if (-not (Test-Path -LiteralPath $Cargo)) {
        $PowerShell = Join-Path $PSHOME $(if ($PSVersionTable.PSEdition -eq "Core") { "pwsh.exe" } else { "powershell.exe" })
        & $PowerShell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "setup.ps1") -SkipVerify
        if ($LASTEXITCODE -ne 0) {
            throw "The isolated Rust setup failed with exit code $LASTEXITCODE."
        }
    }

    if ($Command.Count -eq 0) {
        throw "Usage: .\scripts\run.ps1 <command> [arguments], for example: .\scripts\run.ps1 cargo test --workspace --locked"
    }

    $executable = [string]$Command[0]
    switch ($executable.ToLowerInvariant()) {
        "cargo" { $executable = Join-Path $env:CARGO_HOME "bin\cargo.exe" }
        "rustc" { $executable = Join-Path $env:CARGO_HOME "bin\rustc.exe" }
        "rustdoc" { $executable = Join-Path $env:CARGO_HOME "bin\rustdoc.exe" }
        "rustup" { $executable = Join-Path $env:CARGO_HOME "bin\rustup.exe" }
    }

    [string[]]$arguments = @()
    if ($Command.Count -gt 1) {
        $arguments = $Command[1..($Command.Count - 1)]
    }

    Push-Location $Root
    try {
        $global:LASTEXITCODE = 0
        & $executable @arguments
        $exitCode = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }
}
finally {
    foreach ($name in $environmentNames) {
        $entry = $previousEnvironment[$name]
        if ($entry.Exists) {
            [Environment]::SetEnvironmentVariable($name, $entry.Value, "Process")
        }
        else {
            [Environment]::SetEnvironmentVariable($name, $null, "Process")
        }
    }
}

exit $exitCode
