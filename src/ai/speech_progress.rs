//! Fixed numeric counters only. No audio/text, locks, tasks or playback evidence.
use super::speech::StreamIdentity;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum ProgressCount {
    AudioReceived,
    AudioSent,
    Messages,
    Partials,
    Finals,
    Emitted,
    LanguageEnds,
    LocalLanguageUsed,
    LanguageSkipped,
    AudioEnds,
    Boosted,
    EarlyHeld,
    PrefixBlocks,
}
const COUNTERS: usize = ProgressCount::PrefixBlocks as usize + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum LanguageBasis {
    Unconfirmed = 0,
    Provider = 1,
    LocalText = 2,
}

pub(crate) struct SpeechProgress {
    identity: StreamIdentity,
    counts: [AtomicU64; COUNTERS],
    waiting_language: AtomicBool,
    language_basis: AtomicU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeechProgressSnapshot {
    pub identity: StreamIdentity,
    pub audio_received: u64,
    pub audio_sent: u64,
    pub messages: u64,
    pub partials: u64,
    pub finals: u64,
    pub emitted: u64,
    pub language_ends: u64,
    pub audio_ends: u64,
    pub boosted: u64,
    pub early_held: u64,
    pub prefix_blocks: u64,
    pub local_language_used: u64,
    pub language_skipped: u64,
    pub language_basis: LanguageBasis,
    pub waiting_language: bool,
}
impl SpeechProgress {
    pub(crate) fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            waiting_language: AtomicBool::new(false),
            language_basis: AtomicU64::new(LanguageBasis::Unconfirmed as u64),
        }
    }
    pub(crate) fn increment(&self, count: ProgressCount) {
        let _ = self.counts[count as usize].fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |value| Some(value.saturating_add(1)),
        );
    }
    pub(crate) fn waiting_language(&self, waiting: bool) {
        self.waiting_language.store(waiting, Ordering::Relaxed);
    }
    pub(crate) fn language_basis(&self, basis: LanguageBasis) {
        self.language_basis.store(basis as u64, Ordering::Relaxed);
    }
    pub(crate) fn snapshot(&self) -> SpeechProgressSnapshot {
        let read = |count: ProgressCount| self.counts[count as usize].load(Ordering::Relaxed);
        SpeechProgressSnapshot {
            identity: self.identity,
            audio_received: read(ProgressCount::AudioReceived),
            audio_sent: read(ProgressCount::AudioSent),
            messages: read(ProgressCount::Messages),
            partials: read(ProgressCount::Partials),
            finals: read(ProgressCount::Finals),
            emitted: read(ProgressCount::Emitted),
            language_ends: read(ProgressCount::LanguageEnds),
            audio_ends: read(ProgressCount::AudioEnds),
            boosted: read(ProgressCount::Boosted),
            early_held: read(ProgressCount::EarlyHeld),
            prefix_blocks: read(ProgressCount::PrefixBlocks),
            local_language_used: read(ProgressCount::LocalLanguageUsed),
            language_skipped: read(ProgressCount::LanguageSkipped),
            language_basis: match self.language_basis.load(Ordering::Relaxed) {
                1 => LanguageBasis::Provider,
                2 => LanguageBasis::LocalText,
                _ => LanguageBasis::Unconfirmed,
            },
            waiting_language: self.waiting_language.load(Ordering::Relaxed),
        }
    }
}
impl SpeechProgressSnapshot {
    pub fn label(&self) -> String {
        let stage = if self.audio_received == 0 {
            "尚未收到所選來源音訊；請確認來源正在播放。"
        } else if self.audio_sent == 0 {
            "已收到音訊，尚未完成送出。"
        } else if self.early_held > 0 && self.partials == 0 && self.finals == 0 {
            "已收到提前轉錄；等待服務提交確認以核對音訊身分。"
        } else if self.partials == 0 && self.finals == 0 {
            "音訊已送出，尚未收到服務轉錄；送出不代表辨識成功。"
        } else if self.waiting_language {
            "已收到轉錄，正在等待可核對的來源語言。"
        } else if self.emitted == 0 && self.language_skipped > 0 {
            "本機尚無可採用的語言結果；已略過不確定段落，可手選來源語言。"
        } else if self.emitted == 0 {
            "已收到轉錄，尚未交付字幕處理。"
        } else {
            "已交付辨識結果；翻譯與顯示仍依目前工作階段處理。"
        };
        format!(
            "辨識診斷：收到音訊 {} 包／送出音訊訊息 {} 則；前綴音訊 {} 包；服務訊息 {}；暫定 {}／定稿 {}／交付 {}；提交確認前暫存 {} 則；語言收尾要求 {}；本機音訊收尾要求 {}；低音量增益 {} 包；本機判定採用 {}；語言未確認略過 {} 段\r\n服務／估計語言依據：{}\r\n辨識階段：{stage}",
            self.audio_received,
            self.audio_sent,
            self.prefix_blocks,
            self.messages,
            self.partials,
            self.finals,
            self.emitted,
            self.early_held,
            self.language_ends,
            self.audio_ends,
            self.boosted,
            self.local_language_used,
            self.language_skipped,
            match self.language_basis {
                LanguageBasis::Unconfirmed => "尚未確認",
                LanguageBasis::Provider => "服務語言標記",
                LanguageBasis::LocalText => "本機文字估計（可手選來源語言）",
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::SessionId;
    #[test]
    fn phase11_progress_is_numeric_saturating_and_session_scoped() {
        let identity = StreamIdentity {
            session: SessionId(400),
            generation: 1,
        };
        let progress = SpeechProgress::new(identity);
        assert!(progress.snapshot().label().contains("尚未收到所選來源音訊"));
        progress.increment(ProgressCount::AudioReceived);
        assert!(progress.snapshot().label().contains("尚未完成送出"));
        progress.increment(ProgressCount::AudioSent);
        progress.increment(ProgressCount::Messages);
        assert!(progress.snapshot().label().contains("尚未收到服務轉錄"));
        progress.increment(ProgressCount::EarlyHeld);
        assert!(progress.snapshot().label().contains("等待服務提交確認"));
        assert!(progress.snapshot().label().contains("提交確認前暫存 1 則"));
        assert_eq!(SpeechProgress::new(identity).snapshot().early_held, 0);
        progress.increment(ProgressCount::Partials);
        progress.waiting_language(true);
        assert!(progress.snapshot().label().contains("等待可核對的來源語言"));
        progress.waiting_language(false);
        progress.increment(ProgressCount::Emitted);
        assert!(progress.snapshot().label().contains("已交付辨識結果"));
        progress.increment(ProgressCount::LanguageEnds);
        assert_eq!(progress.snapshot().language_ends, 1);
        assert!(progress.snapshot().label().contains("語言收尾要求 1"));
        progress.increment(ProgressCount::AudioEnds);
        assert_eq!(progress.snapshot().audio_ends, 1);
        assert!(progress.snapshot().label().contains("本機音訊收尾要求 1"));
        progress.increment(ProgressCount::Boosted);
        assert_eq!(progress.snapshot().boosted, 1);
        assert!(progress.snapshot().label().contains("低音量增益 1 包"));
        progress.increment(ProgressCount::PrefixBlocks);
        assert_eq!(progress.snapshot().prefix_blocks, 1);
        assert!(progress.snapshot().label().contains("前綴音訊 1 包"));
        assert_eq!(SpeechProgress::new(identity).snapshot().prefix_blocks, 0);
        progress.language_basis(LanguageBasis::LocalText);
        progress.increment(ProgressCount::LocalLanguageUsed);
        progress.increment(ProgressCount::LanguageSkipped);
        assert!(progress.snapshot().label().contains("本機文字估計"));
        assert_eq!(progress.snapshot().local_language_used, 1);
        assert_eq!(progress.snapshot().language_skipped, 1);
        progress.counts[ProgressCount::Emitted as usize].store(u64::MAX, Ordering::Relaxed);
        progress.increment(ProgressCount::Emitted);
        assert_eq!(progress.snapshot().emitted, u64::MAX);
        assert_eq!(SpeechProgress::new(identity).snapshot().emitted, 0);
    }
}
