# DisplayMux Host Switcher Prototype · Product Facts

> 紀錄日期：2026-09-14

- DisplayMux 是目前工作區中的 Tauri 2.0 跨平台桌面應用程式，目標平台包含 Windows 與 macOS。
- 現有產品畫面以深墨綠側欄、低彩度灰綠工作區、白色內容面板與綠色互動狀態為主要視覺語言。
- 現有主機模型包含本機 Windows 電腦與已連線的 macOS 主機，主機切換操作已存在於切換中心。
- Tauri 官方 Global Shortcut plugin 支援 Windows 與 macOS，可在正式功能階段用於註冊使用者自訂的系統全域快捷鍵。
- macOS 的 Tab 焦點巡覽會受到系統「鍵盤導覽」設定影響，因此主機切換器需要自行處理 Tab、Shift + Tab、Enter 與 Escape，而不能只依賴瀏覽器預設焦點順序。

## 原型邊界

- 此階段只驗證畫面、文案與鍵盤／滑鼠流程。
- 尚未註冊作業系統全域快捷鍵，也不會真的切換螢幕輸入。
