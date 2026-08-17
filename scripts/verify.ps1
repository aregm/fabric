[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Run = Join-Path $PSScriptRoot "run.ps1"
$commands = @(
    @("cargo", "fmt", "--all", "--", "--check"),
    @("cargo", "check", "--workspace", "--all-targets", "--all-features", "--locked"),
    @("cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings"),
    @("cargo", "test", "--workspace", "--all-targets", "--all-features", "--locked"),
    @("cargo", "doc", "--workspace", "--no-deps", "--locked")
)

foreach ($command in $commands) {
    & $Run @command
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Write-Host "All Fabric verification checks passed."

