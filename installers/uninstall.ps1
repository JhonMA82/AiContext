# Remote uninstaller for aicontext (PowerShell).
# Removes only AIContext-owned state; foreign files are reported and left alone.
param(
  [switch]$Managed,
  [string]$InstallDir = "",
  [switch]$Yes,
  [switch]$Help
)

$ErrorActionPreference = "Stop"
$App = "aicontext"

function Show-Usage {
  Write-Output "Usage: uninstall.ps1 [-Managed] [-InstallDir DIR] [-Yes] [-Help]"
  Write-Output ""
  Write-Output "  -Managed  Also remove managed-tools owned by AIContext."
  Write-Output "  -Yes      Required to actually remove anything."
}

if ($Help) { Show-Usage; exit 0 }

if ([string]::IsNullOrEmpty($InstallDir)) {
  if ($env:AICONTEXT_INSTALL_DIR) { $InstallDir = $env:AICONTEXT_INSTALL_DIR }
  else { $InstallDir = Join-Path $env:USERPROFILE ".cargo\bin" }
}

if (-not $Yes) {
  Write-Error "refusing to uninstall without -Yes"
  exit 2
}

$ManagedPrefix = Join-Path $env:USERPROFILE ".local\share\aicontext"

function Test-InsideOwned($Path, $Roots) {
  foreach ($root in $Roots) {
    if ([string]::IsNullOrEmpty($root)) { continue }
    if ($Path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { return $true }
  }
  return $false
}

$roots = @($InstallDir, (Join-Path $env:USERPROFILE ".cargo"), (Join-Path $env:USERPROFILE ".local"), $ManagedPrefix)

# 1. Binary candidates (owned prefixes only).
$candidates = @(
  (Join-Path $InstallDir "$App.exe"),
  (Join-Path $InstallDir $App),
  (Join-Path $ManagedPrefix "bin\$App.exe")
)
$seen = @{}
foreach ($candidate in $candidates) {
  if ($seen.ContainsKey($candidate)) { continue }
  $seen[$candidate] = $true
  if (Test-Path $candidate) {
    if (Test-InsideOwned $candidate $roots) {
      Remove-Item -Force $candidate -ErrorAction SilentlyContinue
      Write-Output "removed binary at $candidate"
    } else {
      Write-Output "left alone: binary at $candidate (outside managed prefix; left untouched)"
    }
  }
}
$onPath = Get-Command $App -ErrorAction SilentlyContinue
if ($onPath -and -not $seen.ContainsKey($onPath.Source)) {
  Write-Output "left alone: binary at $($onPath.Source) (outside managed prefix; left untouched)"
}

# 2. Managed tools registry (owned entries inside the prefix only).
$registry = Join-Path $ManagedPrefix "managed-tools.json"
if (-not $Managed) {
  Write-Output "left alone: managed tools prefix (pass -Managed to remove owned tools)"
} elseif (-not (Test-Path $registry)) {
  Write-Output "left alone: no ownership registry found"
} else {
  try {
    $data = Get-Content $registry -Raw | ConvertFrom-Json
    foreach ($tool in ($data.tools | Where-Object { $_ })) {
      $p = $tool.path
      if ([string]::IsNullOrEmpty($p)) { continue }
      if ((Test-Path $p) -and (Test-InsideOwned $p $roots)) {
        Remove-Item -Force $p -ErrorAction SilentlyContinue
        Write-Output "removed managed tool $($tool.name) at $p"
      } elseif (Test-Path $p) {
        Write-Output "left alone: managed tool $($tool.name) at $p (outside managed prefix; left untouched)"
      }
    }
  } catch {
    Write-Warning "could not parse ownership registry; leaving entries alone: $($_.Exception.Message)"
  }
  Remove-Item -Force $registry -ErrorAction SilentlyContinue
  Write-Output "removed ownership registry at $registry"
}

# 3. Owned agent skill (manifest-gated, like self_update.rs).
$skillDir = Join-Path $env:USERPROFILE ".pi\agent\skills\aicontext-adopt"
$manifest = Join-Path $skillDir ".aicontext-managed.json"
if (Test-Path $manifest) {
  Remove-Item -Force (Join-Path $skillDir "SKILL.md") -ErrorAction SilentlyContinue
  Remove-Item -Force $manifest -ErrorAction SilentlyContinue
  Write-Output "removed owned skill files in $skillDir"
  try {
    if ((Get-ChildItem $skillDir -Force -ErrorAction Stop | Measure-Object).Count -eq 0) {
      Remove-Item -Force $skillDir
      Write-Output "removed skill dir $skillDir"
    } else {
      Write-Output "left alone: skill dir $skillDir (foreign files remain; left untouched)"
    }
  } catch {
    Write-Output "left alone: skill dir $skillDir (foreign files remain; left untouched)"
  }
} elseif (Test-Path (Join-Path $skillDir "SKILL.md")) {
  Write-Output "left alone: skill dir exists without an AIContext ownership manifest; left untouched"
} else {
  Write-Output "left alone: no owned agent skills found"
}

Write-Output "only AIContext-owned state was touched; foreign files, project manifests and .engineering/ dirs were left alone."
