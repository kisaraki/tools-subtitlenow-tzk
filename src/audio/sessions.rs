//! Owned, provider-independent discovery data. No COM objects or capture handles cross here.

use std::{sync::Arc, time::Duration};

use thiserror::Error;

/// Upper bounds protect discovery memory even if an application creates excessive sessions.
pub const MAX_DEVICES: usize = 32;
pub const MAX_SESSIONS: usize = 256;
pub const MAX_WINDOWS: usize = 1024;
pub const MAX_TITLE_CHARS: usize = 512;
/// Polling updates session state and meters; creation notifications retain new sessions.
pub const DISCOVERY_INTERVAL: Duration = Duration::from_millis(250);
/// A UI activity hint (-60 dBFS), not voice detection or a guarantee of audible speakers.
pub const SIGNAL_THRESHOLD: f32 = 0.001;
pub const MAX_ICON_SIDE: u32 = 32;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AudioSourceId {
    pub endpoint: String,
    pub instance: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Active,
    Inactive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessScope {
    SingleProcess,
    /// The PID identifies the original creator, not every participant.
    MultipleProcesses,
    SystemSounds,
}

/// Premultiplied BGRA pixels, top-down, owned in RAM. No native handle escapes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationIcon {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
impl ApplicationIcon {
    /// Only bounded premultiplied BGRA can enter a native renderer.
    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= MAX_ICON_SIDE
            && self.height <= MAX_ICON_SIDE
            && self.bgra.len() == self.width as usize * self.height as usize * 4
            && self
                .bgra
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3]))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioSource {
    pub id: AudioSourceId,
    pub endpoint_name: Option<String>,
    pub process_id: u32,
    /// Query-only FILETIME creation identity, RAM-only. Unknown identity is not selectable.
    pub process_created: Option<u64>,
    pub process_scope: ProcessScope,
    pub application_name: Option<String>,
    /// A directly owned visible window, not a browser tab or an inferred parent process.
    pub window_title: Option<String>,
    pub icon: Option<Arc<ApplicationIcon>>,
    pub state: SessionState,
    /// Native session peak, normalized to 0..=1. None means unavailable, not silence.
    pub peak: Option<f32>,
    pub volume: Option<f32>,
    pub muted: Option<bool>,
}

impl AudioSource {
    /// Reports measured signal separately from the Windows stream-running state.
    pub fn signal_present(&self) -> Option<bool> {
        if self.state == SessionState::Inactive {
            return Some(false);
        }
        self.peak.map(|peak| peak > SIGNAL_THRESHOLD)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanReport {
    pub sources: Vec<AudioSource>,
    pub device_count: usize,
    pub unavailable_devices: usize,
    pub skipped_sessions: usize,
    pub windows_unavailable: bool,
    pub notifications_lost: bool,
    pub limit_reached: bool,
}

impl ScanReport {
    pub fn is_partial(&self) -> bool {
        self.unavailable_devices > 0
            || self.skipped_sessions > 0
            || self.windows_unavailable
            || self.notifications_lost
            || self.limit_reached
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveryUpdate {
    /// Unique within this process, separate from later Live/Prepared session identities.
    pub run_id: u64,
    pub sequence: u64,
    pub elapsed: Duration,
    pub result: Result<ScanReport, DiscoveryError>,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum DiscoveryError {
    #[error("Windows 音訊探索失敗（{operation}，代碼 0x{code:08X}）。")]
    Native { operation: &'static str, code: i32 },
    #[error("Windows 傳回無效的音訊來源資料（{0}）。")]
    InvalidData(&'static str),
    #[error("無法建立音訊探索執行緒。")]
    WorkerStart,
    #[error("音訊探索執行緒已意外停止。")]
    WorkerStopped,
    #[error("音訊探索執行緒發生非預期錯誤。")]
    WorkerPanicked,
    #[error("音訊探索已取消。")]
    Cancelled,
}

/// Reject invalid measurements instead of inventing a usable meter reading.
pub fn normalized_level(value: f32) -> Option<f32> {
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}
