//! Caller-owned selected MP4 subtitle extraction on the shared Prepared CPU lane.
//! Bounded metadata discovery and explicit selection, without cloud or playback.
#[cfg(all(test, windows))]
use super::input::open_media;
use super::{
    MediaError,
    input::{MediaInput, OpenedMedia},
};
use crate::{
    ai::speech::StreamIdentity,
    resolver::contracts::{ExistingText, ResolverCancellation, TextLanguage},
};
use std::{future::Future, sync::Arc};
use tokio::sync::Semaphore;

/// Container language is a hint, never proof of Chinese script or Taiwan usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mp4SubtitleLanguage {
    Chinese,
    Source(crate::ai::speech::SpeechLanguage),
    Undetermined,
}
/// Metadata only: no path, transcript, display name or serializable content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mp4SubtitleTrack {
    pub track_id: u32,
    pub language: Mp4SubtitleLanguage,
}
/// Shared with the Prepared choice budget. No unbounded discovery list.
pub const MAX_MP4_SUBTITLE_TRACKS: usize = super::planning::MAX_SUBTITLE_CHOICES;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mp4SubtitleTracks {
    tracks: [Option<Mp4SubtitleTrack>; MAX_MP4_SUBTITLE_TRACKS],
    len: usize,
}
impl Default for Mp4SubtitleTracks {
    fn default() -> Self {
        Self {
            tracks: [None; MAX_MP4_SUBTITLE_TRACKS],
            len: 0,
        }
    }
}
impl Mp4SubtitleTracks {
    pub fn len(self) -> usize {
        self.len
    }
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
    pub fn get(self, index: usize) -> Option<Mp4SubtitleTrack> {
        self.tracks.get(index).copied().flatten()
    }
    pub(crate) fn push(&mut self, info: Mp4SubtitleTrack) -> Result<(), MediaError> {
        if self.len == MAX_MP4_SUBTITLE_TRACKS {
            return Err(MediaError::TooLarge);
        }
        if info.track_id == 0
            || self.tracks[..self.len]
                .iter()
                .flatten()
                .any(|track| track.track_id == info.track_id)
        {
            return Err(MediaError::InvalidInput);
        }
        self.tracks[self.len] = Some(info);
        self.len += 1;
        Ok(())
    }
    pub(crate) fn single(self) -> Result<Mp4SubtitleTrack, MediaError> {
        if self.len > 1 {
            return Err(MediaError::AmbiguousSubtitle);
        }
        self.get(0).ok_or(MediaError::UnsupportedSubtitle)
    }
}

/// All supported tracks, in container order. No preferred-language guessing.
pub fn discover_local_mp4_subtitles(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
) -> impl Future<Output = Result<Mp4SubtitleTracks, MediaError>> + Send {
    run_with(identity, cancel, super::work::workers(), move |scope| {
        let media = super::input::open_media(&input, scope)?;
        #[cfg(windows)]
        {
            let mut file = media.try_clone_file(scope)?;
            super::local_audio_format::discover_mp4_subtitles(&mut file, media.byte_len(), scope)
        }
        #[cfg(not(windows))]
        {
            let _ = media;
            Err(MediaError::UnsupportedMedia)
        }
    })
}

/// Compatibility single-result API: multiple tracks return AmbiguousSubtitle.
/// This validates the same container,
/// descriptions, tables and ranges as extraction, but does not decode samples.
/// Its temporary source pin is released before returning. Prepare revalidates.
pub fn discover_local_mp4_subtitle(
    input: MediaInput,
    identity: StreamIdentity,
    cancel: ResolverCancellation,
) -> impl Future<Output = Result<Mp4SubtitleTrack, MediaError>> + Send {
    run_with(identity, cancel, super::work::workers(), move |scope| {
        let media = super::input::open_media(&input, scope)?;
        #[cfg(windows)]
        {
            let mut file = media.try_clone_file(scope)?;
            super::local_audio_format::discover_mp4_subtitle(&mut file, media.byte_len(), scope)
        }
        #[cfg(not(windows))]
        {
            let _ = media;
            Err(MediaError::UnsupportedMedia)
        }
    })
}

#[derive(Clone, Copy)]
pub struct Mp4SubtitleSelection {
    track_id: u32,
    language: TextLanguage,
}
impl Mp4SubtitleSelection {
    pub fn new(track_id: u32, language: TextLanguage) -> Result<Self, MediaError> {
        if track_id == 0 {
            return Err(MediaError::InvalidInput);
        }
        Ok(Self { track_id, language })
    }
    pub const fn track_id(self) -> u32 {
        self.track_id
    }
    pub const fn language(self) -> TextLanguage {
        self.language
    }
}
/// The exact source remains pinned across later translation/provider waits.
/// Dropping this owner releases its RAM text and source pin. No serialization.
pub struct ExtractedMp4Subtitle {
    pub(crate) media: OpenedMedia,
    pub(crate) text: ExistingText,
}
impl ExtractedMp4Subtitle {
    pub fn original(&self) -> &ExistingText {
        &self.text
    }
    pub fn media_byte_len(&self) -> u64 {
        self.media.byte_len()
    }
}
pub fn extract_local_mp4_subtitle(
    input: MediaInput,
    identity: StreamIdentity,
    selection: Mp4SubtitleSelection,
    cancel: ResolverCancellation,
) -> impl Future<Output = Result<ExtractedMp4Subtitle, MediaError>> + Send {
    run_with(identity, cancel, super::work::workers(), move |scope| {
        let media = super::input::open_media(&input, scope)?;
        read_pinned(media, identity, selection, scope)
    })
}
/// The caller already owns the Prepared CPU permit. Reuse the pin, not another
/// worker/permit or a reopened path; preserve it through later translation.
pub(crate) fn read_pinned(
    media: OpenedMedia,
    identity: StreamIdentity,
    selection: Mp4SubtitleSelection,
    scope: &ResolverCancellation,
) -> Result<ExtractedMp4Subtitle, MediaError> {
    #[cfg(windows)]
    {
        let mut file = media.try_clone_file(scope)?;
        let text = super::local_audio_format::read_mp4_subtitle(
            &mut file,
            media.byte_len(),
            identity,
            selection,
            scope,
        )?;
        check(scope)?;
        Ok(ExtractedMp4Subtitle { media, text })
    }
    #[cfg(not(windows))]
    {
        let _ = (media, identity, selection, scope);
        Err(MediaError::UnsupportedMedia)
    }
}
struct CancelOnDrop {
    cancel: ResolverCancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.cancel();
        }
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        Err(MediaError::Cancelled)
    } else {
        Ok(())
    }
}
fn run_with<T, F>(
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    workers: Arc<Semaphore>,
    operation: F,
) -> impl Future<Output = Result<T, MediaError>> + Send
where
    T: Send + 'static,
    F: FnOnce(&ResolverCancellation) -> Result<T, MediaError> + Send + 'static,
{
    // Created outside async: even an unpolled dropped request cancels its scope.
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        check(&cancel)?;
        if identity.session.0 == 0 || identity.generation == 0 {
            return Err(MediaError::InvalidInput);
        }
        let permit = workers
            .try_acquire_owned()
            .map_err(|_| MediaError::Overloaded)?;
        let worker_cancel = cancel.clone();
        let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
            // Running local work keeps its CPU permit until actual completion,
            // even if cancellation/Drop aborts the handle but cannot stop I/O.
            let _permit = permit;
            check(&worker_cancel)?;
            let result = operation(&worker_cancel)?;
            check(&worker_cancel)?;
            Ok(result)
        }));
        let result = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(MediaError::Cancelled),
            result = &mut worker.0 => result.map_err(|_| MediaError::WorkerFailed)?,
        };
        check(&cancel)?;
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }
}

#[cfg(all(test, windows))]
#[path = "embedded_mp4_tests.rs"]
mod tests;
