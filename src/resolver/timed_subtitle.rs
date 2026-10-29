//! Explicit HTTPS subtitle locations, declared language/edition, Session RAM only.
//! No catalog, identification confidence, service selection, redirects or disk.
use super::{
    contracts::*,
    text_session::validate_bundle_budget,
    timed_subtitle_parse::{self, MAX_SUBTITLE_DOCUMENT_BYTES},
};
use crate::ai::speech::StreamIdentity;
use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_ENCODING, CONTENT_TYPE, RETRY_AFTER},
};
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::Semaphore;

pub const MAX_SUBTITLE_URL_BYTES: usize = 4096;
pub const SUBTITLE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);
/// One logical lookup per second, with at most two sequential document requests.
pub const MIN_LOOKUP_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_PARSE_JOBS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    WebVtt,
}
/// Locations may contain signed query credentials: intentionally no Debug/Serialize.
#[derive(Clone, PartialEq, Eq)]
pub struct SubtitleTrack {
    url: Url,
    format: SubtitleFormat,
    language: TextLanguage,
}
impl SubtitleTrack {
    pub(crate) fn language(&self) -> TextLanguage {
        self.language
    }
    pub fn new(
        url: &str,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> Result<Self, ResolverError> {
        Self::location(url, format, language, false)
    }
    fn location(
        url: &str,
        format: SubtitleFormat,
        language: TextLanguage,
        local: bool,
    ) -> Result<Self, ResolverError> {
        if url.is_empty()
            || url.len() > MAX_SUBTITLE_URL_BYTES
            || url.contains('\\')
            || url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        {
            return Err(ResolverError::InvalidData);
        }
        // Url normalizes empty userinfo and extra slashes; reject them first.
        if url
            .split_once("://")
            .and_then(|(_, tail)| tail.split(['/', '?', '#']).next())
            .is_none_or(|authority| authority.is_empty() || authority.contains('@'))
        {
            return Err(ResolverError::InvalidData);
        }
        let url = Url::parse(url).map_err(|_| ResolverError::InvalidData)?;
        let secure = url.scheme() == "https";
        // The HTTP exception exists only in test builds, for literal loopback fixtures.
        let loopback =
            cfg!(test) && local && url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
        if (!secure && !loopback)
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(ResolverError::InvalidData);
        }
        Ok(Self {
            url,
            format,
            language,
        })
    }
    #[cfg(test)]
    pub(crate) fn local(
        url: &str,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> Result<Self, ResolverError> {
        Self::location(url, format, language, true)
    }
}
/// Both edition IDs must come from actual provider metadata, not an echoed song ID.
#[derive(Clone, PartialEq, Eq)]
pub struct ReferenceTrack {
    track: SubtitleTrack,
    primary_edition_id: Box<str>,
    reference_edition_id: Box<str>,
}
impl ReferenceTrack {
    pub fn new(
        track: SubtitleTrack,
        primary_edition_id: Box<str>,
        reference_edition_id: Box<str>,
    ) -> Result<Self, ResolverError> {
        if !matches!(track.language, TextLanguage::Source(_))
            || !metadata(&primary_edition_id)
            || !metadata(&reference_edition_id)
            || primary_edition_id != reference_edition_id
        {
            return Err(ResolverError::InvalidData);
        }
        Ok(Self {
            track,
            primary_edition_id,
            reference_edition_id,
        })
    }
}
struct RateGate {
    next: Instant,
    denied: bool,
}
/// Explicit provider for an already selected release. It does not identify media
/// or infer document language/edition from URLs, filenames or ASR preferences.
pub struct TimedSubtitleProvider {
    identity: StreamIdentity,
    content_id: Box<str>,
    primary: SubtitleTrack,
    reference: Option<ReferenceTrack>,
    client: Client,
    requests: Semaphore,
    rate: Mutex<RateGate>,
    parser_workers: Arc<Semaphore>,
}
impl TimedSubtitleProvider {
    pub fn new(
        identity: StreamIdentity,
        content_id: Box<str>,
        primary: SubtitleTrack,
        reference: Option<ReferenceTrack>,
    ) -> Result<Self, ResolverError> {
        if !metadata(&content_id)
            || (reference.is_some() && matches!(primary.language, TextLanguage::Source(_)))
        {
            return Err(ResolverError::InvalidData);
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("SubtitleNow/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(SUBTITLE_LOOKUP_TIMEOUT)
            .timeout(SUBTITLE_LOOKUP_TIMEOUT)
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(60))
            .https_only(!cfg!(test))
            .build()
            .map_err(|_| ResolverError::Network)?;
        static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
        let parser_workers = WORKERS
            .get_or_init(|| {
                let cores = std::thread::available_parallelism().map_or(1, usize::from);
                Arc::new(Semaphore::new(cores.min(MAX_PARSE_JOBS)))
            })
            .clone();
        Ok(Self {
            identity,
            content_id,
            primary,
            reference,
            client,
            requests: Semaphore::new(1),
            rate: Mutex::new(RateGate {
                next: Instant::now(),
                denied: false,
            }),
            parser_workers,
        })
    }
    fn validate(&self, query: &TextQuery) -> Result<(), ResolverError> {
        query.stamp.validate(self.identity, Instant::now())?;
        query.validate(Instant::now())?;
        if query.source != SubtitleSource::ExistingSubtitle {
            return Err(ResolverError::UnsupportedCapability);
        }
        if query.content_id != self.content_id {
            return Err(ResolverError::ContentMismatch);
        }
        Ok(())
    }
    fn admit(&self) -> Result<(), ResolverError> {
        let mut rate = self.rate.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        if rate.denied || now < rate.next {
            return Err(ResolverError::RateLimited);
        }
        rate.next = now + MIN_LOOKUP_INTERVAL;
        Ok(())
    }
    fn cooldown(&self, response: &reqwest::Response) {
        let delay = match response.headers().get(RETRY_AFTER) {
            None => Some(DEFAULT_COOLDOWN),
            Some(value) => value.to_str().ok().and_then(|value| {
                if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
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
            None => rate.denied = true,
        }
    }
    async fn document(
        &self,
        track: &SubtitleTrack,
        cancel: &ResolverCancellation,
    ) -> Result<Option<Vec<u8>>, ResolverError> {
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        let accept = match track.format {
            SubtitleFormat::Srt => "application/x-subrip, text/plain",
            SubtitleFormat::WebVtt => "text/vtt, text/plain",
        };
        let mut response = self
            .client
            .get(track.url.clone())
            .header(ACCEPT, accept)
            .send()
            .await
            .map_err(http_error)?;
        match response.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            401 | 403 => return Err(ResolverError::Credential),
            429 => {
                self.cooldown(&response);
                return Err(ResolverError::RateLimited);
            }
            _ => return Err(ResolverError::Network),
        }
        if !content_type(response.headers().get(CONTENT_TYPE), track.format)
            || response
                .headers()
                .get(CONTENT_ENCODING)
                .is_some_and(|value| {
                    !value
                        .to_str()
                        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("identity"))
                })
            || response
                .content_length()
                .is_some_and(|length| length > MAX_SUBTITLE_DOCUMENT_BYTES as u64)
        {
            return Err(ResolverError::InvalidData);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            if chunk.len() > MAX_SUBTITLE_DOCUMENT_BYTES.saturating_sub(body.len()) {
                return Err(ResolverError::InvalidData);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }
    async fn bundle(
        &self,
        query: &TextQuery,
        cancel: ResolverCancellation,
    ) -> Result<Option<ExistingTextBundle>, ResolverError> {
        self.validate(query)?;
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        let _permit = self
            .requests
            .try_acquire()
            .map_err(|_| ResolverError::TextOverloaded)?;
        self.admit()?;
        let query_deadline = query
            .stamp
            .ended_at
            .checked_add(MAX_WINDOW_AGE)
            .ok_or(ResolverError::Stale)?;
        let deadline = query_deadline.min(
            Instant::now()
                .checked_add(SUBTITLE_LOOKUP_TIMEOUT)
                .ok_or(ResolverError::Timeout)?,
        );
        let job = ResolverCancellation::default();
        let _guard = CancelOnDrop(job.clone());
        let work = async {
            let Some(primary) = self.document(&self.primary, &job).await? else {
                return Ok(None);
            };
            let reference = if let Some(reference) = &self.reference {
                Some(
                    self.document(&reference.track, &job)
                        .await?
                        .ok_or(ResolverError::ContentMismatch)?,
                )
            } else {
                None
            };
            let permit = self
                .parser_workers
                .clone()
                .try_acquire_owned()
                .map_err(|_| ResolverError::TextOverloaded)?;
            let query = TextQuery {
                stamp: query.stamp,
                source: query.source,
                content_id: query.content_id.clone(),
            };
            let primary_format = self.primary.format;
            let primary_language = self.primary.language;
            let reference_data = self.reference.as_ref().map(|reference| {
                (
                    reference.track.format,
                    reference.track.language,
                    reference.primary_edition_id.clone(),
                    reference.reference_edition_id.clone(),
                )
            });
            let job = job.clone();
            let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let text = timed_subtitle_parse::parse(
                    primary_format,
                    &primary,
                    &query,
                    primary_language,
                    &job,
                )?;
                let source_reference = match reference.zip(reference_data) {
                    Some((body, (format, language, primary_edition_id, reference_edition_id))) => {
                        Some(SourceTextReference {
                            text: timed_subtitle_parse::parse(
                                format, &body, &query, language, &job,
                            )?,
                            primary_edition_id,
                            reference_edition_id,
                        })
                    }
                    None => None,
                };
                validate_bundle_budget(&text, source_reference.as_ref())?;
                let bundle = ExistingTextBundle {
                    text,
                    source_reference,
                };
                bundle.validate(&query)?;
                if job.is_cancelled() {
                    return Err(ResolverError::Cancelled);
                }
                query.validate(Instant::now())?;
                Ok::<_, ResolverError>(Some(bundle))
            }));
            (&mut worker.0)
                .await
                .map_err(|_| ResolverError::WorkerPanicked)?
        };
        let result = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
            _ = tokio::time::sleep_until(deadline.into()) => return Err(if Instant::now() >= query_deadline { ResolverError::Stale } else { ResolverError::Timeout }),
            result = work => result,
        }?;
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        self.validate(query)?;
        Ok(result)
    }
}
fn content_type(value: Option<&reqwest::header::HeaderValue>, format: SubtitleFormat) -> bool {
    let Some(value) = value.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let mut parts = value.split(';');
    let media = parts.next().unwrap_or_default().trim();
    if !media.eq_ignore_ascii_case("text/plain")
        && !media.eq_ignore_ascii_case(match format {
            SubtitleFormat::Srt => "application/x-subrip",
            SubtitleFormat::WebVtt => "text/vtt",
        })
    {
        return false;
    }
    for parameter in parts {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return false;
        };
        if !name.trim().eq_ignore_ascii_case("charset")
            || !(value.trim().eq_ignore_ascii_case("utf-8")
                || value.trim().eq_ignore_ascii_case("\"utf-8\""))
        {
            return false;
        }
    }
    true
}
fn http_error(error: reqwest::Error) -> ResolverError {
    if error.is_timeout() {
        ResolverError::Timeout
    } else {
        ResolverError::Network
    }
}
struct CancelOnDrop(ResolverCancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl ExistingTextProvider for TimedSubtitleProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingSubtitle
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move { Ok(self.bundle(query, cancel).await?.map(|bundle| bundle.text)) })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(self.bundle(query, cancel))
    }
}
#[cfg(test)]
#[path = "timed_subtitle_tests.rs"]
mod tests;
