//! One current-session Final ASR hint. Neither audio nor transcript history is
//! retained, and a query never invents timing to fit its actual capture window.
use super::{
    contracts::{MAX_WINDOW_AGE, ResolverCancellation, ResolverError, WindowStamp},
    movie::MAX_DIALOGUE_BYTES,
};
use crate::ai::speech::{
    MAX_TRANSCRIPT_BYTES, SpeechLanguage, StreamIdentity, TranscriptEvent, TranscriptKind,
};
use std::{sync::Mutex, time::Instant};

/// A bounded copy of the current-window dialogue, with its publication token.
/// Replacing, clearing, stopping, or dropping the owner cancels this token.
pub struct MovieDialogueSnapshot {
    pub dialogue: Box<str>,
    pub language: Option<SpeechLanguage>,
    pub cancellation: ResolverCancellation,
    /// Original receipt deadline; selecting a newer window never refreshes it.
    pub expires_at: Instant,
}

/// Short metadata-only critical sections. Audio capture does not call this
/// buffer; there is no I/O, asynchronous work, or callback under its lock.
pub struct MovieHintBuffer {
    identity: StreamIdentity,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    stopped: bool,
    watermark: Option<Watermark>,
    latest: Option<Dialogue>,
}
struct Watermark {
    sequence: u64,
    utterance: u64,
    revision: u64,
    start_us: u64,
    end_us: u64,
}
struct Dialogue {
    text: Box<str>,
    language: Option<SpeechLanguage>,
    start_us: u64,
    end_us: u64,
    received: Instant,
    cancellation: ResolverCancellation,
}
impl State {
    fn clear(&mut self) {
        if let Some(previous) = self.latest.take() {
            previous.cancellation.cancel();
        }
    }
}
impl MovieHintBuffer {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            state: Mutex::new(State::default()),
        }
    }

    /// Keep only valid timed Final events. Missing native timing and Partial
    /// text cannot replace an already usable hint or advance its watermark.
    pub fn observe(&self, event: &TranscriptEvent) -> Result<bool, ResolverError> {
        self.observe_at(event, Instant::now())
    }

    fn observe_at(
        &self,
        event: &TranscriptEvent,
        received: Instant,
    ) -> Result<bool, ResolverError> {
        if event.identity != self.identity
            || event.text.len() > MAX_TRANSCRIPT_BYTES
            || event.duration_us == Some(0)
            || event
                .start_us
                .zip(event.duration_us)
                .is_some_and(|(start, duration)| start.checked_add(duration).is_none())
        {
            return Err(ResolverError::InvalidData);
        }
        // State is limited to one bounded string and scalar metadata; no user
        // code can run under this lock. Recovering poison still permits stop.
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        if event.kind == TranscriptKind::Partial {
            return Ok(false);
        }
        if event.text.trim().is_empty()
            || event.text.len() > MAX_DIALOGUE_BYTES
            || event
                .text
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(ResolverError::InvalidData);
        }
        let Some((start_us, duration_us)) = event.start_us.zip(event.duration_us) else {
            return Ok(false);
        };
        let end_us = start_us
            .checked_add(duration_us)
            .ok_or(ResolverError::InvalidData)?;
        if state.watermark.as_ref().is_some_and(|previous| {
            event.sequence <= previous.sequence
                || (event.utterance == previous.utterance && event.revision <= previous.revision)
                || start_us < previous.start_us
                || end_us < previous.end_us
        }) {
            return Err(ResolverError::Stale);
        }
        // Allocate only after validation and ordering. There is exactly one
        // owner string, and at most one bounded copy per returned snapshot.
        let latest = Dialogue {
            text: event.text.clone(),
            language: event.language,
            start_us,
            end_us,
            received,
            cancellation: ResolverCancellation::default(),
        };
        state.clear();
        state.watermark = Some(Watermark {
            sequence: event.sequence,
            utterance: event.utterance,
            revision: event.revision,
            start_us,
            end_us,
        });
        state.latest = Some(latest);
        Ok(true)
    }

    /// Remove private text and retire all copies of its publication token.
    /// Preserve sequence and native-time ordering metadata so old Final events
    /// cannot re-enter a gap, even with a greater provider arrival sequence.
    pub fn clear(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    /// Permanently retire this session owner.
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stopped = true;
        state.clear();
    }

    /// Return dialogue only if its entire positive native interval is inside a
    /// fresh actual capture window. Unknown capture timing remains unknown.
    pub fn query(
        &self,
        stamp: WindowStamp,
    ) -> Result<Option<MovieDialogueSnapshot>, ResolverError> {
        self.query_at(stamp, Instant::now())
    }

    fn query_at(
        &self,
        stamp: WindowStamp,
        now: Instant,
    ) -> Result<Option<MovieDialogueSnapshot>, ResolverError> {
        stamp.validate(self.identity, now)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return Err(ResolverError::Cancelled);
        }
        let Some(latest) = state.latest.as_ref() else {
            return Ok(None);
        };
        if now
            .checked_duration_since(latest.received)
            .is_none_or(|age| age > MAX_WINDOW_AGE)
        {
            state.clear();
            return Err(ResolverError::Stale);
        }
        let Some(start_us) = stamp.start_us else {
            return Ok(None);
        };
        let end_us = start_us
            .checked_add(stamp.duration_us)
            .ok_or(ResolverError::InvalidData)?;
        if latest.start_us < start_us || latest.end_us > end_us {
            return Ok(None);
        }
        Ok(Some(MovieDialogueSnapshot {
            dialogue: latest.text.clone(),
            language: latest.language,
            cancellation: latest.cancellation.clone(),
            expires_at: latest
                .received
                .checked_add(MAX_WINDOW_AGE)
                .ok_or(ResolverError::InvalidData)?,
        }))
    }
}
impl Drop for MovieHintBuffer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "movie_hints_tests.rs"]
mod tests;
