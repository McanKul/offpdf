param(
  [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$BinOutDir = Join-Path $RepoRoot "src-tauri\binaries"
$ShareOutDir = Join-Path $RepoRoot "src-tauri\share"
New-Item -ItemType Directory -Force -Path $BinOutDir | Out-Null
New-Item -ItemType Directory -Force -Path $ShareOutDir | Out-Null

$Headers = @{
  "User-Agent" = "OffPDF-build"
}

if ($Version.Trim().Length -gt 0) {
  $Tag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
  $ReleaseUrl = "https://api.github.com/repos/oschwartz10612/poppler-windows/releases/tags/$Tag"
} else {
  $ReleaseUrl = "https://api.github.com/repos/oschwartz10612/poppler-windows/releases/latest"
}

Write-Host "Resolving poppler release from $ReleaseUrl"
$Release = Invoke-RestMethod -Uri $ReleaseUrl -Headers $Headers
$Asset = $Release.assets | Where-Object { $_.name -match "^Release-.*\.zip$" } | Select-Object -First 1

if (-not $Asset) {
  $Names = ($Release.assets | ForEach-Object { $_.name }) -join ", "
  throw "Could not find a poppler Release zip asset. Available assets: $Names"
}

$TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("offpdf-poppler-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $TempRoot | Out-Null
$ZipPath = Join-Path $TempRoot $Asset.name

try {
  Write-Host "Downloading $($Asset.name)"
  Invoke-WebRequest -Uri $Asset.browser_download_url -OutFile $ZipPath -Headers $Headers

  Write-Host "Extracting poppler"
  Expand-Archive -Path $ZipPath -DestinationPath $TempRoot -Force

  $PdfToPpm = Get-ChildItem -Path $TempRoot -Recurse -Filter "pdftoppm.exe" |
    Where-Object { $_.FullName -match "[\\/]Library[\\/]bin[\\/]pdftoppm\.exe$" } |
    Select-Object -First 1

  if (-not $PdfToPpm) {
    throw "Could not find pdftoppm.exe in extracted archive."
  }

  $PopplerBin = $PdfToPpm.Directory.FullName
  Write-Host "Copying poppler runtime from $PopplerBin to $BinOutDir"

  foreach ($Tool in @("pdftoppm.exe", "pdftotext.exe")) {
    $ToolPath = Join-Path $PopplerBin $Tool
    if (-not (Test-Path $ToolPath)) {
      throw "Could not find required poppler tool: $Tool"
    }
    Copy-Item -Path $ToolPath -Destination $BinOutDir -Force
  }

  Get-ChildItem -Path $PopplerBin -Filter "*.dll" | Copy-Item -Destination $BinOutDir -Force

  $PopplerData = Get-ChildItem -Path $TempRoot -Recurse -Directory |
    Where-Object { $_.FullName -match "[\\/]share[\\/]poppler$" } |
    Select-Object -First 1

  if ($PopplerData) {
    $TargetData = Join-Path $ShareOutDir "poppler"
    if (Test-Path $TargetData) {
      Remove-Item -Path $TargetData -Recurse -Force
    }
    Write-Host "Copying poppler data from $($PopplerData.FullName) to $TargetData"
    Copy-Item -Path $PopplerData.FullName -Destination $ShareOutDir -Recurse -Force
  } else {
    Write-Host "No poppler data directory found in the archive."
  }

  Write-Host "Bundled poppler files:"
  Get-ChildItem -Path $BinOutDir -File |
    Where-Object { $_.Name -match "^pdf.*\.exe$|\.dll$" } |
    Select-Object Name, Length |
    Format-Table -AutoSize

  if ($IsWindows) {
    Write-Host "pdftoppm version:"
    & (Join-Path $BinOutDir "pdftoppm.exe") -v
    Write-Host "pdftotext version:"
    & (Join-Path $BinOutDir "pdftotext.exe") -v
  } else {
    Write-Host "Skipping poppler exe version checks because this host is not Windows."
  }
} finally {
  if (Test-Path $TempRoot) {
    Remove-Item -Path $TempRoot -Recurse -Force
  }
}
