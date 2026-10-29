//! One selected-source title observed by the native owner. Captions are private,
//! bounded, short-lived RAM metadata; this buffer performs no I/O or inference.
use super::movie::{MovieHintMetadata, MovieMetadataSource, MovieMetadataValidity};
use crate::{
    ai::speech::StreamIdentity,
    resolver::contracts::{
        MAX_METADATA_BYTES, ResolverCancellation, ResolverError, WindowStamp, metadata,
    },
};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// Native title scans must be recent independently of the capture window's age.
/// UI receipt or a newer query never renews an earlier observation.
pub(crate) const MAX_SOURCE_METADATA_AGE: Duration = Duration::from_secs(2);

pub(crate) struct SourceMetadataBuffer {
    identity: StreamIdentity,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    stopped: bool,
    sequence: Option<u64>,
    observed_at: Option<Instant>,
    latest: Option<Title>,
    error: Option<ResolverError>,
}
struct Title {
    text: Box<str>,
    observed_at: Instant,
    expires_at: Instant,
    cancellation: ResolverCancellation,
}
impl State {
    fn clear(&mut self) {
        if let Some(previous) = self.latest.take() {
            previous.cancellation.cancel();
        }
    }

    fn fail(&mut self, error: ResolverError) {
        self.clear();
        self.error = Some(error);
    }
}
impl SourceMetadataBuffer {
    pub(crate) fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            state: Mutex::new(State::default()),
        }
    }

    /// Publish a complete native scan, including absence or failure. The caller
    /// validates HWND/process ownership and rejects truncated captions before
    /// publication. This method validates the bounded text and original scan time.
    pub(crate) fn publish(
        &self,
        sequence: u64,
        observed_at: Instant,
        title: Result<Box<str>, ResolverError>,
    ) -> Result<(), ResolverError> {
        self.publish_at(sequence, observed_at, title, Instant::now())
    }

    fn publish_at(
        &self,
        sequence: u64,
        observed_at: Instant,
        title: Result<Box<str>, ResolverError>,
        now: Instant,
    ) -> Result<(), ResolverError> {
        // Only one <=1 KiB title and scalar metadata can be retained. No user
        // callback or asynchronous work runs while holding this short lock.
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        // Obsolete callbacks cannot clear a newer source, even when their title
        // or timestamp is malformed. Keep ordering watermarks after clearing.
        if state.sequence.is_some_and(|previous| sequence <= previous)
            || state
                .observed_at
                .is_some_and(|previous| observed_at < previous)
        {
            return Err(ResolverError::Stale);
        }
        state.sequence = Some(sequence);
        let Some(age) = now.checked_duration_since(observed_at) else {
            state.fail(ResolverError::Stale);
            // A future timestamp must not prevent a later real scan recovering.
            return Err(ResolverError::Stale);
        };
        state.observed_at = Some(observed_at);
        if age > MAX_SOURCE_METADATA_AGE {
            state.fail(ResolverError::Stale);
            return Err(ResolverError::Stale);
        }
        let text = match title {
            Ok(text) if text.len() <= MAX_METADATA_BYTES && metadata(&text) => text,
            Ok(_) => {
                state.fail(ResolverError::InvalidData);
                return Err(ResolverError::InvalidData);
            }
            Err(error) => {
                state.fail(error);
                return Err(error);
            }
        };
        let Some(expires_at) = observed_at.checked_add(MAX_SOURCE_METADATA_AGE) else {
            state.fail(ResolverError::InvalidData);
            return Err(ResolverError::InvalidData);
        };
        state.error = None;
        // A new scan of an unchanged caption preserves active requests. Their
        // handed-out deadlines remain immutable; only subsequent leases renew.
        if let Some(latest) = state.latest.as_mut()
            && latest.text == text
            && now <= latest.expires_at
        {
            latest.observed_at = observed_at;
            latest.expires_at = expires_at;
            return Ok(());
        }
        state.clear();
        state.latest = Some(Title {
            text,
            observed_at,
            expires_at,
            cancellation: ResolverCancellation::default(),
        });
        Ok(())
    }

    pub(crate) fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stopped = true;
        state.clear();
    }

    /// A failed native worker cannot always provide a later scan sequence. Its
    /// owner retires the title without guessing ordering, while valid later scans
    /// can still recover. A stopped session is never restarted by failure.
    pub(crate) fn fail(&self, error: ResolverError) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.stopped {
            state.fail(error);
        }
    }

    fn metadata_at(
        &self,
        stamp: WindowStamp,
        now: Instant,
    ) -> Result<MovieHintMetadata, ResolverError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        stamp.validate(self.identity, now)?;
        let Some(latest) = state.latest.as_ref() else {
            return Err(state
                .error
                .unwrap_or(ResolverError::SourceMetadataUnavailable));
        };
        if now
            .checked_duration_since(latest.observed_at)
            .is_none_or(|age| age > MAX_SOURCE_METADATA_AGE)
        {
            state.fail(ResolverError::Stale);
            return Err(ResolverError::Stale);
        }
        Ok(MovieHintMetadata {
            stamp,
            window_title: Some(latest.text.clone()),
            filename: None,
            year: None,
            duration_us: None,
            validity: Some(MovieMetadataValidity {
                cancellation: latest.cancellation.clone(),
                expires_at: latest.expires_at,
            }),
        })
    }
}
impl MovieMetadataSource for SourceMetadataBuffer {
    fn metadata(&self, stamp: WindowStamp) -> Result<MovieHintMetadata, ResolverError> {
        self.metadata_at(stamp, Instant::now())
    }
}
impl Drop for SourceMetadataBuffer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "source_metadata_tests.rs"]
mod tests;
