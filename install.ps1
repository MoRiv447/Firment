# Firment 一键安装脚本（Windows / PowerShell）
# 用法:
#   irm https://raw.githubusercontent.com/MoRiv447/Firment/main/install.ps1 | iex
# 可选环境变量:
#   FIRMENT_VERSION   指定版本 tag（默认 latest）
#   FIRMENT_MIRROR    国内镜像根地址，目录结构: {mirror}/{tag}/{asset}
#   FIRMENT_REPO      仓库（默认 MoRiv447/Firment）

$ErrorActionPreference = 'Stop'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch {}

$Repo = if ($env:FIRMENT_REPO) { $env:FIRMENT_REPO } else { 'MoRiv447/Firment' }
$Version = if ($env:FIRMENT_VERSION) { $env:FIRMENT_VERSION } else { 'latest' }
$Mirror = if ($env:FIRMENT_MIRROR) { $env:FIRMENT_MIRROR.TrimEnd('/') } else { '' }

$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -eq 'AMD64') {
    $arch = 'x86_64'
} elseif ($arch -eq 'ARM64') {
    $arch = 'aarch64'
} else {
    throw "不支持的架构: $arch"
}
$AssetName = "firm-$arch-pc-windows-msvc.zip"

$apiHeaders = @{ 'User-Agent' = 'firment-installer' }
if ($Version -eq 'latest') {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers $apiHeaders
} else {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/tags/$Version" -Headers $apiHeaders
}
$Tag = $release.tag_name
$asset = $release.assets | Where-Object { $_.name -eq $AssetName } | Select-Object -First 1
if (-not $asset) {
    throw "当前 release ($Tag) 没有 $AssetName，可能尚未发布或平台不支持"
}

$tmp = Join-Path $env:TEMP "firment-install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp $AssetName
    if ($Mirror) {
        Invoke-WebRequest -UseBasicParsing -Uri "$Mirror/$Tag/$AssetName" -OutFile $zip
    } else {
        Invoke-WebRequest -UseBasicParsing -Uri $asset.browser_download_url -OutFile $zip
    }

    $sumsAsset = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
    if ($sumsAsset) {
        $sumsUrl = if ($Mirror) { "$Mirror/$Tag/SHA256SUMS" } else { $sumsAsset.browser_download_url }
        $sumsText = (Invoke-WebRequest -UseBasicParsing -Uri $sumsUrl).Content
        $line = ($sumsText -split "`n" | Where-Object { $_ -match [regex]::Escape($AssetName) } | Select-Object -First 1)
        if ($line) {
            $expected = ($line -split '\s+')[0].ToLowerInvariant()
            $actual = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($actual -ne $expected) {
                throw "SHA256 校验失败: $AssetName"
            }
        }
    }

    Expand-Archive -LiteralPath $zip -DestinationPath $tmp
    $exe = Get-ChildItem -LiteralPath $tmp -Recurse -Filter 'firm.exe' | Select-Object -First 1
    if (-not $exe) {
        throw "压缩包中未找到 firm.exe"
    }

    Write-Host "Firment $Tag 下载完成，开始安装..."
    & $exe.FullName install
    if ($LASTEXITCODE -ne 0) {
        throw "firm install 失败 (exit $LASTEXITCODE)"
    }
    Write-Host ""
    Write-Host "安装完成！请新开一个终端，直接输入 firm 即可唤起。"
} finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
}
