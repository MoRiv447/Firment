# Firment 一键安装脚本（Windows / PowerShell）
# 用法:
#   irm https://raw.githubusercontent.com/MoRiv447/Firment/main/install.ps1 | iex
# 可选环境变量:
#   FIRMENT_VERSION   指定版本 tag（默认 latest）
#   FIRMENT_MIRROR    国内镜像根地址，目录结构: {mirror}/{tag}/{asset}
#   FIRMENT_REPO      仓库（默认 MoRiv447/Firment）
#   FIRMENT_INSTALL_DIR / FIRMENT_BIN_DIR  安装目录（默认 %USERPROFILE%\.firment\bin）。
#                   两个名字都收：前者是 install.sh 用的，后者是 `firm install` 自己认的；
#                   优先级 FIRMENT_INSTALL_DIR > FIRMENT_BIN_DIR > 默认。
#   FIRMENT_DRY_RUN 设为 1 时只打印安装计划，不下载、不执行
#   FIRMENT_FILES_ONLY 设为 1 时只装文件，不写用户 PATH

$ErrorActionPreference = 'Stop'
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 } catch {}

$Repo = if ($env:FIRMENT_REPO) { $env:FIRMENT_REPO } else { 'MoRiv447/Firment' }
$Version = if ($env:FIRMENT_VERSION) { $env:FIRMENT_VERSION } else { 'latest' }
$Mirror = if ($env:FIRMENT_MIRROR) { $env:FIRMENT_MIRROR.TrimEnd('/') } else { '' }

# Resolved once, here, and passed to `firm install --to` below. It used to be computed inside
# the dry-run block only, which made the plan print a directory the install never used: the
# real work went to `firm install`, which reads FIRMENT_BIN_DIR (or its default) and ignored
# whatever the message had claimed. A dry run that describes a different install than the one
# it precedes is the failure this file's checksum rule already refuses.
$installDir = if ($env:FIRMENT_INSTALL_DIR) {
    $env:FIRMENT_INSTALL_DIR
} elseif ($env:FIRMENT_BIN_DIR) {
    $env:FIRMENT_BIN_DIR
} else {
    Join-Path $env:USERPROFILE '.firment\bin'
}

$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -eq 'AMD64') {
    $arch = 'x86_64'
} elseif ($arch -eq 'ARM64') {
    # The release job builds one Windows image: firm-x86_64-pc-windows-msvc.zip. Asking for
    # the aarch64 one produced a URL no workflow writes, and the failure came back as
    # "可能该版本尚未发布或平台不支持" -- a guess about the server, when the fact was known
    # here. Say the fact, and say what to do instead.
    throw 'Windows ARM64 (aarch64-pc-windows-msvc) 没有发布包；发布矩阵只有 x86_64-pc-windows-msvc。如需 ARM64，请从源码构建：cargo build --release --locked --target aarch64-pc-windows-msvc'
} else {
    throw "不支持的架构: $arch"
}
$AssetName = "firm-$arch-pc-windows-msvc.zip"

$Tag = $Version
$DownloadUrl = ''
$SumsUrl = ''
if ($Mirror) {
    if ($Version -eq 'latest') {
        $apiHeaders = @{ 'User-Agent' = 'firment-installer' }
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers $apiHeaders
        $Tag = $release.tag_name
    }
    $DownloadUrl = "$Mirror/$Tag/$AssetName"
    $SumsUrl = "$Mirror/$Tag/SHA256SUMS"
} else {
    $ReleaseBase = if ($Version -eq 'latest') {
        "https://github.com/$Repo/releases/latest/download"
    } else {
        "https://github.com/$Repo/releases/download/$Version"
    }
    $DownloadUrl = "$ReleaseBase/$AssetName"
    $SumsUrl = "$ReleaseBase/SHA256SUMS"
}

if ($env:FIRMENT_DRY_RUN -eq '1') {
    Write-Host '[dry-run] 以下操作不会真正执行：'
    Write-Host "  仓库      : $Repo"
    Write-Host "  版本      : $Tag"
    Write-Host "  安装包    : $DownloadUrl"
    Write-Host "  校验和    : $SumsUrl"
    Write-Host "  安装目录  : $installDir"
    Write-Host '  步骤      : 下载 -> SHA256 校验 -> 解压 -> firm install --to $安装目录（写用户 PATH + PowerShell 补全）'
    Write-Host '[dry-run] 结束。去掉 FIRMENT_DRY_RUN=1 后重新执行即可真正安装。'
    exit 0
}

$tmp = Join-Path $env:TEMP "firment-install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp $AssetName
    try {
        Invoke-WebRequest -UseBasicParsing -Uri $DownloadUrl -OutFile $zip
    } catch {
        throw "下载失败（$DownloadUrl）：可能该版本尚未发布或平台不支持"
    }

    # Verification is the promise this step prints in the dry-run plan, so a missing
    # checksum file is an error rather than a silent pass: the installer would otherwise
    # run an unverified binary and report success. Same rule as install.sh, which fails
    # closed for the same reason.
    try {
        $sumsText = (Invoke-WebRequest -UseBasicParsing -Uri $SumsUrl).Content
    } catch {
        throw "无法取得校验文件（$SumsUrl）：无法校验安装包，已中止"
    }
    # Match the FILENAME FIELD exactly, the way install.sh's `$2 == a` does. A substring
    # test here would also accept a companion row -- `firm-x86_64-...zip.asc`, or the
    # aarch64 asset of the same name shape -- and then compare the zip against a hash of
    # something else.
    $line = $sumsText -split "`n" | ForEach-Object { $_.Trim() } |
        Where-Object { ($_ -split '\s+')[1] -eq $AssetName } | Select-Object -First 1
    if (-not $line) {
        throw "校验文件中没有 $AssetName 这一行：该平台/版本未被发布校验，已中止"
    }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "SHA256 校验失败: $AssetName"
    }

    Expand-Archive -LiteralPath $zip -DestinationPath $tmp
    $exe = Get-ChildItem -LiteralPath $tmp -Recurse -Filter 'firm.exe' | Select-Object -First 1
    if (-not $exe) {
        throw "压缩包中未找到 firm.exe"
    }

    Write-Host "Firment $Tag 下载完成，开始安装..."
    # The directory the plan printed is the directory the install gets, rather than a name
    # `firm install` happens to re-read from the environment.
    $installArgs = @('--to', $installDir)
    if ($env:FIRMENT_FILES_ONLY -eq '1') { $installArgs += '--files-only' }
    & $exe.FullName install @installArgs
    if ($LASTEXITCODE -ne 0) {
        throw "firm install 失败 (exit $LASTEXITCODE)"
    }
    Write-Host ""
    Write-Host "安装完成！请新开一个终端，直接输入 firm 即可唤起。"
} finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
}
