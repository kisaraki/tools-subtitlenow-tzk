//! Explicit, read-only LRCLIB integration. Only confirmed song metadata is sent;
//! PCM, fingerprints, STT, source app names and credentials never enter requests.
//! GUI/default startup does not create this adapter. Results remain Session RAM.
use super::{
    candidates::{Resolution, SONG_CONFIDENCE, SongState},
    contracts::*,
    lrc,
};
use crate::ai::speech::StreamIdentity;
use reqwest::{
    Client,
    header::{ACCEPT, CONTENT_TYPE, RETRY_AFTER},
};
use serde::Deserialize;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Semaphore;

const ENDPOINT: &str = "https://lrclib.net/api/get";
/// A stricter adapter budget than the general 4 MiB text contract, limiting JSON
/// decode/line parsing CPU on the orchestration task as well as network memory.
pub const MAX_RESPONSE_BYTES: usize = 128 * 1024;
pub const LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);
pub const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(500);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
pub(crate) const MIN_TRACK_DURATION_US: u64 = 1_000_000;
pub(crate) const MAX_TRACK_DURATION_US: u64 = 3600 * 1_000_000;
const DURATION_TOLERANCE_US: u64 = 2 * 1_000_000;

pub struct LyricsTrack {
    song: Arc<SongCandidate>,
    album: Option<Box<str>>,
    duration_us: Option<u64>,
    language: TextLanguage,
}
impl LyricsTrack {
    /// Album/full-track duration must come from actual metadata, never the audio
    /// window length. LRCLIB has no language field: callers supply known language.
    pub fn from_resolution(
        resolution: &Resolution,
        language: TextLanguage,
        album: Option<Box<str>>,
        duration_us: Option<u64>,
    ) -> Result<Self, ResolverError> {
        if resolution.kind != ContentType::Music || resolution.song_state != SongState::Confirmed {
            return Err(ResolverError::ContentMismatch);
        }
        let song = resolution.song.as_ref().ok_or(ResolverError::InvalidData)?;
        if song.confidence.permille() < SONG_CONFIDENCE
            || !metadata(&song.id)
            || !metadata(&song.title)
            || song
                .artist
                .as_deref()
                .is_none_or(|artist| !metadata(artist))
            || album.as_deref().is_some_and(|value| !metadata(value))
            || duration_us.is_some_and(|duration| {
                !(MIN_TRACK_DURATION_US..=MAX_TRACK_DURATION_US).contains(&duration)
            })
        {
            return Err(ResolverError::InvalidData);
        }
        Ok(Self {
            song: Arc::clone(song),
            album,
            duration_us,
            language,
        })
    }
}
struct RateGate {
    next: Instant,
    denied: bool,
}
/// One explicitly configured service per Session. All song bindings share its
/// request limit and server cooldown, including after songs change.
pub struct LrcLibService {
    identity: StreamIdentity,
    client: Client,
    endpoint: String,
    requests: Semaphore,
    rate: Mutex<RateGate>,
}
pub struct LrcLibLyrics {
    service: Arc<LrcLibService>,
    track: LyricsTrack,
}
impl LrcLibService {
    /// Configuration is explicit. No key, account registration, disk store or
    /// automatic lookup is performed here; the fixed endpoint is HTTPS-only.
    pub fn new(identity: StreamIdentity) -> Result<Arc<Self>, ResolverError> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("SubtitleNow/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(LOOKUP_TIMEOUT)
            .timeout(LOOKUP_TIMEOUT)
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| ResolverError::Network)?;
        Ok(Arc::new(Self {
            identity,
            client,
            endpoint: ENDPOINT.into(),
            requests: Semaphore::new(1),
            rate: Mutex::new(RateGate {
                next: Instant::now(),
                denied: false,
            }),
        }))
    }
    pub fn for_song(self: &Arc<Self>, track: LyricsTrack) -> LrcLibLyrics {
        LrcLibLyrics {
            service: Arc::clone(self),
            track,
        }
    }
    fn admit(&self) -> Result<(), ResolverError> {
        let mut rate = self.rate.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        if rate.denied || now < rate.next {
            return Err(ResolverError::RateLimited);
        }
        rate.next = now + MIN_REQUEST_INTERVAL;
        Ok(())
    }
    fn cooldown(&self, response: &reqwest::Response) {
        let header = response.headers().get(RETRY_AFTER);
        let delay = match header {
            None => Some(DEFAULT_COOLDOWN),
            Some(value) => value.to_str().ok().and_then(|value| {
                if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
                    value.parse::<u64>().ok().map(Duration::from_secs)
                } else {
                    httpdate::parse_http_date(value).ok().map(|date| {
                        date.duration_since(SystemTime::now())
                            .unwrap_or(Duration::ZERO)
                    })
                }
            }),
        };
        let mut rate = self.rate.lock().unwrap_or_else(|error| error.into_inner());
        match delay.and_then(|delay| Instant::now().checked_add(delay)) {
            Some(next) => rate.next = rate.next.max(next),
            // Invalid/overflowing server cooldown fails closed until this adapter
            // is dropped. Never shorten a server's cooldown to a local retry cap.
            None => rate.denied = true,
        }
    }
}
impl LrcLibLyrics {
    fn validate(&self, query: &TextQuery) -> Result<(), ResolverError> {
        query.validate(Instant::now())?;
        if query.stamp.identity != self.service.identity {
            return Err(ResolverError::InvalidData);
        }
        if !self.supports(query.source) {
            return Err(ResolverError::UnsupportedCapability);
        }
        if query.content_id != self.track.song.id {
            return Err(ResolverError::ContentMismatch);
        }
        Ok(())
    }
    async fn attempt(
        &self,
        query: &TextQuery,
        cancel: &ResolverCancellation,
    ) -> Result<Option<ExistingText>, ResolverError> {
        let mut parameters = vec![
            ("track_name", self.track.song.title.to_string()),
            (
                "artist_name",
                self.track
                    .song
                    .artist
                    .as_deref()
                    .ok_or(ResolverError::InvalidData)?
                    .to_owned(),
            ),
        ];
        if let Some(album) = &self.track.album {
            parameters.push(("album_name", album.to_string()));
        }
        if let Some(duration) = self.track.duration_us {
            parameters.push(("duration", (duration as f64 / 1_000_000.0).to_string()));
        }
        let mut response = self
            .service
            .client
            .get(&self.service.endpoint)
            .header(ACCEPT, "application/json")
            .query(&parameters)
            .send()
            .await
            .map_err(http_error)?;
        match response.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            429 => {
                self.service.cooldown(&response);
                return Err(ResolverError::RateLimited);
            }
            503 => {
                self.service.cooldown(&response);
                return Err(ResolverError::Network);
            }
            401 | 403 => return Err(ResolverError::Credential),
            500..=599 => return Err(ResolverError::Network),
            _ => return Err(ResolverError::InvalidData),
        }
        if !response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
            || response
                .content_length()
                .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ResolverError::InvalidData);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(bytes.len()) {
                return Err(ResolverError::InvalidData);
            }
            bytes.extend_from_slice(&chunk);
        }
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        let record: Record =
            serde_json::from_slice(&bytes).map_err(|_| ResolverError::InvalidData)?;
        let result = self.convert(record, query)?;
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        Ok(result)
    }
    fn convert(
        &self,
        record: Record,
        query: &TextQuery,
    ) -> Result<Option<ExistingText>, ResolverError> {
        let duration = record.duration_us()?;
        if record.id == 0
            || !metadata(&record.track_name)
            || !metadata(&record.artist_name)
            || record
                .album_name
                .as_deref()
                .is_some_and(|album| !album.is_empty() && !metadata(album))
        {
            return Err(ResolverError::InvalidData);
        }
        if key(&record.track_name) != key(&self.track.song.title)
            || key(&record.artist_name)
                != key(self
                    .track
                    .song
                    .artist
                    .as_deref()
                    .ok_or(ResolverError::InvalidData)?)
            || self.track.album.as_deref().is_some_and(|album| {
                record
                    .album_name
                    .as_deref()
                    .is_none_or(|returned| key(returned) != key(album))
            })
            || self
                .track
                .duration_us
                .is_some_and(|expected| expected.abs_diff(duration) > DURATION_TOLERANCE_US)
        {
            return Err(ResolverError::ContentMismatch);
        }
        let synced = record
            .synced_lyrics
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        let plain = record
            .plain_lyrics
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        if record.instrumental {
            if synced.is_some() || plain.is_some() {
                return Err(ResolverError::InvalidData);
            }
            return Ok(None);
        }
        let cues = match (synced, plain) {
            (Some(text), _) => lrc::synced(text, duration)?,
            (None, Some(text)) => lrc::plain(text)?,
            (None, None) => return Ok(None),
        };
        let text = ExistingText {
            identity: self.service.identity,
            source: SubtitleSource::ExistingLyrics,
            content_id: query.content_id.clone(),
            language: self.track.language,
            cues,
        };
        text.validate(query)?;
        Ok(Some(text))
    }
}
impl ExistingTextProvider for LrcLibLyrics {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingLyrics
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            self.validate(query)?;
            // Reject rather than enqueue concurrent requests; no stale lookup backlog.
            let _permit = self
                .service
                .requests
                .try_acquire()
                .map_err(|_| ResolverError::RateLimited)?;
            self.service.admit()?;
            let result = tokio::select! { biased;
                _ = cancel.cancelled() => Err(ResolverError::Cancelled),
                result = tokio::time::timeout(LOOKUP_TIMEOUT, self.attempt(query, &cancel)) => result.unwrap_or(Err(ResolverError::Timeout)),
            }?;
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            self.validate(query)?; // Also gate delayed 404/instrumental/no-text results.
            Ok(result)
        })
    }
}
fn key(input: &str) -> String {
    input
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}
fn http_error(error: reqwest::Error) -> ResolverError {
    if error.is_timeout() {
        ResolverError::Timeout
    } else {
        ResolverError::Network
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: u64,
    track_name: String,
    artist_name: String,
    album_name: Option<String>,
    duration: f64,
    instrumental: bool,
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}
impl Record {
    fn duration_us(&self) -> Result<u64, ResolverError> {
        if !self.duration.is_finite() || !(1.0..=3600.0).contains(&self.duration) {
            return Err(ResolverError::InvalidData);
        }
        Ok((self.duration * 1_000_000.0).round() as u64)
    }
}

#[cfg(test)]
#[path = "lyrics_tests.rs"]
mod tests;
