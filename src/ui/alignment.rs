//! Bounded explicit alignment commands and content-free status text.
use crate::session::aligned_playback::{AlignedPlaybackState, RealignmentReason};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignmentAction {
    Pause,
    Resume,
    Reset,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct AlignmentCommand {
    pub running: bool,
    pub reset: u64,
}
impl AlignmentCommand {
    pub(crate) fn apply(&mut self, action: AlignmentAction) -> bool {
        match action {
            AlignmentAction::Pause => self.running = false,
            AlignmentAction::Resume => self.running = true,
            AlignmentAction::Reset => {
                let Some(next) = self.reset.checked_add(1) else {
                    return false;
                };
                self.reset = next;
            }
        }
        true
    }
}
pub fn label(state: AlignedPlaybackState) -> String {
    match state {
        AlignedPlaybackState::AdoptionFailed(error) => format!("字幕定位：{error}；使用即時翻譯"),
        AlignedPlaybackState::Paused => "字幕定位：已暫停；播放器由使用者操作".into(),
        AlignedPlaybackState::Collecting(count) => {
            format!("字幕定位：正在蒐集獨立語音觀測（{count}/3）")
        }
        AlignedPlaybackState::Reacquiring(reason) => format!(
            "字幕定位：{}，正在重新校時",
            match reason {
                RealignmentReason::SeekForward => "偵測向前跳轉",
                RealignmentReason::SeekBackward => "偵測向後跳轉／重啟",
                RealignmentReason::AudioGap => "擷取時間中斷",
                RealignmentReason::Drift => "小幅速率漂移",
                RealignmentReason::ExplicitReset => "手動重設",
                RealignmentReason::Discontinuity => "版本或時間不連續",
            }
        ),
        AlignedPlaybackState::Positioned {
            position,
            offset_us,
            ..
        } => {
            let seconds = position.media_us / 1_000_000;
            format!(
                "字幕定位：最近語音位置 {:02}:{:02}.{:03} · 字幕偏移 {:+} ms；未以牆鐘推進",
                seconds / 60,
                seconds % 60,
                position.media_us / 1000 % 1000,
                offset_us / 1000
            )
        }
        AlignedPlaybackState::Unmatched => "字幕定位：尚無唯一配對，使用即時翻譯".into(),
        AlignedPlaybackState::Failed(error) => format!("字幕定位：{error}"),
        AlignedPlaybackState::Stopped => "字幕定位：已停止；保留即時翻譯".into(),
    }
}
