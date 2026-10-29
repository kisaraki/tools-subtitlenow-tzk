//! Current Session metadata for known content. One song and one movie only;
//! publishers provide actual verified facts, never identification or confidence.
use super::{
    lyrics_lrc::{LyricsMetadata, LyricsMetadataSource},
    subtitles_http::{SubtitleMetadata, SubtitleMetadataSource},
};
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        contracts::{
            MAX_METADATA_BYTES, MovieCandidate, ResolverCancellation, ResolverError, SongCandidate,
            TextLanguage, WindowStamp, metadata,
        },
        lyrics::{MAX_TRACK_DURATION_US, MIN_TRACK_DURATION_US},
    },
    ui::content::ContentDisclosure,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

struct SongKey {
    id: Box<str>,
    title: Box<str>,
    artist: Option<Box<str>>,
}
impl SongKey {
    fn matches(&self, song: &SongCandidate) -> bool {
        self.id == song.id && self.title == song.title && self.artist == song.artist
    }
}
struct MovieKey {
    id: Box<str>,
    title: Box<str>,
    year: Option<u16>,
}
impl MovieKey {
    fn matches(&self, movie: &MovieCandidate) -> bool {
        self.id == movie.id && self.title == movie.title && self.year == movie.year
    }
}
struct LyricsEntry {
    key: SongKey,
    revision: u64,
    value: LyricsMetadata,
    availability: ResolverCancellation,
}
struct SubtitleEntry {
    key: MovieKey,
    revision: u64,
    value: SubtitleMetadata,
    availability: ResolverCancellation,
}
#[derive(Default)]
struct State {
    stopped: bool,
    lyrics_stamp: Option<WindowStamp>,
    subtitle_stamp: Option<WindowStamp>,
    lyrics: Option<LyricsEntry>,
    subtitles: Option<SubtitleEntry>,
}
struct Inner {
    identity: StreamIdentity,
    state: Mutex<State>,
    availability: ResolverCancellation,
}
/// One owner controls lifetime; readers cannot retain valid metadata after Drop.
/// Captured text/audio, lookup replies and identifiers are never saved to disk.
pub struct SessionTextMetadata {
    inner: Arc<Inner>,
}
pub struct SessionTextMetadataReader {
    inner: Arc<Inner>,
}
impl SessionTextMetadata {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            inner: Arc::new(Inner {
                identity,
                state: Mutex::new(State::default()),
                availability: ResolverCancellation::default(),
            }),
        }
    }
    pub fn reader(&self) -> Arc<SessionTextMetadataReader> {
        Arc::new(SessionTextMetadataReader {
            inner: self.inner.clone(),
        })
    }
    /// Explicit LRCLIB composition. No lookup or automatic activation occurs here.
    pub fn lyrics_configuration(
        &self,
    ) -> Result<super::lyrics::LyricsConfiguration, ResolverError> {
        super::lyrics_lrc::configuration(self.reader())
    }
    /// Describe every HTTPS text destination before giving this configuration to Live.
    pub fn subtitle_configuration(
        &self,
        disclosure: ContentDisclosure,
    ) -> Result<super::subtitles::SubtitleConfiguration, ResolverError> {
        super::subtitles_http::configuration(disclosure, self.reader())
    }
    /// Missing language stays unknown. Do not infer lyrics language from STT/UI.
    pub fn publish_lyrics(
        &self,
        stamp: WindowStamp,
        song: &SongCandidate,
        value: LyricsMetadata,
    ) -> Result<bool, ResolverError> {
        stamp.validate(self.inner.identity, Instant::now())?;
        if value.identity != self.inner.identity || value.content_id != song.id {
            return Err(ResolverError::ContentMismatch);
        }
        if !bounded(&song.id)
            || !bounded(&song.title)
            || song
                .artist
                .as_deref()
                .is_some_and(|artist| !bounded(artist))
            || value.album.as_deref().is_some_and(|album| !bounded(album))
            || value.duration_us.is_some_and(|duration| {
                !(MIN_TRACK_DURATION_US..=MAX_TRACK_DURATION_US).contains(&duration)
            })
            || value.availability.is_some()
        {
            return Err(ResolverError::InvalidData);
        }
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        advance(&state, state.lyrics_stamp, stamp)?;
        let same = state.lyrics.as_ref().is_some_and(|entry| {
            entry.key.matches(song)
                && entry.value.language == value.language
                && entry.value.album == value.album
                && entry.value.duration_us == value.duration_us
                && !entry.availability.is_cancelled()
        });
        state.lyrics_stamp = Some(stamp);
        if same {
            return Ok(false);
        }
        let availability = ResolverCancellation::linked(self.inner.availability.clone())?;
        if let Some(previous) = state.lyrics.take() {
            previous.availability.cancel();
        }
        state.lyrics = Some(LyricsEntry {
            key: SongKey {
                id: song.id.clone(),
                title: song.title.clone(),
                artist: song.artist.clone(),
            },
            revision: stamp.sequence,
            value,
            availability,
        });
        Ok(true)
    }
    /// Locations, language and paired edition IDs must be supplied by the actual
    /// source. The store neither searches nor guesses them from a filename/title.
    pub fn publish_subtitle(
        &self,
        stamp: WindowStamp,
        movie: &MovieCandidate,
        value: SubtitleMetadata,
    ) -> Result<bool, ResolverError> {
        stamp.validate(self.inner.identity, Instant::now())?;
        if value.identity != self.inner.identity || value.content_id != movie.id {
            return Err(ResolverError::ContentMismatch);
        }
        if !bounded(&movie.id)
            || !bounded(&movie.title)
            || movie.year.is_some_and(|year| !(1..=9999).contains(&year))
            || value.availability.is_some()
            || (value.reference.is_some()
                && matches!(value.primary.language(), TextLanguage::Source(_)))
        {
            return Err(ResolverError::InvalidData);
        }
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        advance(&state, state.subtitle_stamp, stamp)?;
        let same = state.subtitles.as_ref().is_some_and(|entry| {
            entry.key.matches(movie)
                && entry.value.primary == value.primary
                && entry.value.reference == value.reference
                && !entry.availability.is_cancelled()
        });
        state.subtitle_stamp = Some(stamp);
        if same {
            return Ok(false);
        }
        let availability = ResolverCancellation::linked(self.inner.availability.clone())?;
        if let Some(previous) = state.subtitles.take() {
            previous.availability.cancel();
        }
        state.subtitles = Some(SubtitleEntry {
            key: MovieKey {
                id: movie.id.clone(),
                title: movie.title.clone(),
                year: movie.year,
            },
            revision: stamp.sequence,
            value,
            availability,
        });
        Ok(true)
    }
    /// A settled loss of metadata retires the current entry without replaying an
    /// older asynchronous reply. Other content types and their tokens are separate.
    pub fn clear_lyrics(&self, stamp: WindowStamp) -> Result<(), ResolverError> {
        stamp.validate(self.inner.identity, Instant::now())?;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        advance(&state, state.lyrics_stamp, stamp)?;
        state.lyrics_stamp = Some(stamp);
        if let Some(previous) = state.lyrics.take() {
            previous.availability.cancel();
        }
        Ok(())
    }
    pub fn clear_subtitle(&self, stamp: WindowStamp) -> Result<(), ResolverError> {
        stamp.validate(self.inner.identity, Instant::now())?;
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        advance(&state, state.subtitle_stamp, stamp)?;
        state.subtitle_stamp = Some(stamp);
        if let Some(previous) = state.subtitles.take() {
            previous.availability.cancel();
        }
        Ok(())
    }
    pub fn stop(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.availability.cancel();
        state.stopped = true;
        if let Some(previous) = state.lyrics.take() {
            previous.availability.cancel();
        }
        if let Some(previous) = state.subtitles.take() {
            previous.availability.cancel();
        }
    }
}
impl Drop for SessionTextMetadata {
    fn drop(&mut self) {
        self.stop();
    }
}
fn bounded(value: &str) -> bool {
    value.len() <= MAX_METADATA_BYTES && metadata(value)
}
fn advance(
    state: &State,
    previous: Option<WindowStamp>,
    stamp: WindowStamp,
) -> Result<(), ResolverError> {
    if state.stopped {
        return Err(ResolverError::Cancelled);
    }
    if previous.is_some_and(|old| stamp.sequence <= old.sequence || stamp.ended_at < old.ended_at) {
        return Err(ResolverError::Stale);
    }
    Ok(())
}
impl LyricsMetadataSource for SessionTextMetadataReader {
    fn metadata(
        &self,
        identity: StreamIdentity,
        song: &SongCandidate,
    ) -> Result<LyricsMetadata, ResolverError> {
        if identity != self.inner.identity {
            return Err(ResolverError::InvalidData);
        }
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        let entry = state.lyrics.as_ref().ok_or(ResolverError::NotConfigured)?;
        if !entry.key.matches(song) {
            return Err(ResolverError::ContentMismatch);
        }
        if entry.availability.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        Ok(LyricsMetadata {
            identity,
            content_id: entry.value.content_id.clone(),
            language: entry.value.language,
            album: entry.value.album.clone(),
            duration_us: entry.value.duration_us,
            availability: Some(entry.availability.clone()),
        })
    }
    fn revision(&self, identity: StreamIdentity, song: &SongCandidate) -> Option<u64> {
        if identity != self.inner.identity {
            return None;
        }
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state
            .lyrics
            .as_ref()
            .filter(|entry| !state.stopped && entry.key.matches(song))
            .map(|entry| entry.revision)
    }
}
impl SubtitleMetadataSource for SessionTextMetadataReader {
    fn metadata(
        &self,
        identity: StreamIdentity,
        movie: &MovieCandidate,
    ) -> Result<SubtitleMetadata, ResolverError> {
        if identity != self.inner.identity {
            return Err(ResolverError::InvalidData);
        }
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        let entry = state
            .subtitles
            .as_ref()
            .ok_or(ResolverError::NotConfigured)?;
        if !entry.key.matches(movie) {
            return Err(ResolverError::ContentMismatch);
        }
        if entry.availability.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        Ok(SubtitleMetadata {
            identity,
            content_id: entry.value.content_id.clone(),
            primary: entry.value.primary.clone(),
            reference: entry.value.reference.clone(),
            availability: Some(entry.availability.clone()),
        })
    }
    fn revision(&self, identity: StreamIdentity, movie: &MovieCandidate) -> Option<u64> {
        if identity != self.inner.identity {
            return None;
        }
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state
            .subtitles
            .as_ref()
            .filter(|entry| !state.stopped && entry.key.matches(movie))
            .map(|entry| entry.revision)
    }
}

#[cfg(test)]
#[path = "text_metadata_tests.rs"]
mod tests;
