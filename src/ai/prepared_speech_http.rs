//! Explicit selected-provider offline speech transport. Payload encoding and
//! parsing stay on bounded workers; captured media never becomes a disk file.
use super::{
    credentials::ApiKey,
    prepared_recognition::PREPARED_RECOGNITION_TIMEOUT,
    prepared_speech::{
        PreparedSpeechCapabilities, PreparedSpeechError, PreparedSpeechFuture,
        PreparedSpeechRecognizer, PreparedSpeechRequest, PreparedSpeechResponse,
    },
    prepared_speech_wire::{MAX_ASR_HTTP_RESPONSE_BYTES, encode_with_lease, parse_with_lease},
    provider::SpeechProvider,
};
use crate::resolver::contracts::ResolverCancellation;
use reqwest::{
    Client, RequestBuilder, Response,
    header::{
        ACCEPT, AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER,
    },
};
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/audio/transcriptions";
const GEMINI_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";
const MAX_HTTP_RECOGNITIONS: usize = 2;
const MAX_ATTEMPTS: usize = 2;
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(200);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const BASE_RETRY_DELAY_MS: u64 = 100;
const RETRY_JITTER_VARIANTS: u64 = 76;
/// Longer valid server cooldowns remain on the shared clock, but this request
/// returns its typed status instead of truncating that cooldown and retrying.
const MAX_SERVER_RETRY_WAIT: Duration = Duration::from_secs(5);
const MAX_RETRY_AFTER_BYTES: usize = 128;

/// No Debug/Clone implementation: the pooled client retains sensitive headers.
/// Construction does not contact either provider or read a credential store.
pub struct HttpPreparedRecognizer {
    provider: SpeechProvider,
    client: Client,
    endpoint: String,
    next_request: Mutex<tokio::time::Instant>,
    requests: Arc<Semaphore>,
    request_limit: usize,
}
impl HttpPreparedRecognizer {
    pub fn new(provider: SpeechProvider, key: &ApiKey) -> Result<Self, PreparedSpeechError> {
        let mut headers = HeaderMap::new();
        let mut credential = match provider {
            SpeechProvider::OpenAi => {
                let bearer = zeroize::Zeroizing::new(format!("Bearer {}", key.expose()));
                HeaderValue::from_str(&bearer)
            }
            SpeechProvider::Gemini => HeaderValue::from_str(key.expose()),
        }
        .map_err(|_| PreparedSpeechError::Credential)?;
        credential.set_sensitive(true);
        match provider {
            SpeechProvider::OpenAi => headers.insert(AUTHORIZATION, credential),
            SpeechProvider::Gemini => headers.insert("x-goog-api-key", credential),
        };
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let client = Client::builder()
            .default_headers(headers)
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(PREPARED_RECOGNITION_TIMEOUT)
            .pool_max_idle_per_host(MAX_HTTP_RECOGNITIONS)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .build()
            .map_err(|_| PreparedSpeechError::Network)?;
        let request_limit = crate::media::work::worker_limit().min(MAX_HTTP_RECOGNITIONS);
        Ok(Self {
            provider,
            client,
            endpoint: match provider {
                SpeechProvider::OpenAi => OPENAI_ENDPOINT,
                SpeechProvider::Gemini => GEMINI_ENDPOINT,
            }
            .to_owned(),
            next_request: Mutex::new(tokio::time::Instant::now()),
            requests: Arc::new(Semaphore::new(request_limit)),
            request_limit,
        })
    }

    /// The guard is installed before returning even this private fixture seam.
    /// One budget includes admission, rate waits, encode, retries and parsing.
    fn recognize_with_timeout<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
        timeout: Duration,
    ) -> PreparedSpeechFuture<'a> {
        // Provider cleanup owns a child scope. A codec failure may cancel its
        // operation while returning a more precise typed error; it must not
        // overwrite the caller's request token or turn that error into Cancelled.
        // Link and install the guard before boxing, covering unpolled Drop.
        let operation = ResolverCancellation::linked(cancel.clone())
            .map_err(|_| PreparedSpeechError::InvalidData);
        let guard = operation.as_ref().ok().map(|scope| CancelOnDrop {
            cancel: scope.clone(),
            armed: true,
        });
        Box::pin(async move {
            let mut guard = guard;
            check(&cancel)?;
            let operation = operation?;
            if timeout.is_zero() {
                return Err(PreparedSpeechError::Timeout);
            }
            let started = std::time::Instant::now();
            let result = tokio::select! { biased;
                _ = cancel.cancelled() => Err(PreparedSpeechError::Cancelled),
                result = tokio::time::timeout(timeout, self.run(request, &operation)) => {
                    result.map_err(|_| PreparedSpeechError::Timeout)?
                },
            };
            check(&cancel)?;
            if started.elapsed() >= timeout {
                return Err(PreparedSpeechError::Timeout);
            }
            if result.is_ok()
                && let Some(guard) = &mut guard
            {
                guard.armed = false;
            }
            result
        })
    }

    async fn run(
        &self,
        request: &PreparedSpeechRequest,
        cancel: &ResolverCancellation,
    ) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
        check(cancel)?;
        self.capabilities()
            .validate(request.language(), request.chunk().stamp().rate)?;
        check(cancel)?;
        // No request scheduling queue: an overload cannot retain another PCM
        // request waiting for this client's capacity.
        let permit = self
            .requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| PreparedSpeechError::Overloaded)?;
        let lease: Arc<OwnedSemaphorePermit> = Arc::new(permit);
        let payload = encode_with_lease(self.provider, request, cancel, lease.clone()).await?;
        check(cancel)?;
        let content_type = HeaderValue::from_str(&payload.content_type)
            .map_err(|_| PreparedSpeechError::InvalidData)?;
        // RequestBuilder clones the reusable in-memory bytes cheaply for a
        // bounded retry; it does not clone or re-encode all PCM on the executor.
        let template = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, content_type)
            .body(payload.body);
        for attempt in 0..MAX_ATTEMPTS {
            check(cancel)?;
            self.wait_rate(cancel).await?;
            let builder = template
                .try_clone()
                .ok_or(PreparedSpeechError::InvalidData)?;
            match self.attempt(builder, cancel).await {
                Ok(body) => {
                    check(cancel)?;
                    let response =
                        parse_with_lease(self.provider, body, request, cancel, lease.clone())
                            .await?;
                    check(cancel)?;
                    response.validate_for(request)?;
                    return Ok(response);
                }
                Err(failure) if failure.retry && attempt + 1 < MAX_ATTEMPTS => {
                    let delay = Duration::from_millis(
                        BASE_RETRY_DELAY_MS
                            + request.chunk().stamp().sequence % RETRY_JITTER_VARIANTS,
                    );
                    tokio::select! { biased;
                        _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled),
                        _ = tokio::time::sleep(delay) => {},
                    }
                }
                Err(failure) => return Err(failure.error),
            }
        }
        Err(PreparedSpeechError::Network)
    }

    async fn wait_rate(&self, cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
        loop {
            check(cancel)?;
            let mut next = tokio::select! { biased;
                _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled),
                next = self.next_request.lock() => next,
            };
            let now = tokio::time::Instant::now();
            if *next <= now {
                *next = now
                    .checked_add(MIN_REQUEST_INTERVAL)
                    .ok_or(PreparedSpeechError::RateLimited)?;
                drop(next);
                return check(cancel);
            }
            let deadline = *next;
            drop(next); // Never hold the clock mutex over a sleep or HTTP I/O.
            tokio::select! { biased;
                _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => {},
            }
        }
    }

    async fn attempt(
        &self,
        request: RequestBuilder,
        cancel: &ResolverCancellation,
    ) -> Result<Vec<u8>, AttemptFailure> {
        check(cancel).map_err(AttemptFailure::from)?;
        let mut response = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled.into()),
            response = request.send() => response.map_err(http_error).map_err(AttemptFailure::from)?,
        };
        check(cancel).map_err(AttemptFailure::from)?;
        if response.status().as_u16() != 200 {
            let mut failure = AttemptFailure::from(classify_status(response.status().as_u16()));
            if failure.retry
                && let Some(header) = response.headers().get(RETRY_AFTER)
            {
                match retry_after(header).and_then(|delay| {
                    tokio::time::Instant::now()
                        .checked_add(delay)
                        .map(|deadline| (delay, deadline))
                }) {
                    Some((delay, deadline)) => {
                        let mut next = tokio::select! { biased;
                            _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled.into()),
                            next = self.next_request.lock() => next,
                        };
                        *next = (*next).max(deadline);
                        failure.retry = delay <= MAX_SERVER_RETRY_WAIT;
                    }
                    None => failure.retry = false,
                }
            }
            // Discard error responses without reading or logging their bodies.
            return Err(failure);
        }
        if !json_content_type(response.headers().get(CONTENT_TYPE))
            || response
                .headers()
                .get(CONTENT_ENCODING)
                .is_some_and(|value| value.as_bytes() != b"identity")
            || response
                .content_length()
                .is_some_and(|length| length > MAX_ASR_HTTP_RESPONSE_BYTES as u64)
        {
            return Err(PreparedSpeechError::InvalidData.into());
        }
        consume(&mut response, cancel)
            .await
            .map_err(AttemptFailure::from)
    }
}
impl PreparedSpeechRecognizer for HttpPreparedRecognizer {
    fn capabilities(&self) -> PreparedSpeechCapabilities {
        PreparedSpeechCapabilities {
            cancellation: true,
            timestamped_finals: true,
            supported_languages: [true; 5],
            // Neither provider documents a strict five-language-only Auto mode.
            constrained_auto_detection: false,
            sample_rates: [true; 2],
            max_concurrent: self.request_limit,
        }
    }
    fn recognize<'a>(
        &'a self,
        request: &'a PreparedSpeechRequest,
        cancel: ResolverCancellation,
    ) -> PreparedSpeechFuture<'a> {
        self.recognize_with_timeout(request, cancel, PREPARED_RECOGNITION_TIMEOUT)
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
#[derive(Clone, Copy)]
struct AttemptFailure {
    error: PreparedSpeechError,
    retry: bool,
}
impl From<PreparedSpeechError> for AttemptFailure {
    fn from(error: PreparedSpeechError) -> Self {
        Self {
            error,
            retry: error.retryable(),
        }
    }
}
fn classify_status(status: u16) -> PreparedSpeechError {
    match status {
        401 | 403 => PreparedSpeechError::Credential,
        429 => PreparedSpeechError::RateLimited,
        500..=599 => PreparedSpeechError::Network,
        _ => PreparedSpeechError::InvalidData,
    }
}
fn http_error(error: reqwest::Error) -> PreparedSpeechError {
    if error.is_timeout() {
        PreparedSpeechError::Timeout
    } else {
        PreparedSpeechError::Network
    }
}
fn json_content_type(value: Option<&HeaderValue>) -> bool {
    value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
}
fn retry_after(value: &HeaderValue) -> Option<Duration> {
    if value.as_bytes().len() > MAX_RETRY_AFTER_BYTES {
        return None;
    }
    let value = value.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(
        date.duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}
async fn consume(
    response: &mut Response,
    cancel: &ResolverCancellation,
) -> Result<Vec<u8>, PreparedSpeechError> {
    let mut body = Vec::new();
    loop {
        check(cancel)?;
        let chunk = tokio::select! { biased;
            _ = cancel.cancelled() => return Err(PreparedSpeechError::Cancelled),
            chunk = response.chunk() => chunk.map_err(http_error)?,
        };
        check(cancel)?;
        let Some(chunk) = chunk else {
            break;
        };
        let length = body
            .len()
            .checked_add(chunk.len())
            .filter(|length| *length <= MAX_ASR_HTTP_RESPONSE_BYTES)
            .ok_or(PreparedSpeechError::InvalidData)?;
        body.try_reserve_exact(length - body.len())
            .map_err(|_| PreparedSpeechError::Overloaded)?;
        body.extend_from_slice(&chunk);
    }
    check(cancel)?;
    Ok(body)
}
fn check(cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
    if cancel.is_cancelled() {
        Err(PreparedSpeechError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "prepared_speech_http_tests.rs"]
mod tests;
