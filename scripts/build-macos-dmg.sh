#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
MACOS_CONFIG="${PROJECT_DIR}/src-tauri/tauri.macos.conf.json"
APP_PATH="${PROJECT_DIR}/target/release/bundle/macos/DisplayMux.app"
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
[[ -f "${MACOS_CONFIG}" ]] || fail "找不到 ${MACOS_CONFIG}。"

VERSION="$(node -p "JSON.parse(require('fs').readFileSync('${PROJECT_DIR}/src-tauri/tauri.conf.json', 'utf8')).version")"
DMG_PATH="${DMG_DIR}/DisplayMux_${VERSION}_$(uname -m).dmg"

printf '同步前端相依套件...\n'
cd "${PROJECT_DIR}"
pnpm install --frozen-lockfile

printf '使用 %s 建置 DisplayMux.app...\n' "${MACOS_CONFIG}"
pnpm tauri build --config "${MACOS_CONFIG}" --bundles app
[[ -d "${APP_PATH}" ]] || fail "建置命令完成，但找不到 ${APP_PATH}。"

# Without an Apple Developer identity, rustc leaves a linker-level ad-hoc
# signature on the executable. Seal the complete app bundle so Gatekeeper can
# validate its resources after it is copied out of the DMG.
printf '建立本機 ad-hoc 簽章...\n'
codesign --force --deep --sign - "${APP_PATH}"
codesign --verify --deep --strict "${APP_PATH}"

# Tauri's styled DMG helper uses Finder AppleScript and can block on machines
# without Automation permission. hdiutil creates the same drag-to-Applications
# installer without requiring GUI access.
STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/displaymux-dmg.XXXXXX")"
cleanup() {
  rm -rf "${STAGING_DIR}"
}
trap cleanup EXIT

cp -R "${APP_PATH}" "${STAGING_DIR}/DisplayMux.app"
ln -s /Applications "${STAGING_DIR}/Applications"
mkdir -p "${DMG_DIR}"

printf '封裝 DisplayMux DMG...\n'
hdiutil create -quiet -volname DisplayMux -srcfolder "${STAGING_DIR}" -format UDZO -ov "${DMG_PATH}"
hdiutil verify "${DMG_PATH}" >/dev/null

printf '\n建置完成：%s\n' "${DMG_PATH}"
