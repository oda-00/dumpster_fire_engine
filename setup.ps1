#Requires -Version 5.1
<#
.SYNOPSIS
    Bootstrap script for dumpster_fire_engine dev environment on Windows.
.DESCRIPTION
    Self-elevates to Administrator (required for LLVM and Vulkan SDK installers),
    installs Rust if absent, then delegates everything else to the Rust setup binary:
    LLVM 18, Vulkan SDK, CMake, iai-callgrind-runner, and .env.toolchain.ps1.

    All tools are installed automatically — no manual downloads required.
    Falls back to direct installer download if winget/choco are not present.
.EXAMPLE
    .\setup.ps1
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

# ── 1. Re-launch as Administrator if not already elevated ─────────────────────
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $isAdmin) {
    Write-Host '[setup] Requesting Administrator privileges (required for LLVM and Vulkan SDK install)...'
    $argString = "-NoExit -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    if ($args.Count -gt 0) { $argString += ' ' + ($args -join ' ') }

    # Try pwsh (PowerShell 7) first, fall back to powershell (Windows PowerShell 5.1)
    $shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { 'pwsh' } else { 'powershell' }
    Start-Process $shell -Verb RunAs -ArgumentList $argString
    exit 0
}

Set-Location $ScriptDir

# ── 2. Ensure Rust / cargo is available ───────────────────────────────────────
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host '[setup] cargo not found — installing Rust via rustup...'
    $rustupInit = "$env:TEMP\rustup-init.exe"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $rustupInit
    & $rustupInit -y --no-modify-path
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
}

# ── 3. Run the Rust setup binary ──────────────────────────────────────────────
# The binary handles: LLVM 18, Vulkan SDK, CMake, iai-callgrind-runner,
# writing .env.toolchain.ps1, and verifying cargo check --workspace.
& cargo run --manifest-path "$ScriptDir\tools\setup\Cargo.toml" --quiet -- @args
$exitCode = $LASTEXITCODE

# ── 4. Dot-source the generated env file for the current session ──────────────
$envFile = Join-Path $ScriptDir '.env.toolchain.ps1'
if (Test-Path $envFile) {
    . $envFile
    Write-Host "[setup] Sourced $envFile for this session."
    Write-Host "[setup] Add the following to your PowerShell profile for future sessions:"
    Write-Host "        . `"$envFile`""
}

exit $exitCode
