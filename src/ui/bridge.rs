//! Latest-only RAM mailbox, called by the timeline owner, never by audio capture.
//! Lifecycle and snapshot travel atomically, so coalescing cannot lose a session switch.
use crate::{
    ai::{provider::SpeechProvider, speech::StreamIdentity},
    subtitle::timeline::TranscriptTimeline,
    ui::content::{ContentDisclosure, UiContentState, UiContentStatus},
    ui::lyrics::{UiLyricsState, UiLyricsStatus},
    ui::presentation::TimelineSnapshot,
    ui::subtitles::{UiSubtitleState, UiSubtitleStatus},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const UI_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UiStatus {
    Idle,
    Starting,
    CheckingService,
    Stopping,
    Listening,
    Translating,
    UsingExistingText,
    Reconnecting,
    Stopped,
    Error,
}
impl UiStatus {
    /// Empty content must explain lifecycle clearing, not imply active listening.
    pub const fn empty_subtitle_label(self) -> &'static str {
        match self {
            Self::Error => "辨識或翻譯已中斷，請查看錯誤訊息",
            Self::Reconnecting => "正在重新連線，尚無字幕紀錄",
            Self::Stopping => "正在停止，尚無字幕紀錄",
            Self::Stopped => "已停止，尚無字幕紀錄",
            Self::Starting => "正在連線，尚未開始辨識",
            Self::CheckingService => "服務診斷中，未啟動音訊辨識",
            _ => "等待字幕",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "待命",
            Self::Starting => "準備連線",
            Self::CheckingService => "服務診斷（連線、不傳音訊）",
            Self::Stopping => "正在停止",
            Self::Listening => "正在聆聽",
            Self::Translating => "正在翻譯",
            Self::UsingExistingText => "使用既有字幕／歌詞",
            Self::Reconnecting => "重新連線",
            Self::Stopped => "已停止",
            Self::Error => "錯誤，請檢查來源或所選服務",
        }
    }
}
#[derive(Clone, Copy)]
pub struct UiSource {
    pub pid: u32,
    pub provider: SpeechProvider,
}
#[derive(Clone)]
pub struct UiFrame {
    pub(crate) render_ticket: Option<Arc<crate::performance::live::RenderTicket>>,
    pub alignment: Option<crate::session::aligned_playback::AlignedPlaybackState>,
    pub(crate) version: u64,
    pub identity: Option<StreamIdentity>,
    pub source: Option<UiSource>,
    pub live_model: Option<crate::ai::provider::LiveSpeechModel>,
    /// Query-only process birth identity for source-window placement; never an HWND source.
    pub process_created: Option<u64>,
    pub status: UiStatus,
    pub content: UiContentStatus,
    pub lyrics: UiLyricsStatus,
    pub subtitles: UiSubtitleStatus,
    pub translation_notice: Option<crate::session::live_translation::LiveTranslationNotice>,
    pub speech_progress: Option<crate::ai::speech_progress::SpeechProgressSnapshot>,
    pub snapshot: Option<Arc<TimelineSnapshot>>,
}
impl Default for UiFrame {
    fn default() -> Self {
        Self {
            render_ticket: None,
            alignment: None,
            version: 0,
            identity: None,
            source: None,
            live_model: None,
            process_created: None,
            status: UiStatus::Idle,
            content: UiContentStatus::default(),
            lyrics: UiLyricsStatus::default(),
            subtitles: UiSubtitleStatus::default(),
            translation_notice: None,
            speech_progress: None,
            snapshot: None,
        }
    }
}
impl UiFrame {
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn status_text(&self) -> String {
        let speech = match self.source {
            Some(source) => format!(
                "PID {} · {} · {} · {}",
                source.pid,
                match source.provider {
                    SpeechProvider::OpenAi => "OpenAI",
                    SpeechProvider::Gemini => "Gemini",
                },
                self.status.label(),
                match self.status {
                    UiStatus::Listening | UiStatus::Translating => "雲端處理使用所選服務",
                    UiStatus::UsingExistingText => "顯示已定位文字；即時辨識仍使用所選雲端服務",
                    UiStatus::Reconnecting => "等待同一服務重新連線",
                    UiStatus::Starting => "啟動後音訊與文字將送至所選服務",
                    UiStatus::Stopping => "正在取消要求與回收工作階段",
                    _ => "已停止傳送內容",
                }
            ),
            None if self.status == UiStatus::CheckingService => {
                "Gemini · 服務診斷中；未擷取或傳送音訊／字幕".into()
            }
            None => format!(
                "{} · 尚未選擇來源，未啟動擷取或雲端處理",
                self.status.label()
            ),
        };
        let speech = if let Some(model) = self.live_model {
            format!(
                "{speech}\r\n模型：{}{}",
                model.model(),
                if model.native_translation() {
                    " · 中文串流；原文未對應，合成語音不播放"
                } else {
                    ""
                }
            )
        } else {
            speech
        };
        let speech = match self.alignment {
            Some(state) => format!(
                "{speech}\r\n已準備字幕僅在本機定位；音訊辨識仍使用所選雲端。\r\n{}",
                crate::ui::alignment::label(state)
            ),
            None => speech,
        };
        let content = format!("{speech}\r\n{}", self.content.label());
        let lyrics = if self.lyrics.disclosure.is_some() {
            format!("{content}\r\n{}", self.lyrics.label())
        } else {
            content
        };
        let status = if self.subtitles.disclosure.is_some() {
            format!("{lyrics}\r\n{}", self.subtitles.label())
        } else {
            lyrics
        };
        let status = if let Some(progress) = self.speech_progress {
            format!(
                "{status}\r\n{}{}",
                if self.status == UiStatus::Error {
                    "已停止連線，以下為最後診斷取樣：\r\n"
                } else {
                    ""
                },
                progress.label()
            )
        } else {
            status
        };
        if let Some(notice) = self.translation_notice {
            let (heading, detail) =
                if notice.cause == crate::ai::translation::TranslationError::Incomplete {
                    ("翻譯未完成", "服務未完整回傳，此段未完成定稿")
                } else {
                    ("翻譯延遲", "要求逾時或已過期，已捨棄該次未完成要求")
                };
            format!(
                "{status}\r\n{heading}：第 {} 段{detail}；本次連線累計 {} 次。{}",
                notice.sequence,
                notice.skipped,
                if notice.exhausted() {
                    "已達連續失敗上限，本次連線停止處理。"
                } else if notice.consecutive == 0 {
                    "後續翻譯已恢復。"
                } else {
                    "仍處理後續新段落，不補送過時文字。"
                }
            )
        } else {
            status
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UiBridgeError {
    #[error("字幕視窗已關閉，顯示通知已停止。")]
    Closed,
    #[error("字幕來源或顯示版本無效。")]
    InvalidState,
}
#[derive(Clone)]
pub struct UiCancellation(watch::Sender<bool>);
impl UiCancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}
pub struct UiPublisher {
    diagnostics: Option<Arc<crate::performance::live::LiveDiagnostics>>,
    alignment_commands: watch::Receiver<crate::ui::alignment::AlignmentCommand>,
    sender: watch::Sender<Arc<UiFrame>>,
    frame: UiFrame,
    cancel: UiCancellation,
    last_snapshot: Option<Instant>,
    presented: watch::Sender<u64>,
    ready_version: Option<u64>,
}
pub struct UiInbox {
    alignment_commands: watch::Sender<crate::ui::alignment::AlignmentCommand>,
    receiver: watch::Receiver<Arc<UiFrame>>,
    cancel: UiCancellation,
    observed: Option<u64>,
    presented: watch::Sender<u64>,
}
pub fn channel() -> (UiPublisher, UiInbox) {
    let (alignment_sender, alignment_commands) =
        watch::channel(crate::ui::alignment::AlignmentCommand {
            running: true,
            reset: 0,
        });
    let (sender, receiver) = watch::channel(Arc::new(UiFrame::default()));
    let cancel = UiCancellation(watch::channel(false).0);
    let presented = watch::channel(0).0;
    (
        UiPublisher {
            diagnostics: None,
            alignment_commands,
            sender,
            frame: UiFrame::default(),
            cancel: cancel.clone(),
            last_snapshot: None,
            presented: presented.clone(),
            ready_version: None,
        },
        UiInbox {
            alignment_commands: alignment_sender,
            receiver,
            cancel,
            observed: None,
            presented,
        },
    )
}
impl UiPublisher {
    /// Dedicated source-free diagnostic state, only on a fresh owned channel.
    /// It cannot create an audio Session or transition an active Live pipeline.
    pub(crate) fn prepare_service_check(&mut self) -> Result<(), UiBridgeError> {
        self.ready()?;
        if self.frame.status != UiStatus::Idle
            || self.frame.source.is_some()
            || self.frame.identity.is_some()
        {
            return Err(UiBridgeError::InvalidState);
        }
        self.frame.status = UiStatus::CheckingService;
        self.send()
    }
    pub fn attach_diagnostics(
        &mut self,
        diagnostics: Option<Arc<crate::performance::live::LiveDiagnostics>>,
    ) -> Result<(), UiBridgeError> {
        if diagnostics
            .as_ref()
            .is_some_and(|d| self.frame.identity != Some(d.identity()))
        {
            return Err(UiBridgeError::InvalidState);
        }
        self.diagnostics = diagnostics;
        Ok(())
    }
    pub(crate) fn prepare_alignment(&mut self) -> Result<(), UiBridgeError> {
        self.ready()?;
        if self.frame.status != UiStatus::Starting || self.frame.identity.is_some() {
            return Err(UiBridgeError::InvalidState);
        }
        self.frame.alignment = Some(crate::session::aligned_playback::AlignedPlaybackState::Paused);
        self.send()?;
        self.ready_version = Some(self.frame.version);
        Ok(())
    }
    pub(crate) fn alignment_commands(
        &self,
    ) -> watch::Receiver<crate::ui::alignment::AlignmentCommand> {
        self.alignment_commands.clone()
    }
    pub fn publish_alignment(
        &mut self,
        identity: StreamIdentity,
        state: crate::session::aligned_playback::AlignedPlaybackState,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if self.frame.identity != Some(identity) {
            return Ok(false);
        }
        if self.frame.alignment == Some(state) {
            return Ok(false);
        }
        self.frame.alignment = Some(state);
        self.send()?;
        Ok(true)
    }
    /// Disclosure precedes any credential lookup or provider connection in the GUI owner.
    pub fn prepare_source(&mut self, source: UiSource) -> Result<(), UiBridgeError> {
        self.prepare_source_with_creation(source, None)
    }
    pub fn prepare_source_with_creation(
        &mut self,
        source: UiSource,
        created: Option<u64>,
    ) -> Result<(), UiBridgeError> {
        self.ready()?;
        if source.pid == 0 || created == Some(0) {
            return Err(UiBridgeError::InvalidState);
        }
        self.frame.identity = None;
        self.frame.alignment = None;
        self.frame.render_ticket = None;
        self.diagnostics = None;
        self.frame.source = Some(source);
        self.frame.live_model = None;
        self.frame.process_created = created;
        self.frame.snapshot = None;
        self.frame.status = UiStatus::Starting;
        self.frame.translation_notice = None;
        self.frame.speech_progress = None;
        self.frame.content = UiContentStatus::default();
        self.frame.lyrics = UiLyricsStatus::default();
        self.frame.subtitles = UiSubtitleStatus::default();
        self.last_snapshot = None;
        self.send()?;
        self.ready_version = Some(self.frame.version);
        Ok(())
    }
    pub fn prepare_live_model(
        &mut self,
        model: crate::ai::provider::LiveSpeechModel,
    ) -> Result<(), UiBridgeError> {
        self.ready()?;
        if self.frame.status != UiStatus::Starting
            || self
                .frame
                .source
                .is_none_or(|source| source.provider != model.provider())
        {
            return Err(UiBridgeError::InvalidState);
        }
        self.frame.live_model = Some(model);
        self.send()?;
        self.ready_version = Some(self.frame.version);
        Ok(())
    }
    /// The configured destination must be visible before constructing its Session
    /// provider. Advance the same acknowledgement gate as the source disclosure.
    pub fn prepare_content(
        &mut self,
        disclosure: Option<ContentDisclosure>,
    ) -> Result<(), UiBridgeError> {
        self.prepare_content_with_movie_hints(disclosure, false)
    }
    /// The additional dialogue/hint route must be acknowledged with its destination
    /// before constructing a Session provider. No raw hint values enter this frame.
    pub fn prepare_content_with_movie_hints(
        &mut self,
        disclosure: Option<ContentDisclosure>,
        movie_dialogue: bool,
    ) -> Result<(), UiBridgeError> {
        self.ready()?;
        if self.frame.status != UiStatus::Starting
            || self.frame.identity.is_some()
            || (movie_dialogue && disclosure.is_none())
        {
            return Err(UiBridgeError::InvalidState);
        }
        if let Some(disclosure) = disclosure {
            disclosure
                .validate()
                .map_err(|_| UiBridgeError::InvalidState)?;
        }
        self.frame.content = UiContentStatus {
            disclosure,
            movie_dialogue,
            state: UiContentState::Disabled,
        };
        self.send()?;
        self.ready_version = Some(self.frame.version);
        Ok(())
    }
    /// Publish the configured lookup destination before constructing its Session
    /// service. Lyrics and content disclosure share one visible acknowledgement gate.
    pub fn prepare_lyrics(
        &mut self,
        disclosure: Option<ContentDisclosure>,
    ) -> Result<(), UiBridgeError> {
        self.ready()?;
        if self.frame.status != UiStatus::Starting || self.frame.identity.is_some() {
            return Err(UiBridgeError::InvalidState);
        }
        if let Some(disclosure) = disclosure {
            disclosure
                .validate()
                .map_err(|_| UiBridgeError::InvalidState)?;
        }
        self.frame.lyrics = UiLyricsStatus {
            disclosure,
            state: UiLyricsState::Disabled,
        };
        self.send()?;
        self.ready_version = Some(self.frame.version);
        Ok(())
    }
    /// Subtitle reads share the source/service acknowledgement gate; no configured
    /// locations or candidate payloads enter this bounded disclosure.
    pub fn prepare_subtitles(
        &mut self,
        disclosure: Option<ContentDisclosure>,
    ) -> Result<(), UiBridgeError> {
        self.ready()?;
        if self.frame.status != UiStatus::Starting || self.frame.identity.is_some() {
            return Err(UiBridgeError::InvalidState);
        }
        if let Some(disclosure) = disclosure {
            disclosure
                .validate()
                .map_err(|_| UiBridgeError::InvalidState)?;
        }
        self.frame.subtitles = UiSubtitleStatus {
            disclosure,
            state: UiSubtitleState::Disabled,
        };
        self.send()?;
        self.ready_version = Some(self.frame.version);
        Ok(())
    }
    /// The native view acknowledges only after rendering a visible source disclosure.
    pub async fn wait_until_visible(&self) -> Result<(), UiBridgeError> {
        let version = self.ready_version.ok_or(UiBridgeError::InvalidState)?;
        let mut presented = self.presented.subscribe();
        loop {
            self.ready()?;
            if *presented.borrow_and_update() >= version {
                return Ok(());
            }
            tokio::select! {biased;_=self.cancel.cancelled()=>return Err(UiBridgeError::Closed),result=presented.changed()=>{result.map_err(|_|UiBridgeError::Closed)?;}}
        }
    }
    pub fn cancellation(&self) -> UiCancellation {
        self.cancel.clone()
    }
    fn ready(&self) -> Result<(), UiBridgeError> {
        if self.cancel.is_cancelled() || self.sender.is_closed() {
            Err(UiBridgeError::Closed)
        } else {
            Ok(())
        }
    }
    fn send(&mut self) -> Result<(), UiBridgeError> {
        self.frame.version = self
            .frame
            .version
            .checked_add(1)
            .ok_or(UiBridgeError::InvalidState)?;
        self.sender.send_replace(Arc::new(self.frame.clone()));
        Ok(())
    }
    pub fn begin(
        &mut self,
        identity: StreamIdentity,
        source: UiSource,
    ) -> Result<(), UiBridgeError> {
        self.ready()?;
        if source.pid == 0 || self.frame.identity == Some(identity) {
            return Err(UiBridgeError::InvalidState);
        }
        self.frame.alignment = None;
        self.frame.identity = Some(identity);
        self.frame.render_ticket = None;
        self.diagnostics = None;
        if self
            .frame
            .source
            .is_none_or(|previous| previous.pid != source.pid)
        {
            self.frame.process_created = None;
        }
        self.frame.source = Some(source);
        if self
            .frame
            .live_model
            .is_some_and(|model| model.provider() != source.provider)
        {
            self.frame.live_model = None;
        }
        self.frame.snapshot = None;
        self.frame.status = UiStatus::Listening;
        self.frame.translation_notice = None;
        self.frame.speech_progress = None;
        self.frame.content.state = if self.frame.content.disclosure.is_some() {
            UiContentState::Collecting
        } else {
            UiContentState::Disabled
        };
        self.frame.lyrics.state = if self.frame.lyrics.disclosure.is_some() {
            UiLyricsState::Waiting
        } else {
            UiLyricsState::Disabled
        };
        self.frame.subtitles.state = if self.frame.subtitles.disclosure.is_some() {
            UiSubtitleState::Waiting
        } else {
            UiSubtitleState::Disabled
        };
        self.last_snapshot = None;
        self.send()
    }
    /// Status is independent of the one shared subtitle snapshot. No resolver
    /// payload or candidate can become subtitle text through this method.
    pub fn publish_content(
        &mut self,
        identity: StreamIdentity,
        status: UiContentStatus,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if self.frame.identity != Some(identity) {
            return Ok(false);
        }
        if status.disclosure != self.frame.content.disclosure
            || status.movie_dialogue != self.frame.content.movie_dialogue
            || (status.movie_dialogue && status.disclosure.is_none())
            || (status.disclosure.is_none() && status.state != UiContentState::Disabled)
        {
            return Err(UiBridgeError::InvalidState);
        }
        if status == self.frame.content {
            return Ok(false);
        }
        self.frame.content = status;
        self.send()?;
        Ok(true)
    }
    /// Only a bounded lifecycle summary enters this frame. Lookup results cannot
    /// replace the single shared Live AI timeline or disclose an unknown service.
    pub fn publish_lyrics(
        &mut self,
        identity: StreamIdentity,
        status: UiLyricsStatus,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if self.frame.identity != Some(identity) {
            return Ok(false);
        }
        if status.disclosure != self.frame.lyrics.disclosure
            || (status.disclosure.is_none() && status.state != UiLyricsState::Disabled)
        {
            return Err(UiBridgeError::InvalidState);
        }
        if status == self.frame.lyrics {
            return Ok(false);
        }
        self.frame.lyrics = status;
        self.send()?;
        Ok(true)
    }
    /// A lookup summary cannot replace the shared timeline or disclose a service
    /// other than the one already presented for the active session.
    pub fn publish_subtitles(
        &mut self,
        identity: StreamIdentity,
        status: UiSubtitleStatus,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if self.frame.identity != Some(identity) {
            return Ok(false);
        }
        if status.disclosure != self.frame.subtitles.disclosure
            || (status.disclosure.is_none() && status.state != UiSubtitleState::Disabled)
        {
            return Err(UiBridgeError::InvalidState);
        }
        if status == self.frame.subtitles {
            return Ok(false);
        }
        self.frame.subtitles = status;
        self.send()?;
        Ok(true)
    }
    /// Caller samples once per second; latest numeric state only, no content or clock evidence.
    pub fn publish_speech_progress(
        &mut self,
        progress: crate::ai::speech_progress::SpeechProgressSnapshot,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if self.frame.identity != Some(progress.identity)
            || self.frame.speech_progress == Some(progress)
        {
            return Ok(false);
        }
        self.frame.speech_progress = Some(progress);
        self.send()?;
        Ok(true)
    }
    pub fn publish_translation_notice(
        &mut self,
        identity: StreamIdentity,
        notice: Option<crate::session::live_translation::LiveTranslationNotice>,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if self.frame.identity != Some(identity) || self.frame.translation_notice == notice {
            return Ok(false);
        }
        self.frame.translation_notice = notice;
        self.send()?;
        Ok(true)
    }
    /// Rate-limit timeline copies; force final/stable completion so the last update is retained.
    pub fn publish(
        &mut self,
        timeline: &TranscriptTimeline,
        force: bool,
    ) -> Result<bool, UiBridgeError> {
        self.ready()?;
        if Some(timeline.identity()) != self.frame.identity {
            return Ok(false);
        }
        if !force
            && self
                .last_snapshot
                .is_some_and(|instant| instant.elapsed() < UI_REFRESH_INTERVAL)
        {
            return Ok(false);
        }
        let version = self
            .frame
            .version
            .checked_add(1)
            .ok_or(UiBridgeError::InvalidState)?;
        let snapshot = Arc::new(TimelineSnapshot::from_timeline(timeline, version));
        self.frame.status = if matches!(
            snapshot.latest_origin(),
            Some(
                crate::resolver::contracts::SubtitleSource::ExistingSubtitle
                    | crate::resolver::contracts::SubtitleSource::ExistingLyrics
            )
        ) {
            UiStatus::UsingExistingText
        } else {
            UiStatus::Translating
        };
        self.frame.snapshot = Some(snapshot);
        self.frame.render_ticket = self.diagnostics.as_ref().and_then(|metrics| {
            timeline
                .items()
                .last()
                .and_then(|item| metrics.ticket(item))
        });
        self.last_snapshot = Some(Instant::now());
        self.send()?;
        Ok(true)
    }
    /// Terminal/reconnect state always removes private content; there is no history cache.
    pub fn clear(&mut self, status: UiStatus) -> Result<(), UiBridgeError> {
        self.ready()?;
        if !matches!(
            status,
            UiStatus::Reconnecting | UiStatus::Stopped | UiStatus::Error
        ) {
            return Err(UiBridgeError::InvalidState);
        }
        self.replace_terminal(status)
    }
    /// Completion can publish an empty terminal frame even after UI stop cancelled
    /// normal updates. It never restores content or revives a session.
    pub fn finish(&mut self, status: UiStatus) -> Result<(), UiBridgeError> {
        if !matches!(status, UiStatus::Stopped | UiStatus::Error) {
            return Err(UiBridgeError::InvalidState);
        }
        self.replace_terminal(status)
    }
    fn replace_terminal(&mut self, status: UiStatus) -> Result<(), UiBridgeError> {
        self.frame.render_ticket = None;
        self.diagnostics = None;
        self.frame.alignment = None;
        self.frame.identity = None;
        self.frame.snapshot = None;
        self.frame.status = status;
        self.frame.translation_notice = None;
        if status != UiStatus::Error {
            self.frame.speech_progress = None;
        }
        self.frame.content.stop();
        self.frame.lyrics.stop();
        self.frame.subtitles.stop();
        self.last_snapshot = None;
        self.send()
    }
}
impl Drop for UiPublisher {
    fn drop(&mut self) {
        self.frame.render_ticket = None;
        self.frame.alignment = None;
        self.frame.identity = None;
        self.frame.translation_notice = None;
        if self.frame.status != UiStatus::Error {
            self.frame.speech_progress = None;
        }
        self.frame.snapshot = None;
        self.frame.content.stop();
        self.frame.lyrics.stop();
        self.frame.subtitles.stop();
        if self.frame.status != UiStatus::Error {
            self.frame.status = UiStatus::Stopped;
        }
        self.frame.version = self.frame.version.saturating_add(1);
        self.sender.send_replace(Arc::new(self.frame.clone()));
    }
}
impl UiInbox {
    pub fn render_submitted(&self, version: u64) {
        let frame = self.receiver.borrow();
        if frame.version == version
            && let Some(ticket) = &frame.render_ticket
        {
            ticket.submitted();
        }
    }
    pub fn alignment_action(
        &self,
        action: crate::ui::alignment::AlignmentAction,
    ) -> Result<(), UiBridgeError> {
        if self.cancel.is_cancelled()
            || self.receiver.borrow().identity.is_none()
            || self.receiver.borrow().alignment.is_none()
        {
            return Err(UiBridgeError::InvalidState);
        }
        let mut accepted = true;
        self.alignment_commands
            .send_modify(|command| accepted = command.apply(action));
        if accepted {
            Ok(())
        } else {
            Err(UiBridgeError::InvalidState)
        }
    }
    pub(crate) fn acknowledge_visible(&self, version: u64) {
        self.presented.send_if_modified(|previous| {
            if version > *previous {
                *previous = version;
                true
            } else {
                false
            }
        });
    }
    /// Clone the Arc while borrowing, then release the watch lock before painting.
    /// Reading the slot directly also delivers the final frame after sender Drop.
    pub fn poll(&mut self) -> Option<Arc<UiFrame>> {
        let frame = self.receiver.borrow_and_update().clone();
        if self.observed == Some(frame.version) {
            return None;
        }
        self.observed = Some(frame.version);
        Some(frame)
    }
    pub fn cancellation(&self) -> UiCancellation {
        self.cancel.clone()
    }
}
impl Drop for UiInbox {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
#[path = "bridge_alignment_tests.rs"]
mod alignment_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audio::capture::SessionId, domain::DisplayMode, ui::presentation::Presentation};
    fn identity(id: u64) -> StreamIdentity {
        StreamIdentity {
            session: SessionId(id),
            generation: 1,
        }
    }
    fn source() -> UiSource {
        UiSource {
            pid: 42,
            provider: SpeechProvider::OpenAi,
        }
    }
    #[test]
    fn phase11_live_model_disclosure_is_vendor_scoped_and_reset_for_a_new_source() {
        use crate::ai::provider::LiveSpeechModel;
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        assert!(
            publisher
                .prepare_live_model(LiveSpeechModel::GeminiLiveTranslate)
                .is_err()
        );
        publisher
            .prepare_live_model(LiveSpeechModel::GptLiveTranscribe)
            .unwrap();
        assert!(
            inbox
                .poll()
                .unwrap()
                .status_text()
                .contains("gpt-live-transcribe")
        );
        let gemini = UiSource {
            pid: 47,
            provider: SpeechProvider::Gemini,
        };
        publisher.prepare_source(gemini).unwrap();
        assert_eq!(inbox.poll().unwrap().live_model, None);
        publisher
            .prepare_live_model(LiveSpeechModel::GeminiLiveTranslate)
            .unwrap();
        let frame = inbox.poll().unwrap();
        assert!(
            frame
                .status_text()
                .contains("gemini-3.5-live-translate-preview")
        );
        assert!(frame.status_text().contains("原文未對應"));
        publisher.begin(identity(1), source()).unwrap();
        assert_eq!(inbox.poll().unwrap().live_model, None);
    }
    #[test]
    fn phase11_speech_progress_is_latest_only_preserves_subtitles_and_rejects_retired_sessions() {
        use crate::ai::speech_progress::{ProgressCount, SpeechProgress};
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        let before = inbox.poll().unwrap().snapshot.clone().unwrap();
        let progress = SpeechProgress::new(identity(1));
        progress.increment(ProgressCount::AudioReceived);
        progress.increment(ProgressCount::AudioSent);
        progress.increment(ProgressCount::Partials);
        progress.waiting_language(true);
        let snapshot = progress.snapshot();
        assert!(publisher.publish_speech_progress(snapshot).unwrap());
        let frame = inbox.poll().unwrap();
        assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &before));
        assert!(frame.status_text().contains("等待可核對的來源語言"));
        assert!(!publisher.publish_speech_progress(snapshot).unwrap());
        assert!(
            !publisher
                .publish_speech_progress(SpeechProgress::new(identity(2)).snapshot())
                .unwrap()
        );
        publisher.finish(UiStatus::Error).unwrap();
        let frame = inbox.poll().unwrap();
        assert_eq!(frame.speech_progress, Some(snapshot));
        assert!(frame.status_text().contains("已停止連線"));
        publisher.begin(identity(2), source()).unwrap();
        assert!(inbox.poll().unwrap().speech_progress.is_none());
        assert!(!publisher.publish_speech_progress(snapshot).unwrap());
        assert!(
            publisher
                .publish_speech_progress(SpeechProgress::new(identity(2)).snapshot())
                .unwrap()
        );
        publisher.clear(UiStatus::Stopped).unwrap();
        assert!(inbox.poll().unwrap().speech_progress.is_none());
    }
    #[test]
    fn phase11_incomplete_translation_notice_distinguishes_cause_and_exhaustion() {
        use crate::ai::translation::TranslationError;
        use crate::session::live_translation::LiveTranslationNotice;
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        let before = inbox.poll().unwrap();
        let notice = LiveTranslationNotice {
            sequence: 12,
            skipped: 1,
            consecutive: 1,
            cause: TranslationError::Incomplete,
        };
        assert!(
            publisher
                .publish_translation_notice(identity(1), Some(notice))
                .unwrap()
        );
        let warning = inbox.poll().unwrap();
        assert!(warning.status == UiStatus::Translating);
        assert!(Arc::ptr_eq(
            before.snapshot.as_ref().unwrap(),
            warning.snapshot.as_ref().unwrap()
        ));
        assert!(warning.status_text().contains("翻譯未完成：第 12 段"));
        assert!(warning.status_text().contains("此段未完成定稿"));
        assert!(!warning.status_text().contains("要求逾時"));
        publisher
            .publish_translation_notice(
                identity(1),
                Some(LiveTranslationNotice {
                    consecutive: 3,
                    skipped: 3,
                    ..notice
                }),
            )
            .unwrap();
        let stopped = inbox.poll().unwrap().status_text();
        assert!(stopped.contains("已達連續失敗上限"));
        assert!(!stopped.contains("仍處理後續新段落"));
        publisher
            .publish_translation_notice(
                identity(1),
                Some(LiveTranslationNotice {
                    consecutive: 0,
                    ..notice
                }),
            )
            .unwrap();
        assert!(
            inbox
                .poll()
                .unwrap()
                .status_text()
                .contains("後續翻譯已恢復")
        );
    }

    #[test]
    fn phase11_translation_lateness_status_keeps_snapshot_and_rejects_old_session_notices() {
        use crate::ai::translation::TranslationError;
        use crate::session::live_translation::LiveTranslationNotice;
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        let timeline = TranscriptTimeline::new(identity(1));
        publisher.publish(&timeline, true).unwrap();
        let before = inbox.poll().unwrap();
        let notice = LiveTranslationNotice {
            sequence: 7,
            skipped: 1,
            consecutive: 1,
            cause: TranslationError::Timeout,
        };
        assert!(
            !publisher
                .publish_translation_notice(identity(2), Some(notice))
                .unwrap()
        );
        assert!(
            publisher
                .publish_translation_notice(identity(1), Some(notice))
                .unwrap()
        );
        let warning = inbox.poll().unwrap();
        assert!(Arc::ptr_eq(
            before.snapshot.as_ref().unwrap(),
            warning.snapshot.as_ref().unwrap()
        ));
        assert!(warning.status == UiStatus::Translating);
        assert!(warning.status_text().contains("第 7 段"));
        assert!(warning.status_text().contains("仍處理後續新段落"));
        let recovered = LiveTranslationNotice {
            consecutive: 0,
            ..notice
        };
        publisher
            .publish_translation_notice(identity(1), Some(recovered))
            .unwrap();
        assert!(
            inbox
                .poll()
                .unwrap()
                .status_text()
                .contains("後續翻譯已恢復")
        );
        publisher.clear(UiStatus::Reconnecting).unwrap();
        assert!(inbox.poll().unwrap().translation_notice.is_none());
        publisher.begin(identity(2), source()).unwrap();
        assert!(
            !publisher
                .publish_translation_notice(identity(1), Some(notice))
                .unwrap()
        );
        assert!(inbox.poll().unwrap().translation_notice.is_none());
    }
    fn content_disclosure() -> ContentDisclosure {
        ContentDisclosure {
            service: "明確測試內容服務",
            transport: crate::ui::content::ContentTransport::Remote,
        }
    }
    fn lyrics_disclosure() -> ContentDisclosure {
        ContentDisclosure {
            service: "明確測試歌詞服務",
            transport: crate::ui::content::ContentTransport::Remote,
        }
    }
    fn subtitle_disclosure() -> ContentDisclosure {
        ContentDisclosure {
            service: "明確測試字幕服務",
            transport: crate::ui::content::ContentTransport::Remote,
        }
    }
    #[tokio::test]
    async fn subtitle_disclosure_advances_the_shared_visible_gate() {
        let (mut publisher, mut inbox) = channel();
        assert_eq!(
            publisher.prepare_subtitles(Some(subtitle_disclosure())),
            Err(UiBridgeError::InvalidState)
        );
        publisher.prepare_source(source()).unwrap();
        let initial = inbox.poll().unwrap();
        assert!(!initial.status_text().contains("字幕查詢"));
        inbox.acknowledge_visible(initial.version);
        for service in ["", "\n", "unknown\tservice"] {
            assert_eq!(
                publisher.prepare_subtitles(Some(ContentDisclosure {
                    service,
                    ..subtitle_disclosure()
                })),
                Err(UiBridgeError::InvalidState)
            );
            assert!(inbox.poll().is_none());
        }
        publisher
            .prepare_content(Some(content_disclosure()))
            .unwrap();
        publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
        let previous = inbox.poll().unwrap();
        inbox.acknowledge_visible(previous.version);
        publisher
            .prepare_subtitles(Some(subtitle_disclosure()))
            .unwrap();
        inbox.acknowledge_visible(previous.version);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible())
                .await
                .is_err()
        );
        let frame = inbox.poll().unwrap();
        assert!(frame.status_text().contains("內容辨識：明確測試內容服務"));
        assert!(frame.status_text().contains("歌詞查詢：明確測試歌詞服務"));
        assert!(frame.status_text().contains("字幕查詢：明確測試字幕服務"));
        assert!(frame.status_text().contains("僅讀取已設定的字幕位置"));
        assert_eq!(frame.subtitles.state, UiSubtitleState::Disabled);
        inbox.acknowledge_visible(frame.version);
        publisher.wait_until_visible().await.unwrap();
        publisher.begin(identity(1), source()).unwrap();
        assert_eq!(
            inbox.poll().unwrap().subtitles.state,
            UiSubtitleState::Waiting
        );
        assert_eq!(
            publisher.prepare_subtitles(None),
            Err(UiBridgeError::InvalidState)
        );
    }
    #[test]
    fn subtitle_status_updates_preserve_the_shared_live_snapshot() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher
            .prepare_subtitles(Some(subtitle_disclosure()))
            .unwrap();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        let initial = inbox.poll().unwrap();
        let snapshot = initial.snapshot.as_ref().unwrap().clone();
        for state in [
            UiSubtitleState::LookingUp,
            UiSubtitleState::Available,
            UiSubtitleState::Verifying,
            UiSubtitleState::Localizing,
            UiSubtitleState::Preparing,
            UiSubtitleState::UsingExisting,
            UiSubtitleState::Unavailable,
            UiSubtitleState::Failed(crate::resolver::contracts::ResolverError::Timeout),
        ] {
            let status = UiSubtitleStatus {
                disclosure: Some(subtitle_disclosure()),
                state,
            };
            assert!(publisher.publish_subtitles(identity(1), status).unwrap());
            let frame = inbox.poll().unwrap();
            assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &snapshot));
            assert!(frame.status == UiStatus::Translating);
            assert!(frame.status_text().contains("即時翻譯提供"));
            assert!(!publisher.publish_subtitles(identity(1), status).unwrap());
            assert!(inbox.poll().is_none());
        }
    }
    #[test]
    fn subtitle_updates_reject_old_identity_undisclosed_service_and_disabled_lookup() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher.prepare_subtitles(None).unwrap();
        publisher.begin(identity(1), source()).unwrap();
        assert!(!inbox.poll().unwrap().status_text().contains("字幕查詢"));
        assert_eq!(
            publisher.publish_subtitles(
                identity(1),
                UiSubtitleStatus {
                    disclosure: None,
                    state: UiSubtitleState::LookingUp,
                }
            ),
            Err(UiBridgeError::InvalidState)
        );
        assert!(
            !publisher
                .publish_subtitles(identity(1), UiSubtitleStatus::default())
                .unwrap()
        );
        assert!(inbox.poll().is_none());
        publisher.prepare_source(source()).unwrap();
        publisher
            .prepare_subtitles(Some(subtitle_disclosure()))
            .unwrap();
        publisher.begin(identity(2), source()).unwrap();
        inbox.poll();
        let status = UiSubtitleStatus {
            disclosure: Some(subtitle_disclosure()),
            state: UiSubtitleState::Available,
        };
        for disclosure in [
            None,
            Some(ContentDisclosure {
                service: "未公開字幕服務",
                ..subtitle_disclosure()
            }),
            Some(ContentDisclosure {
                transport: crate::ui::content::ContentTransport::Local,
                ..subtitle_disclosure()
            }),
        ] {
            assert_eq!(
                publisher.publish_subtitles(
                    identity(2),
                    UiSubtitleStatus {
                        disclosure,
                        ..status
                    }
                ),
                Err(UiBridgeError::InvalidState)
            );
            assert!(inbox.poll().is_none());
        }
        assert!(!publisher.publish_subtitles(identity(1), status).unwrap());
        assert!(
            !publisher
                .publish_subtitles(
                    StreamIdentity {
                        generation: 2,
                        ..identity(2)
                    },
                    status
                )
                .unwrap()
        );
        assert!(inbox.poll().is_none());
    }
    #[test]
    fn subtitle_terminal_frames_and_source_reset_clear_lifecycle_without_payload_retention() {
        for terminal in [UiStatus::Reconnecting, UiStatus::Stopped, UiStatus::Error] {
            let (mut publisher, mut inbox) = channel();
            publisher.prepare_source(source()).unwrap();
            publisher
                .prepare_subtitles(Some(subtitle_disclosure()))
                .unwrap();
            publisher.begin(identity(1), source()).unwrap();
            publisher
                .publish(&TranscriptTimeline::new(identity(1)), true)
                .unwrap();
            publisher
                .publish_subtitles(
                    identity(1),
                    UiSubtitleStatus {
                        disclosure: Some(subtitle_disclosure()),
                        state: UiSubtitleState::Available,
                    },
                )
                .unwrap();
            publisher.clear(terminal).unwrap();
            let frame = inbox.poll().unwrap();
            assert!(frame.identity.is_none());
            assert!(frame.snapshot.is_none());
            assert_eq!(frame.subtitles.state, UiSubtitleState::Stopped);
            assert!(frame.status_text().contains("已停止讀取"));
            if terminal == UiStatus::Reconnecting {
                publisher.begin(identity(2), source()).unwrap();
                assert_eq!(
                    inbox.poll().unwrap().subtitles.state,
                    UiSubtitleState::Waiting
                );
            }
            publisher.prepare_source(source()).unwrap();
            let frame = inbox.poll().unwrap();
            assert!(frame.subtitles.disclosure.is_none());
            assert_eq!(frame.subtitles.state, UiSubtitleState::Disabled);
            assert!(!frame.status_text().contains("字幕查詢"));
        }
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher
            .prepare_subtitles(Some(subtitle_disclosure()))
            .unwrap();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        inbox.cancellation().cancel();
        publisher.finish(UiStatus::Error).unwrap();
        assert_eq!(
            inbox.poll().unwrap().subtitles.state,
            UiSubtitleState::Stopped
        );
        drop(publisher);
        let frame = inbox.poll().unwrap();
        assert!(frame.identity.is_none());
        assert!(frame.snapshot.is_none());
        assert!(frame.status == UiStatus::Error);
        assert_eq!(frame.subtitles.state, UiSubtitleState::Stopped);
    }
    #[tokio::test]
    async fn lyrics_disclosure_requires_matching_visible_acknowledgement() {
        let (mut publisher, mut inbox) = channel();
        assert_eq!(
            publisher.prepare_lyrics(Some(lyrics_disclosure())),
            Err(UiBridgeError::InvalidState)
        );
        publisher.prepare_source(source()).unwrap();
        let source_frame = inbox.poll().unwrap();
        assert!(!source_frame.status_text().contains("歌詞查詢"));
        inbox.acknowledge_visible(source_frame.version);
        for service in ["", "\n", "unknown\tservice"] {
            assert_eq!(
                publisher.prepare_lyrics(Some(ContentDisclosure {
                    service,
                    ..lyrics_disclosure()
                })),
                Err(UiBridgeError::InvalidState)
            );
            assert!(inbox.poll().is_none());
        }
        publisher
            .prepare_content(Some(content_disclosure()))
            .unwrap();
        let content_frame = inbox.poll().unwrap();
        inbox.acknowledge_visible(content_frame.version);
        publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible())
                .await
                .is_err()
        );
        let frame = inbox.poll().unwrap();
        assert!(frame.status_text().contains("內容辨識：明確測試內容服務"));
        assert!(frame.status_text().contains("歌詞查詢：明確測試歌詞服務"));
        assert!(frame.status_text().contains("歌曲資料將送至此服務"));
        assert_eq!(frame.lyrics.state, UiLyricsState::Disabled);
        inbox.acknowledge_visible(frame.version);
        publisher.wait_until_visible().await.unwrap();
        publisher.begin(identity(1), source()).unwrap();
        assert_eq!(inbox.poll().unwrap().lyrics.state, UiLyricsState::Waiting);
        assert_eq!(
            publisher.prepare_lyrics(None),
            Err(UiBridgeError::InvalidState)
        );
    }
    #[test]
    fn lyrics_status_changes_never_replace_the_shared_live_snapshot() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        let initial = inbox.poll().unwrap();
        let snapshot = initial.snapshot.as_ref().unwrap().clone();
        for state in [
            UiLyricsState::LookingUp,
            UiLyricsState::Available,
            UiLyricsState::Unavailable,
            UiLyricsState::Failed(crate::resolver::contracts::ResolverError::Timeout),
            UiLyricsState::Waiting,
        ] {
            let status = UiLyricsStatus {
                disclosure: Some(lyrics_disclosure()),
                state,
            };
            assert!(publisher.publish_lyrics(identity(1), status).unwrap());
            let frame = inbox.poll().unwrap();
            assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &snapshot));
            assert!(frame.status == UiStatus::Translating);
            assert!(frame.status_text().contains("字幕仍由即時翻譯提供"));
            if state == UiLyricsState::Available {
                assert!(frame.status_text().contains("尚未定位或採用"));
            }
            assert!(!publisher.publish_lyrics(identity(1), status).unwrap());
            assert!(inbox.poll().is_none());
        }
    }
    #[test]
    fn lyrics_updates_reject_old_sessions_and_undisclosed_services() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
        publisher.begin(identity(1), source()).unwrap();
        let status = UiLyricsStatus {
            disclosure: Some(lyrics_disclosure()),
            state: UiLyricsState::Available,
        };
        publisher.publish_lyrics(identity(1), status).unwrap();
        inbox.poll();
        for disclosure in [
            None,
            Some(ContentDisclosure {
                service: "未公開歌詞服務",
                ..lyrics_disclosure()
            }),
            Some(ContentDisclosure {
                transport: crate::ui::content::ContentTransport::Local,
                ..lyrics_disclosure()
            }),
        ] {
            assert_eq!(
                publisher.publish_lyrics(
                    identity(1),
                    UiLyricsStatus {
                        disclosure,
                        ..status
                    }
                ),
                Err(UiBridgeError::InvalidState)
            );
            assert!(inbox.poll().is_none());
        }
        publisher.clear(UiStatus::Reconnecting).unwrap();
        publisher.begin(identity(2), source()).unwrap();
        assert_eq!(inbox.poll().unwrap().lyrics.state, UiLyricsState::Waiting);
        assert!(!publisher.publish_lyrics(identity(1), status).unwrap());
        assert!(
            !publisher
                .publish_lyrics(
                    StreamIdentity {
                        generation: 2,
                        ..identity(2)
                    },
                    status
                )
                .unwrap()
        );
        assert!(inbox.poll().is_none());
    }
    #[test]
    fn disabled_lyrics_cannot_claim_lookup_without_a_configured_service() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher.prepare_lyrics(None).unwrap();
        assert!(!inbox.poll().unwrap().status_text().contains("歌詞查詢"));
        publisher.begin(identity(1), source()).unwrap();
        assert_eq!(inbox.poll().unwrap().lyrics.state, UiLyricsState::Disabled);
        assert_eq!(
            publisher.publish_lyrics(
                identity(1),
                UiLyricsStatus {
                    disclosure: None,
                    state: UiLyricsState::LookingUp
                }
            ),
            Err(UiBridgeError::InvalidState)
        );
        assert!(
            !publisher
                .publish_lyrics(identity(1), UiLyricsStatus::default())
                .unwrap()
        );
        assert!(inbox.poll().is_none());
    }
    #[test]
    fn lyrics_terminal_and_new_source_frames_clear_lookup_state() {
        for terminal in [UiStatus::Reconnecting, UiStatus::Stopped, UiStatus::Error] {
            let (mut publisher, mut inbox) = channel();
            publisher.prepare_source(source()).unwrap();
            publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
            publisher.begin(identity(1), source()).unwrap();
            publisher
                .publish(&TranscriptTimeline::new(identity(1)), true)
                .unwrap();
            publisher
                .publish_lyrics(
                    identity(1),
                    UiLyricsStatus {
                        disclosure: Some(lyrics_disclosure()),
                        state: UiLyricsState::Available,
                    },
                )
                .unwrap();
            publisher.clear(terminal).unwrap();
            let frame = inbox.poll().unwrap();
            assert!(frame.identity.is_none());
            assert!(frame.snapshot.is_none());
            assert_eq!(frame.lyrics.state, UiLyricsState::Stopped);
            assert!(frame.status_text().contains("已停止傳送"));
            publisher.prepare_source(source()).unwrap();
            let frame = inbox.poll().unwrap();
            assert!(frame.lyrics.disclosure.is_none());
            assert_eq!(frame.lyrics.state, UiLyricsState::Disabled);
            assert!(!frame.status_text().contains("歌詞查詢"));
        }
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        publisher
            .publish_lyrics(
                identity(1),
                UiLyricsStatus {
                    disclosure: Some(lyrics_disclosure()),
                    state: UiLyricsState::Available,
                },
            )
            .unwrap();
        drop(publisher);
        let frame = inbox.poll().unwrap();
        assert!(frame.identity.is_none());
        assert!(frame.snapshot.is_none());
        assert_eq!(frame.lyrics.state, UiLyricsState::Stopped);
        assert!(!frame.status_text().contains("尚未定位或採用"));
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher.prepare_lyrics(Some(lyrics_disclosure())).unwrap();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish_lyrics(
                identity(1),
                UiLyricsStatus {
                    disclosure: Some(lyrics_disclosure()),
                    state: UiLyricsState::LookingUp,
                },
            )
            .unwrap();
        inbox.cancellation().cancel();
        publisher.finish(UiStatus::Error).unwrap();
        let frame = inbox.poll().unwrap();
        assert_eq!(frame.lyrics.state, UiLyricsState::Stopped);
        drop(publisher);
        let frame = inbox.poll().unwrap();
        assert!(frame.identity.is_none());
        assert!(frame.snapshot.is_none());
        assert!(frame.status == UiStatus::Error);
        assert_eq!(frame.lyrics.state, UiLyricsState::Stopped);
    }
    #[tokio::test]
    async fn content_disclosure_requires_a_new_visible_acknowledgement() {
        let (mut publisher, mut inbox) = channel();
        assert_eq!(
            publisher.prepare_content(Some(content_disclosure())),
            Err(UiBridgeError::InvalidState)
        );
        publisher.prepare_source(source()).unwrap();
        let version = inbox.poll().unwrap().version;
        inbox.acknowledge_visible(version);
        publisher
            .prepare_content(Some(content_disclosure()))
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible())
                .await
                .is_err()
        );
        let frame = inbox.poll().unwrap();
        assert!(frame.status_text().contains("內容資料將送至此服務"));
        assert_eq!(frame.content.state, UiContentState::Disabled);
        inbox.acknowledge_visible(frame.version);
        publisher.wait_until_visible().await.unwrap();
        publisher.begin(identity(1), source()).unwrap();
        assert_eq!(
            publisher.prepare_content(None),
            Err(UiBridgeError::InvalidState)
        );
    }
    #[tokio::test]
    async fn movie_hint_disclosure_requires_the_latest_visible_acknowledgement() {
        let (mut publisher, mut inbox) = channel();
        assert_eq!(
            publisher.prepare_content_with_movie_hints(Some(content_disclosure()), true),
            Err(UiBridgeError::InvalidState)
        );
        publisher.prepare_source(source()).unwrap();
        let source_version = inbox.poll().unwrap().version;
        inbox.acknowledge_visible(source_version);
        assert_eq!(
            publisher.prepare_content_with_movie_hints(None, true),
            Err(UiBridgeError::InvalidState)
        );
        assert_eq!(
            publisher.prepare_content_with_movie_hints(
                Some(ContentDisclosure {
                    service: "\n",
                    ..content_disclosure()
                }),
                true,
            ),
            Err(UiBridgeError::InvalidState)
        );
        assert!(inbox.poll().is_none());
        publisher
            .prepare_content_with_movie_hints(Some(content_disclosure()), true)
            .unwrap();
        inbox.acknowledge_visible(source_version);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible())
                .await
                .is_err()
        );
        let disclosure_frame = inbox.poll().unwrap();
        assert!(disclosure_frame.identity.is_none());
        assert!(disclosure_frame.content.movie_dialogue);
        assert_eq!(disclosure_frame.content.state, UiContentState::Disabled);
        assert!(
            disclosure_frame
                .status_text()
                .contains("目前已定稿語音辨識文字與已設定媒體線索將送至此服務")
        );
        inbox.acknowledge_visible(disclosure_frame.version);
        publisher.wait_until_visible().await.unwrap();
        publisher.begin(identity(1), source()).unwrap();
        let running = inbox.poll().unwrap();
        assert!(running.content.movie_dialogue);
        assert_eq!(running.content.state, UiContentState::Collecting);
        assert_eq!(
            publisher.prepare_content_with_movie_hints(Some(content_disclosure()), false),
            Err(UiBridgeError::InvalidState)
        );
        assert!(inbox.poll().is_none());
    }
    #[test]
    fn movie_hint_publication_cannot_toggle_the_acknowledged_route() {
        for enabled in [false, true] {
            let (mut publisher, mut inbox) = channel();
            publisher.prepare_source(source()).unwrap();
            publisher
                .prepare_content_with_movie_hints(Some(content_disclosure()), enabled)
                .unwrap();
            publisher.begin(identity(1), source()).unwrap();
            publisher
                .publish(&TranscriptTimeline::new(identity(1)), true)
                .unwrap();
            let frame = inbox.poll().unwrap();
            let snapshot = frame.snapshot.as_ref().unwrap().clone();
            let actual = UiContentStatus {
                disclosure: Some(content_disclosure()),
                movie_dialogue: enabled,
                state: UiContentState::Identifying,
            };
            assert_eq!(
                publisher.publish_content(
                    identity(1),
                    UiContentStatus {
                        movie_dialogue: !enabled,
                        ..actual
                    },
                ),
                Err(UiBridgeError::InvalidState)
            );
            assert!(inbox.poll().is_none());
            assert!(publisher.publish_content(identity(1), actual).unwrap());
            let frame = inbox.poll().unwrap();
            assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &snapshot));
            assert!(frame.status == UiStatus::Translating);
            assert_eq!(frame.content.movie_dialogue, enabled);
            publisher.clear(UiStatus::Stopped).unwrap();
            let stopped = inbox.poll().unwrap();
            assert!(stopped.snapshot.is_none());
            assert_eq!(stopped.content.movie_dialogue, enabled);
            assert_eq!(stopped.content.state, UiContentState::Stopped);
            assert!(!stopped.status_text().contains("將送至"));
            publisher.prepare_source(source()).unwrap();
            assert!(!inbox.poll().unwrap().content.movie_dialogue);
        }
    }
    #[test]
    fn foreign_movie_hint_status_never_changes_the_current_summary_or_snapshot() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher
            .prepare_content_with_movie_hints(Some(content_disclosure()), true)
            .unwrap();
        publisher.begin(identity(2), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(2)), true)
            .unwrap();
        let frame = inbox.poll().unwrap();
        let snapshot = frame.snapshot.as_ref().unwrap().clone();
        let foreign_status = UiContentStatus {
            disclosure: Some(ContentDisclosure {
                service: "其他 Session 服務",
                ..content_disclosure()
            }),
            movie_dialogue: false,
            state: UiContentState::Identifying,
        };
        for old in [
            identity(1),
            StreamIdentity {
                generation: 2,
                ..identity(2)
            },
        ] {
            assert!(!publisher.publish_content(old, foreign_status).unwrap());
        }
        assert!(inbox.poll().is_none());
        let current_status = UiContentStatus {
            disclosure: Some(content_disclosure()),
            movie_dialogue: true,
            state: UiContentState::Identifying,
        };
        publisher
            .publish_content(identity(2), current_status)
            .unwrap();
        let current = inbox.poll().unwrap();
        assert!(current.content == current_status);
        assert!(Arc::ptr_eq(current.snapshot.as_ref().unwrap(), &snapshot));
    }
    #[test]
    fn content_status_preserves_shared_subtitles_and_rejects_undisclosed_destinations() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        publisher
            .prepare_content(Some(content_disclosure()))
            .unwrap();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        let snapshot = inbox.poll().unwrap().snapshot.as_ref().unwrap().clone();
        let status = UiContentStatus {
            disclosure: Some(content_disclosure()),
            movie_dialogue: false,
            state: UiContentState::Ready(
                crate::resolver::contracts::ContentType::Music,
                crate::resolver::candidates::SongState::Confirmed,
            ),
        };
        assert!(publisher.publish_content(identity(1), status).unwrap());
        let frame = inbox.poll().unwrap();
        assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &snapshot));
        assert!(frame.status == UiStatus::Translating);
        assert!(frame.status_text().contains("字幕仍使用即時翻譯"));
        assert!(!publisher.publish_content(identity(1), status).unwrap());
        assert!(inbox.poll().is_none());
        let replacement = UiContentStatus {
            disclosure: Some(ContentDisclosure {
                service: "未公開的測試服務",
                ..content_disclosure()
            }),
            ..status
        };
        assert_eq!(
            publisher.publish_content(identity(1), replacement),
            Err(UiBridgeError::InvalidState)
        );
        assert!(inbox.poll().is_none());
        publisher.clear(UiStatus::Reconnecting).unwrap();
        let frame = inbox.poll().unwrap();
        assert_eq!(frame.content.state, UiContentState::Stopped);
        assert!(frame.status_text().contains("已停止傳送"));
        assert!(frame.snapshot.is_none());
        publisher.begin(identity(2), source()).unwrap();
        assert_eq!(
            inbox.poll().unwrap().content.state,
            UiContentState::Collecting
        );
        assert!(!publisher.publish_content(identity(1), status).unwrap());
        assert!(
            !publisher
                .publish_content(
                    StreamIdentity {
                        generation: 2,
                        ..identity(2)
                    },
                    status
                )
                .unwrap()
        );
        assert!(inbox.poll().is_none());
        publisher.prepare_source(source()).unwrap();
        assert!(inbox.poll().unwrap().content.disclosure.is_none());
    }
    #[test]
    fn disabled_content_cannot_be_published_as_recognition_and_drop_clears_summary() {
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        let status = UiContentStatus {
            disclosure: None,
            movie_dialogue: false,
            state: UiContentState::Identifying,
        };
        assert_eq!(
            publisher.publish_content(identity(1), status),
            Err(UiBridgeError::InvalidState)
        );
        publisher.prepare_source(source()).unwrap();
        publisher
            .prepare_content(Some(content_disclosure()))
            .unwrap();
        publisher.begin(identity(2), source()).unwrap();
        publisher
            .publish_content(
                identity(2),
                UiContentStatus {
                    disclosure: Some(content_disclosure()),
                    movie_dialogue: false,
                    state: UiContentState::Identifying,
                },
            )
            .unwrap();
        drop(publisher);
        let frame = inbox.poll().unwrap();
        assert!(frame.identity.is_none());
        assert!(frame.snapshot.is_none());
        assert_eq!(frame.content.state, UiContentState::Stopped);
    }
    #[test]
    fn process_birth_metadata_survives_reconnect_but_never_moves_to_a_different_pid() {
        let (mut publisher, mut inbox) = channel();
        assert!(
            publisher
                .prepare_source_with_creation(source(), Some(0))
                .is_err()
        );
        publisher
            .prepare_source_with_creation(source(), Some(123))
            .unwrap();
        assert_eq!(inbox.poll().unwrap().process_created, Some(123));
        publisher.begin(identity(1), source()).unwrap();
        assert_eq!(inbox.poll().unwrap().process_created, Some(123));
        publisher.clear(UiStatus::Reconnecting).unwrap();
        publisher.begin(identity(2), source()).unwrap();
        assert_eq!(inbox.poll().unwrap().process_created, Some(123));
        publisher
            .begin(
                identity(3),
                UiSource {
                    pid: source().pid + 1,
                    provider: source().provider,
                },
            )
            .unwrap();
        assert_eq!(inbox.poll().unwrap().process_created, None);
    }
    #[test]
    fn saturated_notifications_retain_only_latest_session_and_frame() {
        let (mut publisher, mut inbox) = channel();
        let mut old = std::sync::Weak::new();
        for id in 1..=1000 {
            publisher.begin(identity(id), source()).unwrap();
            publisher
                .publish(&TranscriptTimeline::new(identity(id)), true)
                .unwrap();
            if id == 1 {
                old = Arc::downgrade(&inbox.poll().unwrap());
            }
        }
        assert!(old.upgrade().is_none());
        let frame = inbox.poll().unwrap();
        assert_eq!(frame.identity, Some(identity(1000)));
        assert!(inbox.poll().is_none());
        assert!(
            !publisher
                .publish(&TranscriptTimeline::new(identity(1)), true)
                .unwrap()
        );
        assert!(inbox.poll().is_none());
    }
    #[test]
    fn final_frame_bypasses_throttle_and_close_cancels_producer() {
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        let timeline = TranscriptTimeline::new(identity(1));
        assert!(publisher.publish(&timeline, false).unwrap());
        assert!(!publisher.publish(&timeline, false).unwrap());
        assert!(publisher.publish(&timeline, true).unwrap());
        assert!(inbox.poll().unwrap().snapshot.is_some());
        let cancel = publisher.cancellation();
        drop(inbox);
        assert!(cancel.is_cancelled());
        assert_eq!(
            publisher.publish(&timeline, true),
            Err(UiBridgeError::Closed)
        );
    }
    #[test]
    fn terminal_drop_clears_content_and_preserves_visible_error() {
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        publisher.clear(UiStatus::Error).unwrap();
        drop(publisher);
        let frame = inbox.poll().unwrap();
        assert!(frame.snapshot.is_none());
        assert!(frame.identity.is_none());
        assert!(frame.status == UiStatus::Error);
        assert!(inbox.poll().is_none());
    }
    #[test]
    fn shared_snapshot_survives_mode_switch_without_backend_restarts() {
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        let frame = inbox.poll().unwrap();
        let snapshot = frame.snapshot.as_ref().unwrap();
        let mut view = Presentation::new(DisplayMode::SideChat);
        view.select_session(identity(1));
        assert!(view.apply_shared(snapshot.clone()));
        for mode in [
            DisplayMode::SideChat,
            DisplayMode::BottomSubtitle,
            DisplayMode::Dual,
        ] {
            view.mode = mode;
            assert!(!publisher.cancellation().is_cancelled());
            assert!(inbox.poll().is_none());
        }
        assert_eq!(view.identity(), Some(identity(1)));
    }
    #[tokio::test]
    async fn cancellation_wakes_pending_owner_on_inbox_drop() {
        let (publisher, inbox) = channel();
        let cancel = publisher.cancellation();
        let wait = cancel.cancelled();
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), &mut wait)
                .await
                .is_err()
        );
        drop(inbox);
        tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn source_disclosure_gate_waits_for_matching_presentation() {
        let (mut publisher, mut inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        let first = inbox.poll().unwrap();
        assert!(first.status == UiStatus::Starting);
        assert!(first.status_text().contains("音訊與文字"));
        assert!(
            tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible())
                .await
                .is_err()
        );
        inbox.acknowledge_visible(first.version);
        publisher.wait_until_visible().await.unwrap();
        publisher.prepare_source(source()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(5), publisher.wait_until_visible())
                .await
                .is_err()
        );
        let second = inbox.poll().unwrap();
        inbox.acknowledge_visible(second.version);
        publisher.wait_until_visible().await.unwrap();
    }
    #[tokio::test]
    async fn closing_before_disclosure_never_opens_the_startup_gate() {
        let (mut publisher, inbox) = channel();
        publisher.prepare_source(source()).unwrap();
        drop(inbox);
        assert_eq!(
            publisher.wait_until_visible().await,
            Err(UiBridgeError::Closed)
        );
    }
    #[test]
    fn terminal_completion_after_stop_clears_content_and_retains_failure() {
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        publisher
            .publish(&TranscriptTimeline::new(identity(1)), true)
            .unwrap();
        inbox.cancellation().cancel();
        assert_eq!(publisher.clear(UiStatus::Error), Err(UiBridgeError::Closed));
        publisher.finish(UiStatus::Error).unwrap();
        drop(publisher);
        let frame = inbox.poll().unwrap();
        assert!(frame.status == UiStatus::Error);
        assert!(frame.snapshot.is_none());
        assert!(frame.identity.is_none());
    }
    struct MockTranslator(std::sync::atomic::AtomicUsize);
    impl crate::ai::translation::Translator for MockTranslator {
        fn capabilities(&self) -> crate::ai::translation::TranslationCapabilities {
            crate::ai::translation::TranslationCapabilities {
                streaming: true,
                cancellation: true,
                supported_languages: [true; 5],
                max_concurrent: 1,
            }
        }
        fn translate<'a>(
            &'a self,
            _: &'a crate::ai::translation::TranslationRequest,
            output: &'a mut crate::ai::translation::TranslationEmitter,
            _: crate::ai::translation::TranslationCancellation,
        ) -> crate::ai::translation::TranslationFuture<'a> {
            Box::pin(async move {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                output.emit("測試暫定譯文")?;
                Ok("測試完成譯文".into())
            })
        }
    }
    #[tokio::test]
    async fn one_translation_flow_feeds_both_views_and_final_is_not_lost() {
        use crate::{
            ai::speech::{TranscriptEvent, TranscriptKind},
            domain::TranslationQuality,
            session::live_translation::LiveTranslation,
            subtitle::timeline::Stability,
        };
        let provider = MockTranslator(std::sync::atomic::AtomicUsize::new(0));
        let mut flow =
            LiveTranslation::new(identity(1), TranslationQuality::Standard, &provider).unwrap();
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        flow.ingest(TranscriptEvent {
            identity: identity(1),
            sequence: 1,
            utterance: 1,
            revision: 1,
            start_us: Some(0),
            duration_us: Some(100000),
            language: None,
            kind: TranscriptKind::Final,
            text: "test source".into(),
        })
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                flow.changed().await.unwrap();
                let final_ready = flow.timeline().items()[0].stability == Stability::Final;
                publisher.publish(flow.timeline(), final_ready).unwrap();
                if final_ready {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let frame = inbox.poll().unwrap();
        let mut view = Presentation::new(DisplayMode::Dual);
        view.select_session(frame.identity.unwrap());
        view.apply_shared(frame.snapshot.as_ref().unwrap().clone());
        assert_eq!(view.rows().count(), 1);
        assert!(view.side_text().contains("test source"));
        assert_eq!(view.overlay_text(), "測試完成譯文");
        assert!(view.set_search("missing"));
        assert_eq!(view.rows().count(), 0);
        assert_eq!(view.overlay_text(), "測試完成譯文");
        for mode in [
            DisplayMode::BottomSubtitle,
            DisplayMode::SideChat,
            DisplayMode::Dual,
        ] {
            view.mode = mode;
        }
        assert_eq!(provider.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        publisher.clear(UiStatus::Stopped).unwrap();
        let stopped = inbox.poll().unwrap();
        assert!(stopped.snapshot.is_none());
        assert!(stopped.status_text().contains("已停止傳送內容"));
        view.stop();
        assert!(view.overlay_text().contains("測試完成譯文"));
        view.clear_subtitles();
        assert!(view.overlay_text().is_empty());
    }

    #[tokio::test]
    async fn phase11_paused_source_finishes_pending_translation_and_retains_captions_until_manual_clear()
     {
        use crate::{
            ai::{
                speech::{TranscriptEvent, TranscriptKind},
                translation::*,
            },
            domain::TranslationQuality,
            session::live_translation::LiveTranslation,
            subtitle::timeline::Stability,
        };
        struct PausedTranslator(tokio::sync::Notify);
        impl Translator for PausedTranslator {
            fn capabilities(&self) -> TranslationCapabilities {
                MockTranslator(std::sync::atomic::AtomicUsize::new(0)).capabilities()
            }
            fn translate<'a>(
                &'a self,
                request: &'a TranslationRequest,
                output: &'a mut TranslationEmitter,
                cancel: TranslationCancellation,
            ) -> TranslationFuture<'a> {
                Box::pin(async move {
                    if request.source.kind == TranscriptKind::Final {
                        output.emit("新")?;
                        tokio::select! { biased; reason=cancel.cancelled()=>return Err(reason),_=self.0.notified()=>{} }
                        Ok("暫停後完成的合成譯文".into())
                    } else {
                        Ok("暫停前可讀的合成譯文".into())
                    }
                })
            }
        }
        let provider = PausedTranslator(tokio::sync::Notify::new());
        let mut flow =
            LiveTranslation::new(identity(1), TranslationQuality::Standard, &provider).unwrap();
        let mut event = TranscriptEvent {
            identity: identity(1),
            utterance: 1,
            sequence: 1,
            revision: 1,
            start_us: None,
            duration_us: None,
            language: None,
            kind: TranscriptKind::Partial,
            text: "synthetic pause source".into(),
        };
        flow.ingest(event.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while flow.timeline().items()[0].translated.is_empty() {
                flow.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        event.revision = 2;
        event.kind = TranscriptKind::Final;
        flow.ingest(event).unwrap();
        assert_eq!(flow.changed().await.unwrap(), None); // First replacement chunk stays hidden.
        let (mut publisher, mut inbox) = channel();
        publisher.begin(identity(1), source()).unwrap();
        publisher.publish(flow.timeline(), true).unwrap();
        let frame = inbox.poll().unwrap();
        let mut view = Presentation::new(DisplayMode::Dual);
        view.select_session(identity(1));
        view.apply_shared(frame.snapshot.as_ref().unwrap().clone());
        assert_eq!(view.overlay_text(), "暫停前可讀的合成譯文");
        // No further audio or transcript is required to finish the owned request.
        provider.0.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while flow.timeline().items()[0].stability != Stability::Final {
                flow.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        publisher.publish(flow.timeline(), true).unwrap();
        let completed = inbox.poll().unwrap();
        view.apply_shared(completed.snapshot.as_ref().unwrap().clone());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), flow.changed())
                .await
                .is_err()
        );
        assert_eq!(flow.cancellation().reason(), None);
        assert_eq!(view.overlay_text(), "暫停後完成的合成譯文");
        assert!(view.side_text().contains("暫停後完成的合成譯文"));
        assert_eq!(completed.identity, Some(identity(1)));
        publisher.clear(UiStatus::Stopped).unwrap();
        assert!(inbox.poll().unwrap().snapshot.is_none());
        view.stop();
        assert!(view.overlay_text().contains("暫停後完成的合成譯文"));
        view.clear_subtitles();
        assert!(view.overlay_text().is_empty());
    }
}
