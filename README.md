# DisplayMux

DisplayMux 是適用於 Windows 10/11 與 macOS 12+ 的 Tauri 2 桌面工具。它只控制使用者指定的「共用螢幕」，不會切換所有螢幕，也不會變更作業系統的多螢幕排列。

可使用的螢幕品牌、型號、輸入埠與主機數量都不是寫死的。只要螢幕支援 DDC/CI，使用者即可從實際偵測結果選擇共用螢幕，並為每台 Windows 或 Mac 設定各自連接的輸入來源。

例如，一台 Windows 同時使用共用螢幕 A 與專用螢幕 B，而多台 Mac 或其他 Windows 也接在螢幕 A 上時，DisplayMux 只會切換螢幕 A；螢幕 B 會保持原狀。

## 核心原則

- 從 DDC/CI 偵測結果選擇唯一的共用螢幕，不以品牌或型號作為固定預設。
- 以完整 EDID 指紋辨識目標，未被選取的螢幕永遠不會收到切換指令。
- 每台主機各自設定其使用的輸入來源，可支援螢幕實際提供的多個輸入埠。
- Windows、Mac、本機與遠端都只是主機角色，不限制特定作業系統必須使用特定輸入埠。
- 找不到唯一目標或目的主機尚未就緒時停止切換，避免誤切其他螢幕或只顯示黑畫面。

## 一般使用者設定流程

1. 在需要參與切換的每台 Windows 或 Mac 安裝並啟動 DisplayMux。
2. 在共用螢幕的 OSD 選單中開啟 DDC/CI。
3. 在設定頁執行螢幕偵測，從實際列出的品牌、型號與識別資訊中選擇一台共用螢幕。
4. 為本機指定它連接到共用螢幕的輸入來源。
5. 在「附近的 DisplayMux 主機」依電腦名稱加入同一區網內的其他主機，輸入相同的配對密碼，並為各主機指定其輸入來源。
6. 在所有參與切換的主機重複設定，並保持 Agent 在登入後自動啟動。

IP、MAC 位址與 Agent Port 會透過 Bonjour／mDNS 自動探索並保存，通常不需要手動輸入。DHCP 位址改變時，再次搜尋即可更新主機資訊。配對密碼至少需要 8 個字元，而且所有要互相控制的主機必須使用相同密碼。

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

1. 確認目的主機的 DisplayMux Agent 是否可連線。
2. 若無回應，向配對時自動取得的 MAC 與廣播位址送出 Wake-on-LAN。
3. 等待目的主機 Agent 就緒，預設最多 45 秒。
4. 由可存取共用螢幕 DDC/CI 的 Agent 切換到目的主機設定的輸入來源。
5. 若目前主機的連接路徑不傳送 DDC，則請另一台可用且已配對的 Agent 代為切換。
6. 若目的主機未就緒或無法唯一辨識共用螢幕，保持目前輸入不變。

已配對主機睡眠時可能不會出現在即時搜尋結果，但 DisplayMux 仍會使用上次配對時保存的位址與 MAC 嘗試喚醒。完整關機後能否喚醒取決於硬體、韌體與作業系統，DisplayMux 無法保證。

## 平台與連接方式

Windows 會透過系統 DDC/CI 介面列舉實際可控制的螢幕。macOS adapter 也會在執行時列舉實際提供 DDC 的顯示器，適用的連接方式包括：

- Mac mini 內建 HDMI 直連
- MacBook Air／Pro 的 USB-C 或 Thunderbolt 至 DisplayPort
- MacBook Air／Pro 的 USB-C 或 Thunderbolt 轉 HDMI

macOS 能否控制 DDC 仍取決於 Mac 晶片世代、macOS 版本、轉接器、擴充座與線材是否完整轉送 DDC。DisplayMux 會顯示實際偵測結果；連接路徑不可用時不會回報切換成功。

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

## 自動更新與發佈

DisplayMux 啟動後可檢查公開的 GitHub Releases，但不會靜默下載或安裝。發現新版本時會顯示版本與 release notes，必須由使用者按下「下載並安裝」；Rust 後端會先驗證 Tauri updater 簽章，成功後才執行安裝與重新啟動。

目前 Repository 已設為 Public，因此應用程式不需 GitHub 帳號或 Personal Access Token 即可讀取已發布的 Release 更新資訊。Draft Release 不會提供給一般使用者；維護者必須完成檢查並正式發布後，客戶端才會偵測到新版本。

建立正式版本前，需同步更新 `package.json`、`src-tauri/Cargo.toml` 與 `src-tauri/tauri.conf.json` 的 SemVer，再建立相同版本的 tag：

```powershell
git tag v0.2.0
```

推送 tag 後，`.github/workflows/release.yml` 會依序建置 Windows x64、macOS Apple Silicon 與 macOS Intel，執行測試、簽署 updater artifacts，並建立 draft Release。

一般貢獻者執行 `pnpm tauri build` 不需要 updater 私鑰；只有 release workflow 會合併 `src-tauri/tauri.release.conf.json` 並建立可供自動更新的簽章 artifacts。

安全注意事項：

- `TAURI_SIGNING_PRIVATE_KEY` 只保存於 GitHub Actions Secrets 與維護者的離線備份，絕對不能提交到 Git。
- updater 私鑰一旦遺失，既有安裝將無法驗證之後的新金鑰；一旦外洩，必須視為重大供應鏈事件。
- GitHub Actions 固定到完整 commit SHA，簽章 Secret 只提供給 release build 步驟。
- 發布前應檢查 Release artifacts、簽章與 release notes，並避免在內容中加入主機名稱、內網位址或其他環境資訊。
