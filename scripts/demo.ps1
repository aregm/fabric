[CmdletBinding()]
param(
    [ValidateSet("up", "down", "reset-keys")]
    [string]$Command = "up",
    [string]$Alice = "claude",
    [string]$Bob = "claude"
)

# Builds the demo and runs `fabric-demo up`, `down` or `reset-keys` from the repository root.
# A backend is claude, ollama:<model> or scripted (tests only).

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Run = Join-Path $PSScriptRoot "run.ps1"
$Exe = Join-Path $Root "target\release\fabric-demo.exe"
# `down` and `reset-keys` use the binary `up` built: Windows locks a running executable, so
# rebuilding it while the demo runs would fail.
if ($Command -eq "up" -or -not (Test-Path -LiteralPath $Exe)) {
    & $Run cargo build -p fabric-demo --release
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

switch ($Command) {
    "up" { & $Exe up --alice $Alice --bob $Bob --repository $Root }
    "down" { & $Exe down --repository $Root }
    "reset-keys" { & $Exe reset-keys --repository $Root }
}
exit $LASTEXITCODE
