//! Explicit, session-only HTTP byte ranges. No decoder, GUI, playback or cache.
use super::input::MediaInput;
use crate::{ai::speech::StreamIdentity, resolver::contracts::ResolverCancellation};
use reqwest::{
    Client, Url,
    header::{self, HeaderMap, HeaderValue},
};
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_REMOTE_RANGE_BYTES: usize = 64 * 1024;
pub const MAX_REMOTE_MEDIA_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_REMOTE_RANGE_REQUESTS: u64 = 65_536;
/// Includes in-flight reads and ranges retained by downstream consumers, even
/// after their source is retired. At most 256 KiB of owned range payload.
pub const MAX_REMOTE_OUTSTANDING_RANGES: usize = 4;
/// Connection-owner quota, independent of CPU worker counts. Each owner has one
/// mutable reader, so there are at most two active media requests process-wide.
const MAX_REMOTE_MEDIA_OWNERS: usize = 2;
const MAX_ETAG_BYTES: usize = 256;
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RemoteMediaError {
    #[error("遠端媒體要求或讀取範圍無效。")]
    InvalidRequest,
    #[error("遠端媒體讀取需要可用的非同步執行環境。")]
    RuntimeUnavailable,
    #[error("遠端媒體不支援必要的固定來源分段讀取。")]
    UnsupportedRange,
    #[error("遠端媒體的回應格式或資料長度無效。")]
    InvalidResponse,
    #[error("遠端媒體已改變，請重新準備；未混用不同版本的內容。")]
    SourceChanged,
    #[error("遠端媒體超過本次讀取上限。")]
    TooLarge,
    #[error("遠端媒體讀取已達容量上限。")]
    Overloaded,
    #[error("遠端媒體要求逾時。")]
    Timeout,
    #[error("遠端媒體網路連線失敗。")]
    Network,
    #[error("遠端媒體要求受到速率限制。")]
    RateLimited,
    #[error("遠端媒體需要授權或無法存取。")]
    AccessDenied,
    #[error("遠端媒體回應重新導向；未自動連線至其他位置。")]
    Redirected,
    #[error("遠端媒體讀取已取消。")]
    Cancelled,
    #[error("遠端媒體要求不屬於目前工作階段或順序。")]
    Stale,
}
impl RemoteMediaError {
    /// Retrying requires a newly opened/validated owner; no stale owner resumes.
    pub fn retryable(self) -> bool {
        matches!(self, Self::Network | Self::Timeout | Self::RateLimited)
    }
}

/// No Debug/Clone/serialization: media bytes are not diagnostic payloads.
pub struct RemoteRangeRequest {
    identity: StreamIdentity,
    sequence: u64,
    offset: u64,
    length: usize,
}
impl RemoteRangeRequest {
    pub fn new(
        identity: StreamIdentity,
        sequence: u64,
        offset: u64,
        length: usize,
    ) -> Result<Self, RemoteMediaError> {
        if identity.session.0 == 0
            || identity.generation == 0
            || sequence == 0
            || sequence > MAX_REMOTE_RANGE_REQUESTS
            || !(1..=MAX_REMOTE_RANGE_BYTES).contains(&length)
            || offset.checked_add(length as u64).is_none()
        {
            return Err(RemoteMediaError::InvalidRequest);
        }
        Ok(Self {
            identity,
            sequence,
            offset,
            length,
        })
    }
}
pub struct RemoteMediaRange {
    request: RemoteRangeRequest,
    bytes: Box<[u8]>,
    cancel: ResolverCancellation,
    _permit: OwnedSemaphorePermit,
}
impl RemoteMediaRange {
    pub fn identity(&self) -> StreamIdentity {
        self.request.identity
    }
    pub fn sequence(&self) -> u64 {
        self.request.sequence
    }
    pub fn offset(&self) -> u64 {
        self.request.offset
    }
    pub fn bytes(&self) -> Result<&[u8], RemoteMediaError> {
        check(&self.cancel)?;
        Ok(&self.bytes)
    }
}
struct Representation {
    length: u64,
    etag: HeaderValue,
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
fn owners() -> Arc<Semaphore> {
    static OWNERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    OWNERS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_REMOTE_MEDIA_OWNERS)))
        .clone()
}
fn buffers() -> Arc<Semaphore> {
    static BUFFERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    BUFFERS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_REMOTE_OUTSTANDING_RANGES)))
        .clone()
}
/// Strong ETag and length are HTTP representation validators, not proof of
/// codec, content identification, playback or a match to any subtitle.
pub struct RemoteHttpMedia {
    identity: StreamIdentity,
    url: Url,
    client: Client,
    representation: Representation,
    cancel: ResolverCancellation,
    next_sequence: u64,
    request_timeout: Duration,
    buffers: Arc<Semaphore>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for RemoteHttpMedia {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl RemoteHttpMedia {
    /// Construction performs no I/O. Drop of even an unpolled open cancels the
    /// supplied scope. Callers should supply a child scope for this source and
    /// poll on a Tokio runtime with networking and timers enabled.
    pub fn open(
        input: MediaInput,
        identity: StreamIdentity,
        cancel: ResolverCancellation,
    ) -> impl Future<Output = Result<Self, RemoteMediaError>> + Send {
        Self::open_with(
            input,
            identity,
            cancel,
            owners(),
            buffers(),
            REMOTE_REQUEST_TIMEOUT,
        )
    }
    fn open_with(
        input: MediaInput,
        identity: StreamIdentity,
        cancel: ResolverCancellation,
        owners: Arc<Semaphore>,
        buffers: Arc<Semaphore>,
        timeout: Duration,
    ) -> impl Future<Output = Result<Self, RemoteMediaError>> + Send {
        let guard = CancelOnDrop {
            cancel: cancel.clone(),
            armed: true,
        };
        async move {
            let mut guard = guard;
            check(&cancel)?;
            if identity.session.0 == 0 || identity.generation == 0 {
                return Err(RemoteMediaError::InvalidRequest);
            }
            let url = input
                .into_remote_url()
                .map_err(|_| RemoteMediaError::InvalidRequest)?;
            tokio::runtime::Handle::try_current()
                .map_err(|_| RemoteMediaError::RuntimeUnavailable)?;
            let permit = owners
                .try_acquire_owned()
                .map_err(|_| RemoteMediaError::Overloaded)?;
            let client = Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(REMOTE_CONNECT_TIMEOUT)
                .timeout(timeout)
                .build()
                .map_err(|_| RemoteMediaError::Network)?;
            // One byte validates range semantics, representation length and the
            // strong validator. It is discarded, never retained as a cache.
            let (representation, _probe) =
                fetch(&client, &url, (0, 1), None, &cancel, timeout).await?;
            check(&cancel)?;
            guard.armed = false;
            Ok(Self {
                identity,
                url,
                client,
                representation,
                cancel,
                next_sequence: 1,
                request_timeout: timeout,
                buffers,
                _permit: permit,
            })
        }
    }
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn byte_len(&self) -> u64 {
        self.representation.length
    }
    /// Includes reads performed by a format gate; continuation must preserve it.
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
    pub(super) fn cancellation(&self) -> ResolverCancellation {
        self.cancel.clone()
    }
    #[cfg(test)]
    pub(super) fn open_for_test(
        input: MediaInput,
        identity: StreamIdentity,
        cancel: ResolverCancellation,
        owners: Arc<Semaphore>,
        buffers: Arc<Semaphore>,
    ) -> impl Future<Output = Result<Self, RemoteMediaError>> + Send {
        Self::open_with(
            input,
            identity,
            cancel,
            owners,
            buffers,
            Duration::from_secs(3),
        )
    }
    /// Reads are serialized by &mut self. Any failure or abandoned future retires
    /// the source and all previously delivered ranges; no partial result escapes.
    pub fn read_range(
        &mut self,
        request: RemoteRangeRequest,
    ) -> impl Future<Output = Result<RemoteMediaRange, RemoteMediaError>> + Send + '_ {
        let guard = CancelOnDrop {
            cancel: self.cancel.clone(),
            armed: true,
        };
        async move {
            let mut guard = guard;
            check(&self.cancel)?;
            if request.identity != self.identity || request.sequence != self.next_sequence {
                return Err(RemoteMediaError::Stale);
            }
            if request
                .offset
                .checked_add(request.length as u64)
                .is_none_or(|end| end > self.representation.length)
            {
                return Err(RemoteMediaError::InvalidRequest);
            }
            let permit = self
                .buffers
                .clone()
                .try_acquire_owned()
                .map_err(|_| RemoteMediaError::Overloaded)?;
            let (_, bytes) = fetch(
                &self.client,
                &self.url,
                (request.offset, request.length),
                Some(&self.representation),
                &self.cancel,
                self.request_timeout,
            )
            .await?;
            check(&self.cancel)?;
            self.next_sequence += 1; // Validated request sequence is at most 65536.
            guard.armed = false;
            Ok(RemoteMediaRange {
                request,
                bytes,
                cancel: self.cancel.clone(),
                _permit: permit,
            })
        }
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), RemoteMediaError> {
    if cancel.is_cancelled() {
        Err(RemoteMediaError::Cancelled)
    } else {
        Ok(())
    }
}
async fn fetch(
    client: &Client,
    url: &Url,
    range: (u64, usize),
    expected: Option<&Representation>,
    cancel: &ResolverCancellation,
    timeout: Duration,
) -> Result<(Representation, Box<[u8]>), RemoteMediaError> {
    check(cancel)?;
    tokio::runtime::Handle::try_current().map_err(|_| RemoteMediaError::RuntimeUnavailable)?;
    let work = async {
        let end = range
            .0
            .checked_add(range.1 as u64)
            .and_then(|value| value.checked_sub(1))
            .ok_or(RemoteMediaError::InvalidRequest)?;
        let mut request = client
            .get(url.clone())
            .header(header::RANGE, format!("bytes={}-{end}", range.0))
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::CACHE_CONTROL, "no-store");
        if let Some(expected) = expected {
            request = request.header(header::IF_MATCH, expected.etag.clone());
        }
        let mut response = request.send().await.map_err(http_error)?;
        let representation = validate_headers(
            response.status().as_u16(),
            response.headers(),
            range,
            expected,
        )?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(range.1)
            .map_err(|_| RemoteMediaError::Overloaded)?;
        while let Some(chunk) = response.chunk().await.map_err(http_error)? {
            check(cancel)?;
            if chunk.is_empty() || chunk.len() > range.1 - bytes.len() {
                return Err(RemoteMediaError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
            tokio::task::yield_now().await;
        }
        if bytes.len() != range.1 {
            return Err(RemoteMediaError::InvalidResponse);
        }
        check(cancel)?;
        Ok((representation, bytes.into_boxed_slice()))
    };
    tokio::select! {biased;
        _=cancel.cancelled()=>Err(RemoteMediaError::Cancelled),
        result=tokio::time::timeout(timeout,work)=>result.map_err(|_|RemoteMediaError::Timeout)?,
    }
}
fn http_error(error: reqwest::Error) -> RemoteMediaError {
    if error.is_timeout() {
        RemoteMediaError::Timeout
    } else {
        RemoteMediaError::Network
    }
}

#[path = "remote_http_headers.rs"]
mod headers;
use headers::validate_headers;
#[cfg(test)]
#[path = "remote_http_tests.rs"]
mod tests;
