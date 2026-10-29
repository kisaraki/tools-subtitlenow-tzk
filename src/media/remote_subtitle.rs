//! Explicit HTTPS whole-document subtitles for Prepared work. No Live lookup
//! stamp, content identification, media matching, GUI or persistent storage.
use super::{MediaError, input::MediaInput, work};
use crate::{
    ai::speech::StreamIdentity,
    resolver::{
        contracts::{ExistingText, ResolverCancellation, ResolverError, TextLanguage},
        timed_subtitle::SubtitleFormat,
        timed_subtitle_parse::{self, MAX_PREPARED_DOCUMENT_BYTES},
    },
};
use reqwest::{
    Client, Url,
    header::{self, HeaderMap},
};
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Each permit covers a request, its raw body/parse, and the retained result.
/// This bounds wire buffers to sixteen MiB, independently of decoded text.
pub const MAX_REMOTE_SUBTITLE_REQUESTS: usize = 2;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteSubtitleError {
    #[error("遠端字幕位置或工作階段設定無效。")]
    InvalidRequest,
    #[error("遠端字幕需要可用的非同步執行環境。")]
    RuntimeUnavailable,
    #[error("遠端字幕處理已達容量上限，未排入本次要求。")]
    Overloaded,
    #[error("遠端字幕超過本次準備容量上限。")]
    TooLarge,
    #[error("遠端字幕回應格式或資料長度無效。")]
    InvalidResponse,
    #[error("找不到明確選定的遠端字幕。")]
    NotFound,
    #[error("遠端字幕需要授權或無法存取。")]
    AccessDenied,
    #[error("遠端字幕要求受到速率限制。")]
    RateLimited,
    #[error("遠端字幕回應重新導向；未自動連線至其他位置。")]
    Redirected,
    #[error("遠端字幕要求逾時。")]
    Timeout,
    #[error("遠端字幕網路連線失敗。")]
    Network,
    #[error("遠端字幕準備已取消。")]
    Cancelled,
    #[error("遠端字幕解析工作異常結束。")]
    WorkerFailed,
    #[error("{0}")]
    Media(MediaError),
}

/// Flat transport failures can be retained by a Prepared Session without
/// recursively embedding RemoteSubtitleError::Media(MediaError).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteSubtitleFailure {
    #[error("遠端字幕位置或工作階段設定無效。")]
    InvalidRequest,
    #[error("遠端字幕需要可用的非同步執行環境。")]
    RuntimeUnavailable,
    #[error("遠端字幕處理已達容量上限，未排入本次要求。")]
    Overloaded,
    #[error("遠端字幕超過本次準備容量上限。")]
    TooLarge,
    #[error("遠端字幕回應格式或資料長度無效。")]
    InvalidResponse,
    #[error("找不到明確選定的遠端字幕。")]
    NotFound,
    #[error("遠端字幕需要授權或無法存取。")]
    AccessDenied,
    #[error("遠端字幕要求受到速率限制。")]
    RateLimited,
    #[error("遠端字幕回應重新導向；未自動連線至其他位置。")]
    Redirected,
    #[error("遠端字幕要求逾時。")]
    Timeout,
    #[error("遠端字幕網路連線失敗。")]
    Network,
    #[error("遠端字幕解析工作異常結束。")]
    WorkerFailed,
}
impl RemoteSubtitleError {
    pub(crate) fn into_media(self) -> MediaError {
        use RemoteSubtitleFailure as Failure;
        MediaError::RemoteSubtitle(match self {
            Self::Media(error) => return error,
            Self::Cancelled => return MediaError::Cancelled,
            Self::InvalidRequest => Failure::InvalidRequest,
            Self::RuntimeUnavailable => Failure::RuntimeUnavailable,
            Self::Overloaded => Failure::Overloaded,
            Self::TooLarge => Failure::TooLarge,
            Self::InvalidResponse => Failure::InvalidResponse,
            Self::NotFound => Failure::NotFound,
            Self::AccessDenied => Failure::AccessDenied,
            Self::RateLimited => Failure::RateLimited,
            Self::Redirected => Failure::Redirected,
            Self::Timeout => Failure::Timeout,
            Self::Network => Failure::Network,
            Self::WorkerFailed => Failure::WorkerFailed,
        })
    }
}

/// A declared format/language and private location; no inference or URL echo.
/// Constructing a location starts no network, file or provider work.
pub struct PreparedSubtitleUrl {
    url: Url,
    format: SubtitleFormat,
    language: TextLanguage,
}
impl PreparedSubtitleUrl {
    pub(crate) fn language(&self) -> TextLanguage {
        self.language
    }
    pub fn new(
        raw: &str,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> Result<Self, RemoteSubtitleError> {
        let url = Self::url(raw)?;
        if url.scheme() != "https" {
            return Err(RemoteSubtitleError::InvalidRequest);
        }
        Ok(Self {
            url,
            format,
            language,
        })
    }
    fn url(raw: &str) -> Result<Url, RemoteSubtitleError> {
        MediaInput::remote(raw)
            .and_then(MediaInput::into_remote_url)
            .map_err(|_| RemoteSubtitleError::InvalidRequest)
    }
    #[cfg(test)]
    pub(crate) fn local(
        raw: &str,
        format: SubtitleFormat,
        language: TextLanguage,
    ) -> Result<Self, RemoteSubtitleError> {
        let url = Self::url(raw)?;
        if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") {
            return Err(RemoteSubtitleError::InvalidRequest);
        }
        Ok(Self {
            url,
            format,
            language,
        })
    }
}

/// Session RAM only. Drop retires access and releases request/body capacity.
/// This result proves a parsed document, never a match to media or playback.
pub struct PreparedRemoteSubtitle {
    identity: StreamIdentity,
    sequence: u64,
    original: ExistingText,
    lease: PreparedRemoteSubtitleLease,
}
/// Moves with the original text into a prepared document. Scope and capacity
/// remain owned through analysis, translation, completion and Session Ready.
pub(crate) struct PreparedRemoteSubtitleLease {
    scope: CancelOnDrop,
    _permit: OwnedSemaphorePermit,
}
impl PreparedRemoteSubtitleLease {
    pub(crate) fn check(&self) -> Result<(), RemoteSubtitleError> {
        check(&self.scope.0)
    }
}
impl PreparedRemoteSubtitle {
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn original(&self) -> Result<&ExistingText, RemoteSubtitleError> {
        self.lease.check()?;
        Ok(&self.original)
    }
    pub(crate) fn into_parts(
        self,
    ) -> Result<(ExistingText, PreparedRemoteSubtitleLease), RemoteSubtitleError> {
        self.lease.check()?;
        Ok((self.original, self.lease))
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

#[cfg(test)]
type ParseHook = Arc<dyn Fn(&ResolverCancellation) + Send + Sync>;
struct FetchPolicy {
    cpu: Arc<Semaphore>,
    budget: Arc<Semaphore>,
    timeout: Duration,
    #[cfg(test)]
    before_parse: Option<ParseHook>,
}
fn requests() -> Arc<Semaphore> {
    static REQUESTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    REQUESTS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_REMOTE_SUBTITLE_REQUESTS)))
        .clone()
}

/// Passive construction, including Drop before the first poll. The linked child
/// scope is owned outside the async body so future Drop cancels only this work.
/// Full-document Prepared parsing deliberately has no Live capture deadline.
/// Poll on a Tokio runtime with I/O and timer drivers enabled. The runtime check
/// reports a missing runtime; it does not detect a caller-disabled driver.
pub fn fetch_remote_subtitle(
    input: PreparedSubtitleUrl,
    identity: StreamIdentity,
    sequence: u64,
    cancel: ResolverCancellation,
) -> impl Future<Output = Result<PreparedRemoteSubtitle, RemoteSubtitleError>> + Send {
    fetch_with(
        input,
        identity,
        sequence,
        cancel,
        FetchPolicy {
            cpu: work::workers(),
            budget: requests(),
            timeout: REQUEST_TIMEOUT,
            #[cfg(test)]
            before_parse: None,
        },
    )
}
fn fetch_with(
    input: PreparedSubtitleUrl,
    identity: StreamIdentity,
    sequence: u64,
    parent: ResolverCancellation,
    policy: FetchPolicy,
) -> impl Future<Output = Result<PreparedRemoteSubtitle, RemoteSubtitleError>> + Send {
    let scope = ResolverCancellation::linked(parent)
        .map(CancelOnDrop)
        .map_err(|_| RemoteSubtitleError::InvalidRequest);
    async move {
        let scope = scope?;
        check(&scope.0)?;
        if identity.session.0 == 0 || identity.generation == 0 || sequence == 0 {
            return Err(RemoteSubtitleError::InvalidRequest);
        }
        tokio::runtime::Handle::try_current()
            .map_err(|_| RemoteSubtitleError::RuntimeUnavailable)?;
        let permit = policy
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| RemoteSubtitleError::Overloaded)?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .https_only(!cfg!(test))
            .user_agent(concat!("SubtitleNow/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT.min(policy.timeout))
            .timeout(policy.timeout)
            .pool_max_idle_per_host(1)
            .build()
            .map_err(|_| RemoteSubtitleError::Network)?;
        let job = async {
            let bytes = document(&client, &input, &scope.0).await?;
            check(&scope.0)?;
            let cpu = policy
                .cpu
                .clone()
                .try_acquire_owned()
                .map_err(|_| RemoteSubtitleError::Overloaded)?;
            let cancel = scope.0.clone();
            let content_id = format!(
                "prepared:{}:{}:{sequence}",
                identity.session.0, identity.generation
            );
            let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
                let _cpu = cpu;
                let permit = permit;
                #[cfg(test)]
                if let Some(hook) = policy.before_parse {
                    hook(&cancel);
                }
                check(&cancel)?;
                let original = timed_subtitle_parse::parse_prepared(
                    input.format,
                    &bytes,
                    identity,
                    &content_id,
                    input.language,
                    &cancel,
                )
                .map_err(parse_error)?;
                drop(bytes);
                check(&cancel)?;
                Ok::<_, RemoteSubtitleError>((original, permit))
            }));
            (&mut worker.0)
                .await
                .map_err(|_| RemoteSubtitleError::WorkerFailed)?
        };
        let (original, permit) = tokio::select! { biased;
            _ = scope.0.cancelled() => return Err(RemoteSubtitleError::Cancelled),
            result = tokio::time::timeout(policy.timeout, job) => result.map_err(|_| RemoteSubtitleError::Timeout)??,
        };
        check(&scope.0)?;
        Ok(PreparedRemoteSubtitle {
            identity,
            sequence,
            original,
            lease: PreparedRemoteSubtitleLease {
                scope,
                _permit: permit,
            },
        })
    }
}
async fn document(
    client: &Client,
    input: &PreparedSubtitleUrl,
    cancel: &ResolverCancellation,
) -> Result<Vec<u8>, RemoteSubtitleError> {
    check(cancel)?;
    let accept = match input.format {
        SubtitleFormat::Srt => "application/x-subrip, text/plain",
        SubtitleFormat::WebVtt => "text/vtt, text/plain",
    };
    let mut response = client
        .get(input.url.clone())
        .header(header::ACCEPT, accept)
        .header(header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(http_error)?;
    match response.status().as_u16() {
        200 => {}
        401 | 403 => return Err(RemoteSubtitleError::AccessDenied),
        404 => return Err(RemoteSubtitleError::NotFound),
        429 => return Err(RemoteSubtitleError::RateLimited),
        300..=399 => return Err(RemoteSubtitleError::Redirected),
        _ => return Err(RemoteSubtitleError::Network),
    }
    let length = response_length(response.headers(), input.format)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length.unwrap_or(0))
        .map_err(|_| RemoteSubtitleError::Overloaded)?;
    while let Some(chunk) = response.chunk().await.map_err(http_error)? {
        check(cancel)?;
        if chunk.len() > MAX_PREPARED_DOCUMENT_BYTES.saturating_sub(bytes.len()) {
            return Err(RemoteSubtitleError::TooLarge);
        }
        if length.is_some_and(|length| chunk.len() > length.saturating_sub(bytes.len())) {
            return Err(RemoteSubtitleError::InvalidResponse);
        }
        // Exact reservation avoids geometric Vec growth beyond the wire budget
        // for unknown-length responses. Allocator bookkeeping is not payload.
        bytes
            .try_reserve_exact(chunk.len())
            .map_err(|_| RemoteSubtitleError::Overloaded)?;
        bytes.extend_from_slice(&chunk);
    }
    if length.is_some_and(|length| bytes.len() != length) {
        return Err(RemoteSubtitleError::InvalidResponse);
    }
    check(cancel)?;
    Ok(bytes)
}
fn response_length(
    headers: &HeaderMap,
    format: SubtitleFormat,
) -> Result<Option<usize>, RemoteSubtitleError> {
    if !content_type(headers, format) {
        return Err(RemoteSubtitleError::InvalidResponse);
    }
    let encoding = headers.get_all(header::CONTENT_ENCODING);
    if encoding.iter().count() > 1
        || encoding.iter().next().is_some_and(|value| {
            !value
                .to_str()
                .is_ok_and(|value| value.eq_ignore_ascii_case("identity"))
        })
    {
        return Err(RemoteSubtitleError::InvalidResponse);
    }
    let lengths = headers.get_all(header::CONTENT_LENGTH);
    let transfers = headers.get_all(header::TRANSFER_ENCODING);
    if lengths.iter().count() > 1
        || transfers.iter().count() > 1
        || (lengths.iter().next().is_some() && transfers.iter().next().is_some())
        || transfers.iter().next().is_some_and(|value| {
            !value
                .to_str()
                .is_ok_and(|value| value.eq_ignore_ascii_case("chunked"))
        })
    {
        return Err(RemoteSubtitleError::InvalidResponse);
    }
    if lengths.iter().next().is_none() {
        return Ok(None);
    }
    let raw = lengths
        .iter()
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or(RemoteSubtitleError::InvalidResponse)?;
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RemoteSubtitleError::InvalidResponse);
    }
    let length = raw
        .parse::<u64>()
        .map_err(|_| RemoteSubtitleError::TooLarge)?;
    if length > MAX_PREPARED_DOCUMENT_BYTES as u64 {
        return Err(RemoteSubtitleError::TooLarge);
    }
    Ok(Some(length as usize))
}
fn content_type(headers: &HeaderMap, format: SubtitleFormat) -> bool {
    let values = headers.get_all(header::CONTENT_TYPE);
    if values.iter().count() != 1 {
        return false;
    }
    let Some(value) = values.iter().next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let mut parts = value.split(';');
    let media = parts.next().unwrap_or_default().trim();
    let declared = match format {
        SubtitleFormat::Srt => "application/x-subrip",
        SubtitleFormat::WebVtt => "text/vtt",
    };
    if !media.eq_ignore_ascii_case("text/plain") && !media.eq_ignore_ascii_case(declared) {
        return false;
    }
    let mut charset = false;
    for parameter in parts {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return false;
        };
        if charset
            || !name.trim().eq_ignore_ascii_case("charset")
            || !(value.trim().eq_ignore_ascii_case("utf-8")
                || value.trim().eq_ignore_ascii_case("\"utf-8\""))
        {
            return false;
        }
        charset = true;
    }
    true
}
fn check(cancel: &ResolverCancellation) -> Result<(), RemoteSubtitleError> {
    if cancel.is_cancelled() {
        Err(RemoteSubtitleError::Cancelled)
    } else {
        Ok(())
    }
}
fn parse_error(error: ResolverError) -> RemoteSubtitleError {
    match error {
        ResolverError::Cancelled => RemoteSubtitleError::Cancelled,
        ResolverError::UnsupportedCapability => {
            RemoteSubtitleError::Media(MediaError::UnsupportedSubtitle)
        }
        _ => RemoteSubtitleError::Media(MediaError::InvalidSubtitle),
    }
}
fn http_error(error: reqwest::Error) -> RemoteSubtitleError {
    if error.is_timeout() {
        RemoteSubtitleError::Timeout
    } else {
        RemoteSubtitleError::Network
    }
}

#[cfg(test)]
#[path = "remote_subtitle_tests.rs"]
mod tests;
