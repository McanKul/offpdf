param(
  [string]$Version = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$OutDir = Join-Path $RepoRoot "src-tauri\libreoffice"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Get-ChildItem -Path $OutDir -Force |
  Where-Object { $_.Name -ne ".gitkeep" } |
  Remove-Item -Recurse -Force

$Choco = Get-Command choco -ErrorAction SilentlyContinue
if (-not $Choco) {
  throw "Chocolatey is required to prepare the Windows LibreOffice runtime."
}

$InstallArgs = @("install", "libreoffice-fresh", "--yes", "--no-progress")
if ($Version.Trim().Length -gt 0) {
  $InstallArgs += "--version=$Version"
}

Write-Host "Installing LibreOffice with Chocolatey"
& choco @InstallArgs
if ($LASTEXITCODE -ne 0) {
  throw "Chocolatey failed to install LibreOffice."
}

$CandidateRoots = @()
if ($env:ProgramFiles) {
  $CandidateRoots += Join-Path $env:ProgramFiles "LibreOffice"
}
if (${env:ProgramFiles(x86)}) {
  $CandidateRoots += Join-Path ${env:ProgramFiles(x86)} "LibreOffice"
}
if ($env:ChocolateyInstall) {
  $CandidateRoots += Join-Path $env:ChocolateyInstall "lib\libreoffice-fresh"
}
$CandidateRoots = $CandidateRoots | Where-Object { $_ -and (Test-Path $_) }

if (-not $CandidateRoots) {
  throw "Could not find a LibreOffice installation directory after Chocolatey finished."
}

$Soffice = Get-ChildItem -Path $CandidateRoots -Recurse -File |
  Where-Object { $_.FullName -match "[\\/]program[\\/]soffice\.(com|exe)$" } |
  Sort-Object @{ Expression = { if ($_.Name -eq "soffice.com") { 0 } else { 1 } } } |
  Select-Object -First 1

if (-not $Soffice) {
  throw "Could not locate LibreOffice program\soffice.com or program\soffice.exe after installation."
}

$LibreOfficeRoot = $Soffice.Directory.Parent.FullName
Write-Host "Copying LibreOffice runtime from $LibreOfficeRoot to $OutDir"
Copy-Item -Path (Join-Path $LibreOfficeRoot "*") -Destination $OutDir -Recurse -Force

$BundledSoffice = Join-Path $OutDir "program\soffice.com"
if (-not (Test-Path $BundledSoffice)) {
  $BundledSoffice = Join-Path $OutDir "program\soffice.exe"
}
if (-not (Test-Path $BundledSoffice)) {
  throw "Bundled soffice.com or soffice.exe was not copied to $OutDir\program."
}

Write-Host "LibreOffice version:"
& $BundledSoffice --version

$SmokeDir = Join-Path ([System.IO.Path]::GetTempPath()) ("offpdf-libreoffice-smoke-" + [guid]::NewGuid())
$SmokeProfile = Join-Path $SmokeDir "profile"
New-Item -ItemType Directory -Force -Path $SmokeDir | Out-Null
New-Item -ItemType Directory -Force -Path $SmokeProfile | Out-Null
$SmokeInput = Join-Path $SmokeDir "smoke.rtf"
Set-Content -Path $SmokeInput -Value "{\rtf1\ansi OffPDF LibreOffice smoke test}" -Encoding ASCII
$SmokeProfileUrl = "file:///" + ($SmokeProfile -replace "\\", "/")

Write-Host "Running LibreOffice conversion smoke test"
$SmokeLog = & $BundledSoffice --headless --norestore --nodefault --nolockcheck --nofirststartwizard "-env:UserInstallation=$SmokeProfileUrl" --convert-to pdf --outdir $SmokeDir $SmokeInput 2>&1
$SmokeLog | ForEach-Object { Write-Host $_ }
if ($LASTEXITCODE -ne 0) {
  throw "LibreOffice smoke test failed."
}
$SmokeOutput = Join-Path $SmokeDir "smoke.pdf"
if (-not (Test-Path $SmokeOutput)) {
  Write-Host "Smoke directory contents:"
  Get-ChildItem -Path $SmokeDir -Recurse | Select-Object FullName, Length | Format-Table -AutoSize
  throw "LibreOffice smoke test did not produce smoke.pdf."
}
Remove-Item -Path $SmokeDir -Recurse -Force

Write-Host "Bundled LibreOffice top-level files:"
Get-ChildItem -Path $OutDir |
  Select-Object Name, Length |
  Format-Table -AutoSize
