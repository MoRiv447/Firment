#!/usr/bin/env sh
# Firment 一键安装脚本（macOS / Linux）
# 用法:
#   curl -fsSL https://raw.githubusercontent.com/MoRiv447/Firment/main/install.sh | sh
# 可选环境变量:
#   FIRMENT_VERSION   指定版本 tag（默认 latest）
#   FIRMENT_MIRROR    国内镜像根地址，目录结构: {mirror}/{tag}/{asset}
#   FIRMENT_REPO      仓库（默认 MoRiv447/Firment）
#   FIRMENT_INSTALL_DIR  安装目录（默认 ~/.firment/bin）
set -eu

REPO="${FIRMENT_REPO:-MoRiv447/Firment}"
VERSION="${FIRMENT_VERSION:-latest}"
MIRROR="${FIRMENT_MIRROR:-}"
INSTALL_DIR="${FIRMENT_INSTALL_DIR:-$HOME/.firment/bin}"

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

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
TARBALL="$TMP/$ASSET"
curl -fL "$ASSET_URL" -o "$TARBALL" || { echo "下载失败（$ASSET_URL）：可能该版本尚未发布或平台不支持" >&2; exit 1; }

SUMS="$(curl -fsSL "$SUM_URL" 2>/dev/null || true)"
if [ -n "$SUMS" ]; then
    EXPECTED="$(printf '%s\n' "$SUMS" | awk -v a="$ASSET" '$2 == a { print $1 }' | head -n 1)"
    if [ -n "$EXPECTED" ]; then
        if command -v sha256sum >/dev/null 2>&1; then
            ACTUAL="$(sha256sum "$TARBALL" | awk '{print $1}')"
        else
            ACTUAL="$(shasum -a 256 "$TARBALL" | awk '{print $1}')"
        fi
        [ "$ACTUAL" = "$EXPECTED" ] || { echo "SHA256 校验失败: $ASSET" >&2; exit 1; }
    fi
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "$TARBALL" -C "$TMP"
BIN="$(find "$TMP" -type f -name firm | head -n 1)"
[ -n "$BIN" ] || { echo "压缩包中未找到 firm" >&2; exit 1; }
install -m 755 "$BIN" "$INSTALL_DIR/firm"

case "${SHELL:-}" in
    *zsh*) RC="$HOME/.zshrc" ;;
    *bash*) RC="$HOME/.bashrc" ;;
    *) RC="$HOME/.profile" ;;
esac
if ! grep -qF '.firment/bin' "$RC" 2>/dev/null; then
    printf '\n# firment\nexport PATH="$HOME/.firment/bin:$PATH"\n' >> "$RC"
    echo "已把 PATH 写入 $RC，新终端生效"
fi

echo "Firment $TAG 安装完成: $INSTALL_DIR/firm"
echo "新开终端后直接输入 firm 即可。"
