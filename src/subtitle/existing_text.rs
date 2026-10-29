//! Phase 7 Chinese text normalization. This produces a candidate, not alignment
//! evidence or permission to replace Live AI. No renderer, cloud or disk access.
use super::localization::taiwan_phrase_conversion;
use crate::{
    ai::speech::{SpeechLanguage, StreamIdentity},
    domain::TARGET_LANGUAGE,
    resolver::{
        contracts::{
            ExistingText, ResolverCancellation, ResolverError, TextLanguage, TextQuery, WindowStamp,
        },
        text_session::MAX_SESSION_TEXT_BYTES,
    },
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc::{self, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

/// One active conversion and at most one cancelled/replacement input waiting.
const PENDING_CONVERSIONS: usize = 1;
const CONVERSION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextLocalizationError {
    #[error("{0}")]
    Invalid(#[from] ResolverError),
    #[error("此來源字幕需要翻譯；尚未送至翻譯服務。")]
    TranslationRequired(SpeechLanguage),
    #[error("字幕本地化工作已取消。")]
    Cancelled,
    #[error("字幕本地化佇列已滿，已捨棄本次工作。")]
    Overloaded,
    #[error("字幕本地化逾時，未採用本次結果。")]
    Timeout,
    #[error("字幕本地化工作異常結束。")]
    WorkerPanicked,
    #[error("無法建立字幕本地化工作。")]
    WorkerStart,
}
/// Original text/timing and target strings share one cue index. No Debug,
/// serialization, export or persistent history. A later arbiter must verify
/// freshness, identity and actual playback alignment before any adoption.
pub struct LocalizedText {
    stamp: WindowStamp,
    original: ExistingText,
    translated: Vec<Box<str>>,
}
impl LocalizedText {
    pub fn stamp(&self) -> WindowStamp {
        self.stamp
    }
    pub fn original(&self) -> &ExistingText {
        &self.original
    }
    pub fn translated(&self) -> &[Box<str>] {
        &self.translated
    }
    pub const fn target_language(&self) -> &'static str {
        TARGET_LANGUAGE
    }
}
type ConversionResult = Result<LocalizedText, TextLocalizationError>;
struct Work {
    query: TextQuery,
    text: ExistingText,
    translated: Option<Vec<Box<str>>>,
    cancel: ResolverCancellation,
    reply: oneshot::Sender<ConversionResult>,
}
struct CancelOnDrop(ResolverCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// One Session-owned CPU lane, matching the single selected subtitle candidate.
/// Borrowing mutably prevents concurrent requests from this owner. Input/result
/// queues are bounded, dropping a request cancels it, and Drop joins the worker.
pub struct ChineseTextLocalizer {
    identity: StreamIdentity,
    sender: Option<SyncSender<Work>>,
    worker: Option<JoinHandle<()>>,
    shutdown: ResolverCancellation,
    previous: Option<WindowStamp>,
}
impl ChineseTextLocalizer {
    pub fn new(identity: StreamIdentity) -> Result<Self, TextLocalizationError> {
        Self::with_converter(identity, normalize)
    }
    fn with_converter(
        identity: StreamIdentity,
        mut convert: impl FnMut(
            StreamIdentity,
            &TextQuery,
            ExistingText,
            &ResolverCancellation,
        ) -> ConversionResult
        + Send
        + 'static,
    ) -> Result<Self, TextLocalizationError> {
        let (sender, receiver) = mpsc::sync_channel::<Work>(PENDING_CONVERSIONS);
        let shutdown = ResolverCancellation::default();
        let stopped = shutdown.clone();
        // A dedicated single lane for one active candidate; no pool or per-cue thread.
        let worker = thread::Builder::new()
            .name("subtitle-localization".into())
            .spawn(move || {
                while let Ok(work) = receiver.recv() {
                    if stopped.is_cancelled() {
                        break;
                    }
                    let result = catch_unwind(AssertUnwindSafe(|| match work.translated {
                        Some(translated) => normalize_translated(
                            identity,
                            &work.query,
                            work.text,
                            translated,
                            &work.cancel,
                        ),
                        None => convert(identity, &work.query, work.text, &work.cancel),
                    }))
                    .unwrap_or(Err(TextLocalizationError::WorkerPanicked));
                    // Failed send drops the entire candidate if its caller is gone.
                    let _ = work.reply.send(result);
                }
            })
            .map_err(|_| TextLocalizationError::WorkerStart)?;
        Ok(Self {
            identity,
            sender: Some(sender),
            worker: Some(worker),
            shutdown,
            previous: None,
        })
    }
    /// Cancelling/dropping this future stops work between cues (each at most 8 KiB).
    /// Results are rechecked against original freshness before leaving this owner.
    pub async fn normalize(
        &mut self,
        query: TextQuery,
        text: ExistingText,
        cancel: ResolverCancellation,
    ) -> ConversionResult {
        self.submit(query, text, None, cancel).await
    }
    pub(crate) async fn normalize_translations(
        &mut self,
        query: TextQuery,
        text: ExistingText,
        translated: Vec<Box<str>>,
        cancel: ResolverCancellation,
    ) -> ConversionResult {
        self.submit(query, text, Some(translated), cancel).await
    }
    async fn submit(
        &mut self,
        query: TextQuery,
        text: ExistingText,
        translated: Option<Vec<Box<str>>>,
        cancel: ResolverCancellation,
    ) -> ConversionResult {
        if cancel.is_cancelled() || self.shutdown.is_cancelled() {
            return Err(TextLocalizationError::Cancelled);
        }
        query.stamp.validate(self.identity, Instant::now())?;
        query.validate(Instant::now())?;
        if self.previous.is_some_and(|old| {
            query.stamp.sequence <= old.sequence || query.stamp.ended_at < old.ended_at
        }) {
            return Err(ResolverError::Stale.into());
        }
        validate_budget(&text)?;
        if let Some(translated) = &translated {
            validate_translations(&text, translated)?;
        }
        let stamp = query.stamp;
        let request_cancel = ResolverCancellation::default();
        let _request = CancelOnDrop(request_cancel.clone());
        let (reply, result) = oneshot::channel();
        self.sender
            .as_ref()
            .ok_or(TextLocalizationError::Cancelled)?
            .try_send(Work {
                query,
                text,
                translated,
                cancel: request_cancel,
                reply,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => TextLocalizationError::Overloaded,
                mpsc::TrySendError::Disconnected(_) => TextLocalizationError::WorkerPanicked,
            })?;
        self.previous = Some(stamp);
        let result = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(TextLocalizationError::Cancelled),
            result = tokio::time::timeout(CONVERSION_TIMEOUT, result) => result
                .map_err(|_| TextLocalizationError::Timeout)?
                .map_err(|_| TextLocalizationError::WorkerPanicked)?,
        }?;
        if cancel.is_cancelled() {
            return Err(TextLocalizationError::Cancelled);
        }
        stamp.validate(self.identity, Instant::now())?;
        Ok(result)
    }
    pub fn stop(&mut self) {
        self.shutdown.cancel();
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.previous = None;
    }
}
impl Drop for ChineseTextLocalizer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) use crate::resolver::text_session::validate_text_budget as validate_budget;

fn validate_translations(
    original: &ExistingText,
    translated: &[Box<str>],
) -> Result<(), ResolverError> {
    if !matches!(original.language, TextLanguage::Source(_))
        || translated.len() != original.cues.len()
        || translated
            .iter()
            .any(|text| text.len() > crate::resolver::contracts::MAX_CUE_BYTES)
        || translated
            .iter()
            .try_fold(0usize, |bytes, text| bytes.checked_add(text.len()))
            .is_none_or(|bytes| bytes > MAX_SESSION_TEXT_BYTES)
    {
        return Err(ResolverError::InvalidData);
    }
    Ok(())
}
fn normalize_translated(
    identity: StreamIdentity,
    query: &TextQuery,
    original: ExistingText,
    translated: Vec<Box<str>>,
    cancel: &ResolverCancellation,
) -> ConversionResult {
    if cancel.is_cancelled() {
        return Err(TextLocalizationError::Cancelled);
    }
    query.stamp.validate(identity, Instant::now())?;
    validate_budget(&original)?;
    original.validate(query)?;
    validate_translations(&original, &translated)?;
    let mut output = Vec::with_capacity(translated.len());
    let mut bytes = 0usize;
    for text in translated {
        if cancel.is_cancelled() {
            return Err(TextLocalizationError::Cancelled);
        }
        if text.trim().is_empty()
            || text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(ResolverError::InvalidData.into());
        }
        let text = taiwan_phrase_conversion(&text)
            .map_err(|_| ResolverError::InvalidData)?
            .into_boxed_str();
        bytes = bytes
            .checked_add(text.len())
            .ok_or(ResolverError::InvalidData)?;
        if bytes > MAX_SESSION_TEXT_BYTES {
            return Err(ResolverError::InvalidData.into());
        }
        output.push(text);
    }
    if cancel.is_cancelled() {
        return Err(TextLocalizationError::Cancelled);
    }
    query.stamp.validate(identity, Instant::now())?;
    Ok(LocalizedText {
        stamp: query.stamp,
        original,
        translated: output,
    })
}
fn normalize(
    identity: StreamIdentity,
    query: &TextQuery,
    original: ExistingText,
    cancel: &ResolverCancellation,
) -> ConversionResult {
    if cancel.is_cancelled() {
        return Err(TextLocalizationError::Cancelled);
    }
    query.stamp.validate(identity, Instant::now())?;
    validate_budget(&original)?;
    original.validate(query)?;
    if let TextLanguage::Source(language) = original.language {
        return Err(TextLocalizationError::TranslationRequired(language));
    }
    let mut translated = Vec::with_capacity(original.cues.len());
    let mut bytes = 0usize;
    for cue in &original.cues {
        if cancel.is_cancelled() {
            return Err(TextLocalizationError::Cancelled);
        }
        let text = match original.language {
            TextLanguage::TaiwanTraditional => cue.text.clone(),
            TextLanguage::OtherTraditional | TextLanguage::SimplifiedChinese => {
                taiwan_phrase_conversion(&cue.text)
                    .map_err(|_| ResolverError::InvalidData)?
                    .into_boxed_str()
            }
            TextLanguage::Source(language) => {
                return Err(TextLocalizationError::TranslationRequired(language));
            }
        };
        bytes = bytes
            .checked_add(text.len())
            .ok_or(ResolverError::InvalidData)?;
        if bytes > MAX_SESSION_TEXT_BYTES {
            return Err(ResolverError::InvalidData.into());
        }
        translated.push(text);
    }
    if cancel.is_cancelled() {
        return Err(TextLocalizationError::Cancelled);
    }
    query.stamp.validate(identity, Instant::now())?;
    Ok(LocalizedText {
        stamp: query.stamp,
        original,
        translated,
    })
}

#[cfg(test)]
#[path = "existing_text_tests.rs"]
mod tests;
