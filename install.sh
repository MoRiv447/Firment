#!/usr/bin/env sh
# Firment 一键安装脚本（macOS / Linux）
# 用法:
#   curl -fsSL https://raw.githubusercontent.com/MoRiv447/Firment/main/install.sh | sh
# 可选环境变量:
#   FIRMENT_VERSION   指定版本 tag（默认 latest）
#   FIRMENT_MIRROR    国内镜像根地址，目录结构: {mirror}/{tag}/{asset}
#   FIRMENT_REPO      仓库（默认 MoRiv447/Firment）
#   FIRMENT_INSTALL_DIR  安装目录（默认 ~/.firment/bin）。`FIRMENT_BIN_DIR` 是同义的别名：
#                      那是 `firm install` 自己认的名字，两个都收，是为了在 Unix 上和
#                      install.ps1 上设置同一个变量能得到同一个目录。
#   FIRMENT_DRY_RUN   设为 1 时只打印安装计划，不下载、不执行
set -eu

REPO="${FIRMENT_REPO:-MoRiv447/Firment}"
VERSION="${FIRMENT_VERSION:-latest}"
MIRROR="${FIRMENT_MIRROR:-}"
INSTALL_DIR="${FIRMENT_INSTALL_DIR:-${FIRMENT_BIN_DIR:-$HOME/.firment/bin}}"

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux) OS_TARGET="unknown-linux-gnu" ;;
    Darwin) OS_TARGET="apple-darwin" ;;
    *) echo "不支持的平台: $OS" >&2; exit 1 ;;
esac
case "$ARCH" in
    x86_64|amd64) ARCH_TARGET="x86_64" ;;
    aarch64|arm64) ARCH_TARGET="aarch64" ;;
    *) echo "不支持的架构: $ARCH" >&2; exit 1 ;;
esac

# The release job builds four images: x86_64 Linux, and x86_64/aarch64 macOS. There is no
# aarch64 Linux image, so on a Raspberry Pi or an SBC — the machines this project is
# otherwise about — the honest answer is that one sentence, not a URL for an asset no
# workflow writes. A wrong URL then arrives as "该版本尚未发布或平台不支持", which is a
# guess about a server, and the fact was already known here.
if [ "$OS_TARGET" = "unknown-linux-gnu" ] && [ "$ARCH_TARGET" = "aarch64" ]; then
    echo "Linux 只发布 x86_64-unknown-linux-gnu；aarch64-unknown-linux-gnu 没有被构建" >&2
    echo "（发布矩阵：x86_64-unknown-linux-gnu、x86_64-apple-darwin、aarch64-apple-darwin）。" >&2
    echo "在 SBC/树莓派上请从源码安装：" >&2
    echo "  cargo build --release --locked --target aarch64-unknown-linux-gnu" >&2
    exit 1
fi

ASSET="firm-${ARCH_TARGET}-${OS_TARGET}.tar.gz"

TAG="$VERSION"
if [ -n "$MIRROR" ]; then
    if [ "$VERSION" = "latest" ]; then
        RELEASE_JSON="$(curl -fsSL -H 'User-Agent: firment-installer' "https://api.github.com/repos/$REPO/releases/latest")"
        TAG="$(printf '%s' "$RELEASE_JSON" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
    fi
    ASSET_URL="$MIRROR/$TAG/$ASSET"
    SUM_URL="$MIRROR/$TAG/SHA256SUMS"
else
    if [ "$VERSION" = "latest" ]; then
        ASSET_URL="https://github.com/$REPO/releases/latest/download/$ASSET"
        SUM_URL="https://github.com/$REPO/releases/latest/download/SHA256SUMS"
    else
        ASSET_URL="https://github.com/$REPO/releases/download/$VERSION/$ASSET"
        SUM_URL="https://github.com/$REPO/releases/download/$VERSION/SHA256SUMS"
    fi
fi

if [ "${FIRMENT_DRY_RUN:-}" = "1" ]; then
    echo "[dry-run] 以下操作不会真正执行："
    echo "  仓库      : $REPO"
    echo "  版本      : $TAG"
    echo "  安装包    : $ASSET_URL"
    echo "  校验和    : $SUM_URL"
    echo "  安装目录  : $INSTALL_DIR"
    echo "  步骤      : 下载 -> SHA256 校验 -> 解压 -> 安装到 \$INSTALL_DIR/firm 并写入 PATH"
    echo "[dry-run] 结束。去掉 FIRMENT_DRY_RUN=1 后重新执行即可真正安装。"
    exit 0
fi

TMP="$(mktemp -d)"
# `STAGE` is created only just before the binary is put in place, but the trap is installed here so
# an interrupt anywhere in between cannot leave it behind either.
STAGE=""
cleanup() {
    rm -rf "$TMP"
    if [ -n "$STAGE" ]; then
        rm -f "$STAGE"
    fi
}
trap cleanup EXIT
TARBALL="$TMP/$ASSET"
curl -fL "$ASSET_URL" -o "$TARBALL" || { echo "下载失败（$ASSET_URL）：可能该版本尚未发布或平台不支持" >&2; exit 1; }

# 校验是这一步的承诺，不是可选项：dry-run 里写着「下载 -> SHA256 校验 -> 解压」，
# 而取不到校验文件时静默跳过，等于装了来路不明的二进制还报告成功。缺文件、缺这一行
# 都是错误 —— 安装包和 SHA256SUMS 由同一次 release 产出，缺一个就说明拿错了地方。
SUMS="$(curl -fsSL "$SUM_URL" 2>/dev/null || true)"
[ -n "$SUMS" ] || { echo "无法取得校验文件（$SUM_URL）：无法校验安装包，已中止" >&2; exit 1; }
EXPECTED="$(printf '%s\n' "$SUMS" | awk -v a="$ASSET" '$2 == a { print $1 }' | head -n 1)"
[ -n "$EXPECTED" ] || { echo "校验文件中没有 $ASSET 这一行：该平台/版本未被发布校验，已中止" >&2; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "$TARBALL" | awk '{print $1}')"
else
    ACTUAL="$(shasum -a 256 "$TARBALL" | awk '{print $1}')"
fi
[ "$ACTUAL" = "$EXPECTED" ] || { echo "SHA256 校验失败: $ASSET" >&2; exit 1; }

mkdir -p "$INSTALL_DIR"
tar -xzf "$TARBALL" -C "$TMP"
BIN="$(find "$TMP" -type f -name firm | head -n 1)"
[ -n "$BIN" ] || { echo "压缩包中未找到 firm" >&2; exit 1; }
# 先写同目录的暂存名，再 rename 到位：`install` 是直接写目标文件的，中断（Ctrl-C、磁盘写满）
# 会在 PATH 上留下一个截断的 firm，而原来那个可用的已经没了。暂存在 $INSTALL_DIR 内、与目标同
# 一个文件系统，所以 `mv` 走的是 rename(2)，是原子的 —— 也就是 `firm update` 从
# crates/firment-cli/src/install.rs 的 `replace_file` 拿到的那条保证。（另外 rename 覆盖
# **正在运行**的二进制不会失败，而在原地写会得到 ETXTBSY。）
STAGE="$INSTALL_DIR/.firm.new.$$"
install -m 755 "$BIN" "$STAGE"
mv -f "$STAGE" "$INSTALL_DIR/firm"
STAGE=""

case "${SHELL:-}" in
    *zsh*) RC="$HOME/.zshrc" ;;
    *bash*) RC="$HOME/.bashrc" ;;
    *) RC="$HOME/.profile" ;;
esac
# The directory the binary actually landed in, not the default one: with
# FIRMENT_INSTALL_DIR set, the old line put a path on PATH that nothing had been
# installed into and still reported success. Same string on both sides of the
# check, so a second run does not append a second copy.
if ! grep -qF "$INSTALL_DIR" "$RC" 2>/dev/null; then
    printf '\n# firment\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$RC"
    echo "已把 $INSTALL_DIR 加入 PATH（写入 $RC，新终端生效）"
fi

echo "Firment $TAG 安装完成: $INSTALL_DIR/firm"
echo "新开终端后直接输入 firm 即可。"
