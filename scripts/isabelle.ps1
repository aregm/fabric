Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Command = @($args)

$environmentNames = @(
    "CHERE_INVOKING",
    "HOME",
    "LANG",
    "PATH",
    "USER_HOME"
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
    $IsabelleTooling = Join-Path $Root ".tooling\isabelle"
    $IsabelleHome = Join-Path $IsabelleTooling "Isabelle2025-2"
    $UserHome = Join-Path $IsabelleTooling "home"
    $Bash = Join-Path $IsabelleHome "contrib\cygwin\bin\bash.exe"

    if (-not (Test-Path -LiteralPath $Bash)) {
        throw "Isabelle is not installed. Run: powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup-demo.ps1"
    }
    if ($Command.Count -eq 0) {
        throw "Usage: .\scripts\isabelle.ps1 <tool> [arguments], for example: .\scripts\isabelle.ps1 version"
    }

    New-Item -ItemType Directory -Force -Path $UserHome | Out-Null

    # On Windows, Isabelle derives its per-user directory from USERPROFILE unless USER_HOME is set.
    # Pointing it into .tooling keeps settings and session heaps inside the repository.
    $env:USER_HOME = "/cygdrive/" + $UserHome.Substring(0, 1).ToLowerInvariant() + $UserHome.Substring(2).Replace("\", "/")
    $env:HOME = $UserHome
    $env:PATH = "$(Join-Path $IsabelleHome 'bin');$env:PATH"
    $env:LANG = "en_US.UTF-8"
    $env:CHERE_INVOKING = "true"

    $quoted = @($Command | ForEach-Object { "'" + ([string]$_).Replace("'", "'\''") + "'" })

    Push-Location $Root
    try {
        $global:LASTEXITCODE = 0
        & $Bash --login -c ("isabelle " + ($quoted -join " "))
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
