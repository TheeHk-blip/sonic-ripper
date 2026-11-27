# Install sonic-cli, its sidecars, and the ytm-* Python helpers for the current user.
$ErrorActionPreference = 'Stop'

$Repo = if ($env:SONIC_REPO) { $env:SONIC_REPO } else { 'TheeHk-blip/sonic-ripper' }
$Version = if ($env:SONIC_VERSION) { $env:SONIC_VERSION } else { 'v1.0.7' }
$SidecarTag = if ($env:SONIC_SIDECAR_TAG) { $env:SONIC_SIDECAR_TAG } else { 'sidecar-binaries-v1' }
$ToolsDir = if ($env:SONIC_TOOLS_DIR) { $env:SONIC_TOOLS_DIR } else { Join-Path $env:USERPROFILE '.sonic-cli' }
$BinDir = if ($env:SONIC_BIN_DIR) { $env:SONIC_BIN_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }

$PyScripts = @(
  'ytm-artist-batch.py'
  'ytm_config.py'
  'ytm_index.py'
  'ytm-find-albums.py'
  'ytm-validate-albums.py'
  'ytm-download-albums.py'
  'ytm-resolve-singles.py'
  'ytm-download-singles.py'
  'wiki_discovery.py'
  'progress_ui.py'
  'checkbox_select.py'
  'opus_settags.py'
  'opus_retag.py'
  'normalize_ampersand_tags.py'
)

function Write-Step([string]$Message) {
  Write-Host "==> $Message" -ForegroundColor Yellow
}

function Get-Target {
  switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { return 'x86_64-pc-windows-msvc' }
    'ARM64' { return 'aarch64-pc-windows-msvc' }
    default { throw "Unsupported Windows architecture: $env:PROCESSOR_ARCHITECTURE" }
  }
}

function Download-File([string]$Url, [string]$Destination) {
  $tmp = "$Destination.part"
  if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) {
    throw 'curl.exe is required (Windows 10+ includes it in System32)'
  }
  & curl.exe -fsSL --retry 3 --proto '=https' --tlsv1.2 -o $tmp $Url
  if ($LASTEXITCODE -ne 0) { throw "Failed to download $Url" }
  Move-Item -Force $tmp $Destination
}

function Get-RawUrl([string]$Script) {
  return "https://raw.githubusercontent.com/${Repo}/${Version}/crates/sonic-cli/src/${Script}"
}

function Get-ReleaseUrl([string]$Tag, [string]$Asset) {
  return "https://github.com/${Repo}/releases/download/${Tag}/${Asset}"
}

function Add-ToUserPath([string]$Directory) {
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $entries = @()
  if ($userPath) { $entries = $userPath -split ';' | Where-Object { $_ } }
  if ($entries -notcontains $Directory) {
    $entries += $Directory
    [Environment]::SetEnvironmentVariable('Path', ($entries -join ';'), 'User')
  }
  if ($env:Path -notlike "*$Directory*") { $env:Path = "$Directory;$env:Path" }
}

$Target = Get-Target

Write-Step "Installing Sonic CLI tools to $ToolsDir"
New-Item -ItemType Directory -Force -Path $ToolsDir, $BinDir | Out-Null

Write-Step "Downloading Python helpers from $Version"
foreach ($script in $PyScripts) {
  Download-File (Get-RawUrl $script) (Join-Path $ToolsDir $script)
}

Write-Step "Downloading sonic-cli for $Target"
try {
  Download-File (Get-ReleaseUrl $Version "sonic-cli-$Target.exe") (Join-Path $ToolsDir 'sonic-cli.exe')
} catch {
  throw "No prebuilt sonic-cli asset for $Target in release $Version. Publish it or set SONIC_VERSION. $($_.Exception.Message)"
}

foreach ($tool in @('sonic-yt-dlp', 'sonic-ffmpeg', 'sonic-deno')) {
  Write-Step "Downloading $tool for $Target"
  Download-File (Get-ReleaseUrl $SidecarTag "$tool-$Target.exe") (Join-Path $ToolsDir "$tool.exe")
}

$getCatalog = @'
@echo off
setlocal
if not defined SONIC_TOOLS_DIR set "SONIC_TOOLS_DIR=%USERPROFILE%\.sonic-cli"
if not defined SONIC_BINARIES_DIR set "SONIC_BINARIES_DIR=%SONIC_TOOLS_DIR%"
where python >nul 2>nul
if not errorlevel 1 (
  python "%SONIC_TOOLS_DIR%\ytm-artist-batch.py" %*
) else (
  py -3 "%SONIC_TOOLS_DIR%\ytm-artist-batch.py" %*
)
exit /b %ERRORLEVEL%
'@

$cliLauncher = @'
@echo off
setlocal
if not defined SONIC_TOOLS_DIR set "SONIC_TOOLS_DIR=%USERPROFILE%\.sonic-cli"
"%SONIC_TOOLS_DIR%\sonic-cli.exe" %*
exit /b %ERRORLEVEL%
'@

Set-Content -Path (Join-Path $BinDir 'get-catalog.cmd') -Value $getCatalog -Encoding ASCII
Set-Content -Path (Join-Path $BinDir 'sonic-cli.cmd') -Value $cliLauncher -Encoding ASCII

Add-ToUserPath $BinDir

Write-Step "Installed get-catalog and sonic-cli into $BinDir"
Write-Host "  tools directory: $ToolsDir"
Write-Host ''
Write-Host 'Open a new terminal, then set your library/cookies if you have not already:'
Write-Host '  setx SONIC_MUSIC_ROOT "%USERPROFILE%\Music"'
Write-Host '  setx YTM_COOKIE_FILE "%USERPROFILE%\music.youtube.com_cookies.txt"'
Write-Host ''
Write-Host 'Try: get-catalog --artist "Sade" --status-only'
