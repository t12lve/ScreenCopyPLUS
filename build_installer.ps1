# Build script for ScreenCopyPLUS Installer
$ErrorActionPreference = "Stop"

Write-Host "1. Compilation de ScreenCopyPLUS en mode Release (Rust)..." -ForegroundColor Cyan
cargo build --release

Copy-Item "target/release/screencopy-plus.exe" -Destination "ScreenCopyPLUS.exe" -Force

Write-Host "2. Compilation de l'installeur Inno Setup..." -ForegroundColor Cyan
$iscc = "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
    $iscc = "ISCC.exe"
}

$tempOut = "$env:TEMP\screencopy_dist"
New-Item -ItemType Directory -Force -Path $tempOut | Out-Null
New-Item -ItemType Directory -Force -Path "dist" | Out-Null

& $iscc /O"$tempOut" "installer\ScreenCopyPLUS_Setup.iss"

Copy-Item "$tempOut\ScreenCopyPLUS_Setup_v5.0.1.exe" -Destination "dist\ScreenCopyPLUS_Setup_v5.0.1.exe" -Force
Write-Host "Installeur gnr avec succs dans dist\ScreenCopyPLUS_Setup_v5.0.1.exe !" -ForegroundColor Green
