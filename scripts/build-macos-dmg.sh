#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
TAURI_CONFIG="${PROJECT_DIR}/src-tauri/tauri.conf.json"
MACOS_CONFIG="${PROJECT_DIR}/src-tauri/tauri.macos.conf.json"
DMG_DIR="${PROJECT_DIR}/target/release/bundle/dmg"

fail() {
  printf '錯誤：%s\n' "$1" >&2
  exit 1
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "DMG 只能在 macOS 上建置。"
fi

if [[ -f "${CARGO_HOME:-${HOME}/.cargo}/env" ]]; then
  # rustup installs this environment file, but non-interactive shells may not load it.
  # shellcheck disable=SC1091
  source "${CARGO_HOME:-${HOME}/.cargo}/env"
fi

command -v xcrun >/dev/null 2>&1 || fail "找不到 Xcode Command Line Tools，請先執行 xcode-select --install。"
command -v codesign >/dev/null 2>&1 || fail "找不到 macOS codesign 工具。"
command -v hdiutil >/dev/null 2>&1 || fail "找不到 macOS hdiutil 工具。"
command -v cargo >/dev/null 2>&1 || fail "找不到 Cargo，請先從 https://rustup.rs 安裝 Rust 1.85 以上版本。"
command -v node >/dev/null 2>&1 || fail "找不到 Node.js，請先安裝 Node.js 22 以上版本。"
command -v pnpm >/dev/null 2>&1 || fail "找不到 pnpm，請先安裝 pnpm 10 以上版本。"
[[ -f "${TAURI_CONFIG}" ]] || fail "找不到 ${TAURI_CONFIG}。"
[[ -f "${MACOS_CONFIG}" ]] || fail "找不到 ${MACOS_CONFIG}。"

read_tauri_config() {
  node -e '
    const fs = require("node:fs");
    const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
    const value = config[process.argv[2]];
    if (typeof value !== "string" || value.length === 0) process.exit(1);
    process.stdout.write(value);
  ' "${TAURI_CONFIG}" "$1"
}

PRODUCT_NAME="$(read_tauri_config productName)" || fail "tauri.conf.json 缺少 productName。"
VERSION="$(read_tauri_config version)" || fail "tauri.conf.json 缺少 version。"
APP_PATH="${PROJECT_DIR}/target/release/bundle/macos/${PRODUCT_NAME}.app"

case "$(uname -m)" in
  arm64) BUNDLE_ARCH="aarch64" ;;
  x86_64) BUNDLE_ARCH="x64" ;;
  *) fail "不支援的 macOS CPU 架構：$(uname -m)。" ;;
esac

DMG_NAME="${PRODUCT_NAME}_${VERSION}_${BUNDLE_ARCH}.dmg"
DMG_PATH="${DMG_DIR}/${DMG_NAME}"
TEMP_DMG_PATH="${DMG_DIR}/.${DMG_NAME}.partial.dmg"
STAGING_DIR=""

cleanup() {
  if [[ -n "${STAGING_DIR}" && -d "${STAGING_DIR}" ]]; then
    rm -rf "${STAGING_DIR}"
  fi
  if [[ -f "${TEMP_DMG_PATH}" ]]; then
    rm -f "${TEMP_DMG_PATH}"
  fi
}
trap cleanup EXIT INT TERM

printf '同步前端相依套件...\n'
cd "${PROJECT_DIR}"
pnpm install --frozen-lockfile

printf '使用 %s 建置 %s.app...\n' "${MACOS_CONFIG}" "${PRODUCT_NAME}"
pnpm tauri build --config "${MACOS_CONFIG}" --bundles app
[[ -d "${APP_PATH}" ]] || fail "建置命令完成，但找不到 ${APP_PATH}。"

# Seal the complete bundle. Use APPLE_SIGNING_IDENTITY when provided; otherwise
# create a local ad-hoc signature suitable for development installation.
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"
if [[ "${SIGNING_IDENTITY}" == "-" ]]; then
  printf '建立本機 ad-hoc 簽章...\n'
else
  printf '使用指定的 Apple signing identity 簽章...\n'
fi
codesign --force --deep --sign "${SIGNING_IDENTITY}" "${APP_PATH}"
codesign --verify --deep --strict "${APP_PATH}"

STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/displaymux-dmg.XXXXXX")"
cp -R "${APP_PATH}" "${STAGING_DIR}/${PRODUCT_NAME}.app"
ln -s /Applications "${STAGING_DIR}/Applications"
mkdir -p "${DMG_DIR}"

printf '封裝 %s...\n' "${DMG_NAME}"
hdiutil create \
  -quiet \
  -volname "${PRODUCT_NAME}" \
  -srcfolder "${STAGING_DIR}" \
  -format UDZO \
  -ov \
  "${TEMP_DMG_PATH}"
hdiutil verify "${TEMP_DMG_PATH}" >/dev/null
mv -f "${TEMP_DMG_PATH}" "${DMG_PATH}"

printf '\n建置完成：%s\n' "${DMG_PATH}"
