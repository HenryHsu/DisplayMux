# DisplayMux

DisplayMux 是適用於 Windows 10/11 與 macOS 12+ 的 Tauri 2 桌面工具。它只控制使用者指定的「共用螢幕」，不會切換所有螢幕，也不會變更作業系統的多螢幕排列。

可使用的螢幕品牌、型號、輸入埠與主機數量都不是寫死的。只要螢幕支援 DDC/CI，使用者即可從實際偵測結果選擇共用螢幕，並為每台 Windows 或 Mac 設定各自連接的輸入來源。

例如，一台 Windows 同時使用共用螢幕 A 與專用螢幕 B，而多台 Mac 或其他 Windows 也接在螢幕 A 上時，DisplayMux 只會切換螢幕 A；螢幕 B 會保持原狀。

## 核心原則

- 從可實際讀取 DDC/CI 輸入的偵測結果選擇唯一共用螢幕；只有一台時自動保存，多台時一律由使用者選擇。
- 以完整 EDID 指紋辨識目標，未被選取的螢幕永遠不會收到切換指令。
- 每台主機各自設定其使用的輸入來源，可支援螢幕實際提供的多個輸入埠。
- Windows、Mac、本機與遠端都只是主機角色，不限制特定作業系統必須使用特定輸入埠。
- 單一切換動作會自動處理 Wake-on-LAN、Agent 就緒確認與 DDC/CI 路徑，不要求使用者選擇切換模式。

## 一般使用者設定流程

1. 在需要參與切換的每台 Windows 或 Mac 安裝並啟動 DisplayMux。
2. 在共用螢幕的 OSD 選單中開啟 DDC/CI。
3. 在設定頁執行螢幕偵測；若只有一台可控制螢幕會自動選取，若有多台則從實際列出的品牌、型號與識別資訊中手動選擇。
4. 為本機指定它連接到共用螢幕的輸入來源。
5. 在「附近的 DisplayMux 主機」依電腦名稱加入同一區網內的其他主機，輸入相同的配對密碼，並為各主機指定其輸入來源。
6. 在所有參與切換的主機重複設定，並保持 Agent 在登入後自動啟動。

IP、MAC 位址與 Agent Port 會透過 Bonjour／mDNS 自動探索並保存，通常不需要手動輸入。DHCP 位址改變時，再次搜尋即可更新主機資訊。配對密碼至少需要 8 個字元，而且所有要互相控制的主機必須使用相同密碼。

本機 DDC/CI 切換不需要 Internet、mDNS、配對密碼或另一台主機在線。網路 Agent 用於區域網路上的就緒確認、Wake-on-LAN，以及本機 DDC/CI 失敗後的已驗證遠端代切；Agent 不可用時會自動退回本機 DDC/CI。Windows 版按關閉或最小化會留在系統匣繼續執行背景服務；只有系統匣的「結束 DisplayMux」會真正結束程序。

## 共用螢幕與輸入來源

DisplayMux 使用 MCCS 的 VCP `0x60` 控制輸入來源。介面會把常見值顯示為容易辨識的名稱：

| VCP 值 | 常見名稱 |
| --- | --- |
| `0x0F` | DisplayPort 1 |
| `0x10` | DisplayPort 2 |
| `0x11` | HDMI 1 |
| `0x12` | HDMI 2 |

這些只是 MCCS 常見對照，不是 DisplayMux 的固定設定。不同螢幕可能使用其他值；未知但有效的值會顯示為「自訂輸入 (`0xNN`)」，使用者仍可選用。USB-C 沒有跨廠商一致的 VCP `0x60` 值，因此 DisplayMux 不會只憑數值猜測為 USB-C。

更換螢幕、線材或連接埠後，應重新執行偵測並確認共用螢幕及每台主機的輸入來源。DisplayMux 不會用舊型號或「主螢幕」的概念自動替代無法辨識的目標。

## 切換與喚醒流程

1. 切換至遠端主機時，先向配對時保存的 MAC 與廣播位址送出 Wake-on-LAN；切換至本機時略過喚醒。
2. 若配對與區域網路可用，確認目的主機的 DisplayMux Agent；必要時等待就緒，預設最多 45 秒。
3. 目的主機就緒或網路路徑不可用時，都由目前主機優先透過本機 DDC/CI 切換到指定輸入。
4. 只有本機 DDC/CI 失敗時，才嘗試請已配對且通過驗證的主機代為切換。
5. 無法確認 Agent 時不再要求使用者選擇另一種模式，而是自動使用本機 DDC/CI，並提示對端未就緒時可能暫時黑畫面。

已配對主機睡眠時可能不會出現在即時搜尋結果，但 DisplayMux 仍會使用上次配對時保存的位址與 MAC 嘗試喚醒。完整關機後能否喚醒取決於硬體、韌體與作業系統，DisplayMux 無法保證。

## 平台與連接方式

Windows 會透過系統 DDC/CI 介面列舉實際可控制的螢幕。macOS adapter 也會在執行時列舉實際提供 DDC 的顯示器，適用的連接方式包括：

- Mac mini 內建 HDMI 直連
- MacBook Air／Pro 的 USB-C 或 Thunderbolt 至 DisplayPort
- MacBook Air／Pro 的 USB-C 或 Thunderbolt 轉 HDMI

macOS 能否控制 DDC 仍取決於 Mac 晶片世代、macOS 版本、轉接器、擴充座與線材是否完整轉送 DDC。DisplayMux 會顯示實際偵測結果；連接路徑不可用時不會回報切換成功。

### macOS 擴充座與 USB 顯示晶片限制

外接螢幕能正常顯示畫面或被 macOS 偵測，不代表該連接路徑也提供 DDC/CI。DisplayMux 必須能從 macOS 取得對應的 DDC／I²C service，才能讀寫 MCCS VCP `0x60`：

- 原生 USB-C DisplayPort Alt Mode 或 Thunderbolt 至 DisplayPort 的直連路徑最有機會完整提供 DDC/CI。
- MST 擴充座不一定無法使用，但結果取決於晶片、韌體、連接拓撲及 macOS 能否正確辨識每台實體螢幕；應以 DisplayMux 實際讀取 VCP 的結果為準。
- DisplayLink 透過驅動程式壓縮 framebuffer，再經 USB 傳送到擴充座晶片；一般 macOS DDC API 不一定能取得這條路徑的實體 I²C service。DisplayLink Manager 即使可透過自有功能調整亮度或對比，也不代表第三方程式可以送出輸入切換 VCP `0x60`。
- Silicon Motion InstantView／SM76x／SM77x 也屬於 USB 虛擬顯示與壓縮傳輸。除非廠商驅動程式提供可用的 DDC API，DisplayMux 應視為影像可用但 DDC/CI 不可用。

上述限制無法靠重試、重新配對或修正顯示器排列順序補回不存在的 DDC 通道。若直連可控制、經擴充座只能顯示畫面，通常代表限制位於擴充座或其驅動程式。此時可改用原生 Thunderbolt／DisplayPort／HDMI 連接，或由另一台具有可用 DDC/CI 路徑的已配對主機代為切換。

建議的網路與電源設定：

- Agent TCP Port 預設為 `47653`，參與配對的主機應保持一致。
- 私人區域網路的防火牆需允許 `5353/UDP`（mDNS）與 `47653/TCP`（Agent）。
- macOS 可開啟「Wake for network access」。
- Windows 可在網卡與韌體設定中啟用 Wake-on-LAN。
- 若電腦有多張實體或虛擬網卡，可在「進階網路資訊」確認自動取得的 MAC 是否屬於實際連網介面。

## 安全與隱私

DisplayMux 採 fail-closed 設計：只有剛好一台顯示器符合完整 EDID 指紋時才允許切換。找不到序號、沒有相符裝置或同時出現多台相符裝置時都會停止操作，不會退而控制作業系統主螢幕或全部螢幕。

已配對主機之間的控制要求使用 HMAC-SHA256 驗證、30 秒有效期限與 nonce 重播防護。正常操作不會把配對密碼輸出到應用程式日誌。Wake-on-LAN Magic Packet 本身沒有身分驗證，因此只用於喚醒，不直接授權螢幕切換。

mDNS 搜尋只在區域網路內廣播服務資訊。自動更新檢查不會在請求內容中加入螢幕設定、電腦名稱、區網位址、配對密碼或 EDID 指紋；與一般 HTTPS 連線相同，GitHub 仍可取得來源 IP、User-Agent 等必要連線中繼資料。

## 開發與建置

需求：Rust 1.85+、Node.js 22+、pnpm 10+。macOS 建置另需 Xcode Command Line Tools。

### 多國語言

桌面介面目前提供英文（`en`）與繁體中文（`zh-TW`）。啟動時會依作業系統提供給 WebView 的語言偏好自動選擇；`zh-TW`、`zh-Hant`、`zh-HK` 與 `zh-MO` 使用繁體中文，其餘尚未支援的語言回退英文。前端字串集中在 `src/locales/`，共用語系偵測、fallback 與參數插值位於 `src/i18n.ts`。新增語言時應建立完整資源檔，不要在畫面元件中直接加入使用者可見字串。

```powershell
pnpm install --frozen-lockfile
pnpm tauri dev
```

只建置前端：

```powershell
pnpm build
```

完整驗證：

```powershell
cargo check --all-targets
cargo test --all-targets -- --nocapture
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

CLI 可用於診斷。請先用 `list` 取得實際顯示器識別資訊，再以 `--dry-run` 確認目標與輸入值：

```powershell
cargo run -p displaymux-cli -- list
cargo run -p displaymux-cli -- switch <manufacturer> <product> <serial|-> <input> --dry-run
```

只有確定要實際切換時才移除 `--dry-run`。

正式建立安裝包：

```powershell
pnpm tauri build
```

Windows 預設產生 NSIS `setup.exe`，macOS 預設產生 `.app` 與 `.dmg`。Windows MSI 需要額外啟用 Windows 的 VBSCRIPT optional feature，因此不列入預設 bundle。

macOS 可直接執行專用腳本；腳本會依 `pnpm-lock.yaml` 同步套件，明確套用
`src-tauri/tauri.macos.conf.json` 產生 `.app`，建立可在未設定 Apple Developer
憑證的本機上安裝測試之 ad-hoc 簽章，再封裝成 `.dmg`：

```bash
./scripts/build-macos-dmg.sh
```

也可透過 pnpm 執行相同腳本：

```bash
pnpm build:dmg
```

DMG 產物會位於 `target/release/bundle/dmg/`。

### macOS Gatekeeper 與未公證測試版

目前 macOS DMG 只有 ad-hoc 簽章，尚未使用 Apple Developer ID 正式簽章及 Apple notarization。從瀏覽器下載後，Gatekeeper 可能顯示「無法驗證開發者」或「Apple 無法檢查是否包含惡意軟體」，並阻擋第一次啟動。

只有在確認 DMG 來自本專案可信任的 GitHub Release、且檔案未遭竄改時，才應允許執行。建議優先使用 macOS 圖形介面：

1. 將 `DisplayMux.app` 拖曳到 `/Applications`，並嘗試開啟一次。
2. 開啟「系統設定」→「隱私權與安全性」。
3. 在「安全性」區域找到被阻擋的 DisplayMux，按下「仍要打開」。
4. 完成身分驗證後，再次確認開啟。macOS 會只為這個 App 保存例外。

若「仍要打開」沒有出現，而且已確認 App 來源可信，可在終端機只移除 DisplayMux 的 quarantine 屬性：

```bash
xattr -dr com.apple.quarantine /Applications/DisplayMux.app
open /Applications/DisplayMux.app
```

這不是系統範圍的白名單，也不應對不明來源的 App 或整個 `/Applications` 執行。Apple 的官方操作與風險說明請參考 [Open apps safely on your Mac](https://support.apple.com/102445)。

## 自動更新

DisplayMux 啟動後可檢查公開的 GitHub Releases，但不會靜默下載或安裝。發現新版本時會顯示版本與 release notes，必須由使用者按下「下載並安裝」；Rust 後端會先驗證 Tauri updater 簽章，成功後才執行安裝與重新啟動。
