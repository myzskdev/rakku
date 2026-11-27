#!/usr/bin/env sh
set -eu

# rakku (by fukulabs.dev) — one-line Linux installer
APP_NAME="${APP_NAME:-rakku}"
REPO="${REPO:-myzskdev/rakku}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
VERSION="${VERSION:-latest}"

OS="$(uname -s)"
if [ "$OS" != "Linux" ]; then
  echo "error: ${APP_NAME} is intended for Linux servers (detected: ${OS})" >&2
  exit 1
fi

ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  x86_64|amd64) ARCH="amd64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *)
    echo "error: unsupported architecture: ${ARCH_RAW}" >&2
    exit 1
    ;;
esac

ASSET="${APP_NAME}-linux-${ARCH}"
if [ "$VERSION" = "latest" ]; then
  BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
  BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${ASSET} from ${BASE_URL}..."
curl -fsSL "${BASE_URL}/${ASSET}" -o "${TMP_DIR}/${APP_NAME}"

if command -v sha256sum >/dev/null 2>&1; then
  if curl -fsSL "${BASE_URL}/SHA256SUMS.txt" -o "${TMP_DIR}/SHA256SUMS.txt" 2>/dev/null; then
    EXPECTED="$(grep "  ${ASSET}\$" "${TMP_DIR}/SHA256SUMS.txt" | awk '{print $1}' || true)"
    if [ -n "$EXPECTED" ]; then
      ACTUAL="$(sha256sum "${TMP_DIR}/${APP_NAME}" | awk '{print $1}')"
      if [ "$EXPECTED" != "$ACTUAL" ]; then
        echo "error: SHA-256 checksum mismatch for ${ASSET}" >&2
        exit 1
      fi
      echo "Checksum verified."
    fi
  fi
fi

chmod 0755 "${TMP_DIR}/${APP_NAME}"
mkdir -p "$INSTALL_DIR"
mv "${TMP_DIR}/${APP_NAME}" "${INSTALL_DIR}/${APP_NAME}"

echo "Installed ${APP_NAME} to ${INSTALL_DIR}/${APP_NAME}"
echo "Run 'sudo ${APP_NAME} install' to set up your first GitHub Actions runner."
