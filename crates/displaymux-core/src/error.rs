use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DisplayMuxError {
    #[error("找不到設定的共用螢幕；未變更任何螢幕")]
    TargetNotFound,

    #[error("找到 {count} 台符合共用螢幕指紋的裝置；為避免誤控，已停止操作")]
    AmbiguousTarget { count: usize },

    #[error("找不到先前列舉的螢幕裝置：{0}")]
    MonitorNoLongerAvailable(String),

    #[error("無效的螢幕輸入值：0x{0:02X}")]
    InvalidInput(u32),

    #[error("無法辨識螢幕輸入值：{0}；請輸入 0x01 至 0xFF，或使用十進位 1 至 255")]
    InvalidInputCode(String),

    #[error("此平台尚未提供螢幕控制功能")]
    UnsupportedPlatform,

    #[error("MAC 位址格式無效：{0}")]
    InvalidMacAddress(String),

    #[error("無法送出網路喚醒封包：{0}")]
    WakeFailed(String),

    #[error("無法連線至另一台主機：{0}")]
    PeerUnavailable(String),

    #[error("另一台主機拒絕了未通過驗證的要求")]
    AuthenticationFailed,

    #[error("要求已過期或可能被重播")]
    StaleRequest,

    #[error("無法完成螢幕操作：{0}")]
    Backend(String),
}
