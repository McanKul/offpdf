param(
  [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$OutDir = Join-Path $RepoRoot "src-tauri\tesseract"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Get-ChildItem -Path $OutDir -Force |
  Where-Object { $_.Name -ne ".gitkeep" } |
  Remove-Item -Recurse -Force

$Choco = Get-Command choco -ErrorAction SilentlyContinue
if (-not $Choco) {
  throw "Chocolatey is required to prepare the Windows Tesseract runtime."
}

$InstallArgs = @("install", "tesseract", "--yes", "--no-progress")
if ($Version.Trim().Length -gt 0) {
  $InstallArgs += "--version=$Version"
}

Write-Host "Installing Tesseract with Chocolatey"
& choco @InstallArgs
if ($LASTEXITCODE -ne 0) {
  throw "Chocolatey failed to install Tesseract."
}

$CandidateRoots = @()
if ($env:ProgramFiles) {
  $CandidateRoots += Join-Path $env:ProgramFiles "Tesseract-OCR"
}
if (${env:ProgramFiles(x86)}) {
  $CandidateRoots += Join-Path ${env:ProgramFiles(x86)} "Tesseract-OCR"
}
if ($env:ChocolateyInstall) {
  $CandidateRoots += Join-Path $env:ChocolateyInstall "lib\tesseract"
}
$CandidateRoots = $CandidateRoots | Where-Object { $_ -and (Test-Path $_) }

if (-not $CandidateRoots) {
  throw "Could not find a Tesseract installation directory after Chocolatey finished."
}

$TesseractExe = Get-ChildItem -Path $CandidateRoots -Recurse -Filter "tesseract.exe" |
  Select-Object -First 1

if (-not $TesseractExe) {
  throw "Could not locate tesseract.exe after installation."
}

$TesseractRoot = $TesseractExe.Directory.FullName
Write-Host "Copying Tesseract runtime from $TesseractRoot to $OutDir"
Copy-Item -Path (Join-Path $TesseractRoot "*") -Destination $OutDir -Recurse -Force

$TessdataDir = Join-Path $OutDir "tessdata"
New-Item -ItemType Directory -Force -Path $TessdataDir | Out-Null

$Headers = @{
  "User-Agent" = "OffPDF-build"
}

foreach ($Lang in @("eng", "tur", "deu", "fra", "osd")) {
  $Target = Join-Path $TessdataDir "$Lang.traineddata"
  $Url = "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/main/$Lang.traineddata"
  Write-Host "Downloading OCR language data: $Lang"
  Invoke-WebRequest -Uri $Url -OutFile $Target -Headers $Headers
}

$ConfigsDir = Join-Path $TessdataDir "configs"
New-Item -ItemType Directory -Force -Path $ConfigsDir | Out-Null
$PdfConfig = Join-Path $ConfigsDir "pdf"
if (-not (Test-Path $PdfConfig)) {
  Set-Content -Path $PdfConfig -Value "tessedit_create_pdf 1" -Encoding ASCII
}

$BundledExe = Join-Path $OutDir "tesseract.exe"
if (-not (Test-Path $BundledExe)) {
  throw "Bundled tesseract.exe was not copied to $OutDir."
}

Write-Host "Tesseract version:"
& $BundledExe --version

Write-Host "Bundled OCR languages:"
& $BundledExe --tessdata-dir $TessdataDir --list-langs

Write-Host "Bundled Tesseract files:"
Get-ChildItem -Path $OutDir -File |
  Select-Object Name, Length |
  Format-Table -AutoSize
