param(
  [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$OutDir = Join-Path $RepoRoot "src-tauri\binaries"
$RuntimeOutDir = Join-Path $RepoRoot "src-tauri\windows-runtime"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
New-Item -ItemType Directory -Force -Path $RuntimeOutDir | Out-Null

$Headers = @{
  "User-Agent" = "OffPDF-build"
}

if ($Version.Trim().Length -gt 0) {
  $Tag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
  $ReleaseUrl = "https://api.github.com/repos/qpdf/qpdf/releases/tags/$Tag"
} else {
  $ReleaseUrl = "https://api.github.com/repos/qpdf/qpdf/releases/latest"
}

Write-Host "Resolving qpdf release from $ReleaseUrl"
$Release = Invoke-RestMethod -Uri $ReleaseUrl -Headers $Headers
$Asset = $Release.assets | Where-Object { $_.name -match "msvc64\.zip$" } | Select-Object -First 1

if (-not $Asset) {
  $Names = ($Release.assets | ForEach-Object { $_.name }) -join ", "
  throw "Could not find a qpdf msvc64 zip asset. Available assets: $Names"
}

$TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("offpdf-qpdf-" + [guid]::NewGuid())
try {
  New-Item -ItemType Directory -Path $TempRoot | Out-Null
  $ZipPath = Join-Path $TempRoot $Asset.name

  Write-Host "Downloading $($Asset.name)"
  Invoke-WebRequest -Uri $Asset.browser_download_url -OutFile $ZipPath -Headers $Headers

  Write-Host "Extracting qpdf"
  Expand-Archive -Path $ZipPath -DestinationPath $TempRoot -Force

  $QpdfExe = Get-ChildItem -Path $TempRoot -Recurse -Filter "qpdf.exe" |
    Where-Object { $_.FullName -match "[\\/]bin[\\/]qpdf\.exe$" } |
    Select-Object -First 1

  if (-not $QpdfExe) {
    throw "Could not find qpdf.exe in extracted archive."
  }

  $QpdfBin = $QpdfExe.Directory.FullName
  Write-Host "Copying qpdf runtime from $QpdfBin to $OutDir"

  Copy-Item -Path (Join-Path $QpdfBin "qpdf.exe") -Destination $OutDir -Force
  Get-ChildItem -Path $QpdfBin -Filter "*.dll" | Copy-Item -Destination $OutDir -Force

  foreach ($Runtime in @("msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll")) {
    $RuntimeSource = Join-Path $QpdfBin $Runtime
    if (-not (Test-Path $RuntimeSource)) {
      throw "Could not find required Windows runtime in the qpdf archive: $Runtime"
    }
    Copy-Item -Path $RuntimeSource -Destination $RuntimeOutDir -Force
  }
} finally {
  if (Test-Path -LiteralPath $TempRoot) {
    Remove-Item -LiteralPath $TempRoot -Recurse -Force
  }
}

Write-Host "Bundled files:"
Get-ChildItem -Path $OutDir | Select-Object Name, Length | Format-Table -AutoSize

Write-Host "Runtime DLLs staged at the application root:"
Get-ChildItem -Path $RuntimeOutDir -File | Select-Object Name, Length | Format-Table -AutoSize

if ($IsWindows) {
  Write-Host "qpdf version:"
  & (Join-Path $OutDir "qpdf.exe") --version
} else {
  Write-Host "Skipping qpdf.exe --version because this host is not Windows."
}
