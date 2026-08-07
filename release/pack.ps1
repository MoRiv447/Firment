# 本地打包（Windows x64 优先；Linux/macOS 由 GitHub Actions 自动构建）
# 用法: powershell -ExecutionPolicy Bypass -File release\pack.ps1
param(
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    if (-not $SkipBuild) {
        & "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release 失败" }
    }

    $dist = Join-Path $root 'dist'
    if (Test-Path -LiteralPath $dist) { Remove-Item -Recurse -Force -LiteralPath $dist }
    New-Item -ItemType Directory -Path $dist | Out-Null

    $exe = Join-Path $root 'target\release\firm.exe'
    $zip = Join-Path $dist 'firm-x86_64-pc-windows-msvc.zip'
    Compress-Archive -LiteralPath $exe -DestinationPath $zip -Force

    $sums = Get-ChildItem -LiteralPath $dist -File | ForEach-Object {
        $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "{0}  {1}" -f $hash, $_.Name
    }
    $sums | Set-Content -LiteralPath (Join-Path $dist 'SHA256SUMS') -Encoding ascii

    Write-Host "打包完成:"
    Get-ChildItem -LiteralPath $dist | ForEach-Object { Write-Host "  $($_.Name) ($($_.Length) B)" }
} finally {
    Pop-Location
}
