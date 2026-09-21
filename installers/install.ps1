# Remote installer for aicontext (PowerShell).
# Primary source: official cargo-dist installer published on GitHub Releases.
# Fallback: cargo install from the git repository (the crate is not
# published on crates.io; requires a Rust toolchain).
# Only writes inside the user prefix; never touches project manifests.
param(
  [string]$Version = "latest",
  [string]$InstallDir = "",
  [switch]$NoCargoFallback,
  [switch]$Help
)

$ErrorActionPreference = "Stop"
$App = "aicontext"
$Repo = "JhonMA82/AiContext"

function Show-Usage {
  Write-Output "Usage: install.ps1 [-Version VERSION] [-InstallDir DIR] [-NoCargoFallback] [-Help]"
  Write-Output ""
  Write-Output "Installs $App remotely (idempotent, safe to re-run)."
  Write-Output ""
  Write-Output "  -Version VERSION  Version to install (default: latest). Accepts 'latest', '0.4.0' or 'v0.4.0'."
  Write-Output "  -InstallDir DIR   Binary directory (default: `$env:USERPROFILE\.cargo\bin)."
  Write-Output "Remote usage:"
  Write-Output "  irm https://raw.githubusercontent.com/$Repo/v0.5.0/installers/install.ps1 | iex"
}

if ($Help) { Show-Usage; exit 0 }

if ([string]::IsNullOrEmpty($InstallDir)) {
  if ($env:AICONTEXT_INSTALL_DIR) { $InstallDir = $env:AICONTEXT_INSTALL_DIR }
  else { $InstallDir = Join-Path $env:USERPROFILE ".cargo\bin" }
}

$CleanVersion = $Version
if ($CleanVersion -ne "latest") { $CleanVersion = $CleanVersion.TrimStart("v") }

function Get-InstallerUrl {
  if ($CleanVersion -eq "latest") {
    return "https://github.com/$Repo/releases/latest/download/$App-installer.ps1"
  }
  return "https://github.com/$Repo/releases/download/v$CleanVersion/$App-installer.ps1"
}

function Invoke-OfficialInstaller {
  $url = Get-InstallerUrl
  Write-Output "downloading official installer: $url"
  $tmp = [System.IO.Path]::GetTempFileName() + ".ps1"
  try {
    $prev = [Net.ServicePointManager]::SecurityProtocol
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $tmp -TimeoutSec 60
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $env:AICONTEXT_INSTALL_DIR = $InstallDir
    & powershell -ExecutionPolicy Bypass -File $tmp
    return $true
  } catch {
    Write-Warning "official installer failed: $($_.Exception.Message)"
    return $false
  } finally {
    if (Test-Path $tmp) { Remove-Item -Force $tmp -ErrorAction SilentlyContinue }
  }
}

function Invoke-CargoFallback {
  if ($NoCargoFallback) { return $false }
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { return $false }
  if ($CleanVersion -eq "latest") {
    Write-Output "falling back to: cargo install --git https://github.com/$Repo --locked"
    & cargo install --git "https://github.com/$Repo" --locked
  } else {
    Write-Output "falling back to: cargo install --git https://github.com/$Repo --tag v$CleanVersion --locked"
    & cargo install --git "https://github.com/$Repo" --tag "v$CleanVersion" --locked
  }
  return ($LASTEXITCODE -eq 0)
}

$installed = Invoke-OfficialInstaller
if (-not $installed) { $installed = Invoke-CargoFallback }

if (-not $installed) {
  Write-Error "install failed: official installer unreachable and no cargo fallback available"
  exit 5
}

$bin = Join-Path $InstallDir "$App.exe"
if ((Test-Path $bin) -or (Get-Command $App -ErrorAction SilentlyContinue)) {
  Write-Output "installed $App into $InstallDir"
  Write-Output "done. Next: $App status"
} else {
  Write-Error "warning: installer ran but no $App binary was found in $InstallDir"
  exit 5
}
