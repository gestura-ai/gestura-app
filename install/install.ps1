<#
.SYNOPSIS
  Gestura installer (Windows)

.DESCRIPTION
  Installs Gestura from GitHub Releases in one of two modes:
    - full (default): installs GUI + CLI via MSI
    - cli: installs only the CLI via zip (or legacy single .exe)

.EXAMPLE
  iwr -useb https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.ps1 | iex

.EXAMPLE
  iwr -useb https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.ps1 | iex; Install-Gestura -Mode cli
#>

[CmdletBinding()]
param(
  [ValidateSet('full','cli')]
  [string]$Mode = 'full',

  [string]$Tag,

  [string]$Repo = 'gestura-ai/gestura-app',

  [ValidateSet('x86_64')]
  [string]$Arch = 'x86_64',

  [string]$InstallDir,

  [switch]$NoVerify,
  [switch]$RequireVerify,
  [switch]$DryRun
)

function Write-Log {
  <#
  .SYNOPSIS
    Writes a prefixed log line to stderr.
  #>
  param([Parameter(Mandatory=$true)][string]$Message)
  [Console]::Error.WriteLine("[gestura-install] $Message")
}

function Get-LatestTag {
  <#
  .SYNOPSIS
    Returns the latest GitHub Release tag for the configured repo.
  #>
  param([Parameter(Mandatory=$true)][string]$Repo)
  $url = "https://api.github.com/repos/$Repo/releases/latest"
  $r = Invoke-RestMethod -Uri $url -Method Get
  if (-not $r.tag_name) { throw "Could not parse tag_name from $url" }
  return [string]$r.tag_name
}

function Get-DownloadUrl {
  <#
  .SYNOPSIS
    Computes the GitHub Releases download URL for an asset.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$Repo,
    [Parameter(Mandatory=$true)][string]$Tag,
    [Parameter(Mandatory=$true)][string]$AssetName
  )
  return "https://github.com/$Repo/releases/download/$Tag/$AssetName"
}

function Download-File {
  <#
  .SYNOPSIS
    Downloads a URL to a destination path.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$Url,
    [Parameter(Mandatory=$true)][string]$DestPath
  )
  Write-Log "Downloading $Url"
  Invoke-WebRequest -Uri $Url -OutFile $DestPath -UseBasicParsing -ErrorAction Stop | Out-Null
}

function Try-DownloadAssets {
  <#
  .SYNOPSIS
    Attempts to download the first available asset from GitHub Releases.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$Repo,
    [Parameter(Mandatory=$true)][string]$Tag,
    [Parameter(Mandatory=$true)][string]$WorkDir,
    [Parameter(Mandatory=$true)][string[]]$AssetNames
  )
  foreach ($asset in $AssetNames) {
    $dest = Join-Path $WorkDir $asset
    $url = Get-DownloadUrl -Repo $Repo -Tag $Tag -AssetName $asset
    try {
      Download-File -Url $url -DestPath $dest
      return $dest
    } catch {
      continue
    }
  }
  return $null
}

function Get-ChecksumMap {
  <#
  .SYNOPSIS
    Downloads and parses the release SHA256SUMS file into a hashtable.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$Repo,
    [Parameter(Mandatory=$true)][string]$Tag,
    [Parameter(Mandatory=$true)][string]$WorkDir,
    [switch]$Require
  )
  $name = "gestura-$Tag-SHA256SUMS.txt"
  $path = Join-Path $WorkDir $name
  $url = Get-DownloadUrl -Repo $Repo -Tag $Tag -AssetName $name

  try {
    Download-File -Url $url -DestPath $path
  } catch {
    if ($Require) { throw "Checksum file missing and -RequireVerify set" }
    Write-Log "WARN: checksum file not available for $Tag; proceeding without verification"
    return $null
  }

  $map = @{}
  foreach ($line in Get-Content -Path $path) {
    if ($line -match '^[0-9a-fA-F]{64}\s{2,}(.+)$') {
      $hash = $line.Split(' ', 2)[0]
      $file = ($line -replace '^[0-9a-fA-F]{64}\s{2,}', '')
      $map[$file] = $hash
    }
  }
  return $map
}

function Maybe-Verify {
  <#
  .SYNOPSIS
    Verifies a downloaded asset against the release SHA256SUMS file.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$Repo,
    [Parameter(Mandatory=$true)][string]$Tag,
    [Parameter(Mandatory=$true)][string]$WorkDir,
    [Parameter(Mandatory=$true)][string]$AssetPath,
    [switch]$NoVerify,
    [switch]$RequireVerify
  )
  if ($NoVerify) {
    Write-Log "Skipping verification (-NoVerify)"
    return
  }

  $map = Get-ChecksumMap -Repo $Repo -Tag $Tag -WorkDir $WorkDir -Require:$RequireVerify
  if (-not $map) { return }

  $name = Split-Path -Leaf $AssetPath
  if (-not $map.ContainsKey($name)) {
    if ($RequireVerify) { throw "Checksum entry for $name missing and -RequireVerify set" }
    Write-Log "WARN: checksum entry for $name missing; proceeding without verification"
    return
  }

  $expected = $map[$name]
  $actual = (Get-FileHash -Path $AssetPath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expected.ToLowerInvariant()) {
    throw "Checksum mismatch for $name: expected $expected, got $actual"
  }
  Write-Log "Verified SHA-256 for $name"
}

function Get-DefaultInstallDir {
  <#
  .SYNOPSIS
    Returns the default CLI install directory.
  #>
  param([string]$InstallDir)
  if ($InstallDir) { return $InstallDir }
  return (Join-Path $env:LOCALAPPDATA 'Gestura\bin')
}

function Install-FullMsi {
  <#
  .SYNOPSIS
    Installs the full MSI silently.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$MsiPath,
    [switch]$DryRun
  )
  $args = "/i `"$MsiPath`" /qn /norestart"
  if ($DryRun) {
    Write-Log "DRY RUN: would run: msiexec.exe $args"
    return
  }
  Start-Process -FilePath 'msiexec.exe' -ArgumentList $args -Wait -NoNewWindow
}

function Install-CliFromZip {
  <#
  .SYNOPSIS
    Installs the CLI from a zip archive.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$ZipPath,
    [Parameter(Mandatory=$true)][string]$DestDir,
    [switch]$DryRun
  )
  if ($DryRun) {
    Write-Log "DRY RUN: would extract $ZipPath and copy gestura.exe -> $DestDir"
    return
  }

  New-Item -ItemType Directory -Path $DestDir -Force | Out-Null
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
  New-Item -ItemType Directory -Path $tmp -Force | Out-Null
  Expand-Archive -Path $ZipPath -DestinationPath $tmp -Force
  $exe = Join-Path $tmp 'gestura.exe'
  if (-not (Test-Path $exe)) { throw "Archive did not contain gestura.exe" }
  Copy-Item -Path $exe -Destination (Join-Path $DestDir 'gestura.exe') -Force
  Write-Log "Installed CLI to $(Join-Path $DestDir 'gestura.exe')"
  Write-Log "Note: ensure $DestDir is on PATH (User Environment Variables)"
}

function Install-CliLegacyExe {
  <#
  .SYNOPSIS
    Installs the CLI from the legacy single-exe asset.
  #>
  param(
    [Parameter(Mandatory=$true)][string]$ExePath,
    [Parameter(Mandatory=$true)][string]$DestDir,
    [switch]$DryRun
  )
  if ($DryRun) {
    Write-Log "DRY RUN: would copy $ExePath -> $DestDir\\gestura.exe"
    return
  }
  New-Item -ItemType Directory -Path $DestDir -Force | Out-Null
  Copy-Item -Path $ExePath -Destination (Join-Path $DestDir 'gestura.exe') -Force
  Write-Log "Installed CLI to $(Join-Path $DestDir 'gestura.exe')"
}

function Install-Gestura {
  <#
  .SYNOPSIS
    Main installer entrypoint.
  #>
  param(
    [ValidateSet('full','cli')][string]$Mode = 'full',
    [string]$Tag,
    [string]$Repo = 'gestura-ai/gestura-app',
    [string]$InstallDir,
    [switch]$NoVerify,
    [switch]$RequireVerify,
    [switch]$DryRun
  )

  if ($PSVersionTable.PSEdition -ne 'Desktop' -and $IsWindows -ne $true) {
    throw "This script is intended to run on Windows."
  }

  $resolvedTag = $Tag
  if (-not $resolvedTag) {
    $resolvedTag = Get-LatestTag -Repo $Repo
  }
  Write-Log "Using release tag: $resolvedTag"

  $workDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
  New-Item -ItemType Directory -Path $workDir -Force | Out-Null

  if ($Mode -eq 'cli') {
    $dest = Get-DefaultInstallDir -InstallDir $InstallDir
    $assetPath = Try-DownloadAssets -Repo $Repo -Tag $resolvedTag -WorkDir $workDir -AssetNames @(
      "gestura-cli-$resolvedTag-windows-x86_64.zip",
      "gestura-cli-windows-x86_64.exe"
    )
    if (-not $assetPath) { throw "Unable to download CLI asset" }
    Maybe-Verify -Repo $Repo -Tag $resolvedTag -WorkDir $workDir -AssetPath $assetPath -NoVerify:$NoVerify -RequireVerify:$RequireVerify
    if ($assetPath.ToLowerInvariant().EndsWith('.zip')) {
      Install-CliFromZip -ZipPath $assetPath -DestDir $dest -DryRun:$DryRun
    } else {
      Install-CliLegacyExe -ExePath $assetPath -DestDir $dest -DryRun:$DryRun
    }
    Write-Log "Done. Try: gestura --help"
    return
  }

  # full
  $msi = Try-DownloadAssets -Repo $Repo -Tag $resolvedTag -WorkDir $workDir -AssetNames @(
    "Gestura-$resolvedTag-windows-x86_64.msi"
  )
  if (-not $msi) { throw "Unable to download full installer (MSI)" }
  Maybe-Verify -Repo $Repo -Tag $resolvedTag -WorkDir $workDir -AssetPath $msi -NoVerify:$NoVerify -RequireVerify:$RequireVerify
  Install-FullMsi -MsiPath $msi -DryRun:$DryRun
  Write-Log "Done. GUI should be installed; CLI should be available as 'gestura.exe'."
}

# If executed directly, run with the script parameters.
Install-Gestura -Mode $Mode -Tag $Tag -Repo $Repo -InstallDir $InstallDir -NoVerify:$NoVerify -RequireVerify:$RequireVerify -DryRun:$DryRun
