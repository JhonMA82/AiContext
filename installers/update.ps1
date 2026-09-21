# Remote updater for aicontext (PowerShell).
# Read-only with -Check; mutating updates require -Yes.
param(
  [switch]$Check,
  [string]$Version = "latest",
  [string]$InstallDir = "",
  [switch]$Yes,
  [switch]$NoCargoFallback,
  [switch]$Help
)

$ErrorActionPreference = "Stop"
$App = "aicontext"
$Repo = "JhonMA82/AiContext"

function Show-Usage {
  Write-Output "Usage: update.ps1 [-Check] [-Version VERSION] [-InstallDir DIR] [-Yes] [-NoCargoFallback] [-Help]"
  Write-Output ""
  Write-Output "  -Check   Compare versions only; never change anything."
  Write-Output "  -Yes     Required to actually update (refused without it)."
  Write-Output "Remote usage:"
  Write-Output "  irm https://raw.githubusercontent.com/$Repo/main/installers/update.ps1 | iex"
}

if ($Help) { Show-Usage; exit 0 }

if ([string]::IsNullOrEmpty($InstallDir)) {
  if ($env:AICONTEXT_INSTALL_DIR) { $InstallDir = $env:AICONTEXT_INSTALL_DIR }
  else { $InstallDir = Join-Path $env:USERPROFILE ".cargo\bin" }
}

$CleanVersion = $Version
if ($CleanVersion -ne "latest") { $CleanVersion = $CleanVersion.TrimStart("v") }

if ($Check) {
  $cmd = Get-Command $App -ErrorAction SilentlyContinue
  $localBin = Join-Path $InstallDir "$App.exe"
  if ($cmd) { & $App self update --check; exit $LASTEXITCODE }
  if (Test-Path $localBin) { & $localBin self update --check; exit $LASTEXITCODE }
  Write-Output "$App is not installed; an update would install version: $CleanVersion"
  exit 0
}

if (-not $Yes) {
  Write-Error "refusing to update without -Yes"
  exit 2
}

# Prefer the managed path first; fall back to the remote installer.
$managedOk = $false
$cmd = Get-Command $App -ErrorAction SilentlyContinue
if ($cmd) {
  & $App self update --yes
  if ($LASTEXITCODE -eq 0) { $managedOk = $true }
}
if (-not $managedOk) {
  if ($CleanVersion -eq "latest") {
    $url = "https://github.com/$Repo/releases/latest/download/$App-installer.ps1"
  } else {
    $url = "https://github.com/$Repo/releases/download/v$CleanVersion/$App-installer.ps1"
  }
  Write-Output "downloading official installer: $url"
  $tmp = [System.IO.Path]::GetTempFileName() + ".ps1"
  try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $tmp -TimeoutSec 60
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $env:AICONTEXT_INSTALL_DIR = $InstallDir
    & powershell -ExecutionPolicy Bypass -File $tmp
    if ($LASTEXITCODE -ne 0) { throw "official installer exited with code $LASTEXITCODE" }
    Write-Output "updated $App into $InstallDir"
  } catch {
    if ((-not $NoCargoFallback) -and (Get-Command cargo -ErrorAction SilentlyContinue)) {
      if ($CleanVersion -eq "latest") { & cargo install $App --locked }
      else { & cargo install $App --locked --version $CleanVersion }
      if ($LASTEXITCODE -ne 0) { Write-Error "update failed"; exit 5 }
    } else {
      Write-Error "update failed: $($_.Exception.Message)"
      exit 5
    }
  } finally {
    if (Test-Path $tmp) { Remove-Item -Force $tmp -ErrorAction SilentlyContinue }
  }
}
