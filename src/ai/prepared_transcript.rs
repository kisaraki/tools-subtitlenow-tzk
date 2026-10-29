//! Ordered, bounded merging of offline ASR finals. This is recognized source
//! text, never an existing subtitle publication or a playback alignment proof.
use super::{
    prepared_speech::{
        MAX_ASR_BATCH_CHUNKS, MAX_ASR_CONTEXT_BYTES, MAX_ASR_CONTEXT_ITEMS, MAX_ASR_DOCUMENT_BYTES,
        MAX_ASR_DOCUMENT_CUES, PreparedChunkStamp, PreparedSpeechError, PreparedSpeechRequest,
        PreparedSpeechResponse, map_clock_error, validate_text,
    },
    speech::{SpeechLanguage, StreamIdentity},
};
use crate::resolver::contracts::ResolverCancellation;
use crate::{
    domain::SourceLanguage,
    media::{
        decode_worker::MAX_DECODED_MEDIA_DURATION_US,
        decoded_clock::{DecodedClockSnapshot, NativeMediaInterval},
    },
};
use std::{collections::VecDeque, sync::Arc};

/// At most two distinct preparation scopes per bounded four-chunk batch. Real
/// producers reuse their session scope; reject arbitrary scope churn instead
/// of keeping an unbounded list or forgetting cancellation of committed text.
const MAX_PREPARED_CLOCK_SCOPES: usize = 2 * MAX_ASR_BATCH_CHUNKS;

/// Native timing has already been checked and converted to absolute media time.
/// Text remains session-only and has no Clone, Debug or serialization support.
pub struct PreparedTranscriptCue {
    pub start_us: u64,
    pub duration_us: u64,
    pub text: Box<str>,
}
pub struct PreparedTranscript {
    identity: StreamIdentity,
    language: SpeechLanguage,
    cues: Vec<PreparedTranscriptCue>,
}
impl PreparedTranscript {
    pub const fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub const fn language(&self) -> SpeechLanguage {
        self.language
    }
    pub fn cues(&self) -> &[PreparedTranscriptCue] {
        &self.cues
    }
    /// Structural ASR validation for borrowed document processing. The consumer
    /// must separately check this identity against its expected active Session.
    /// SpeechLanguage is a closed five-language enum; neither Auto nor an
    /// arbitrary provider language code can enter this transcript type.
    pub(crate) fn validate(&self) -> Result<(), PreparedSpeechError> {
        if self.identity.session.0 == 0
            || self.identity.generation == 0
            || self.cues.is_empty()
            || self.cues.len() > MAX_ASR_DOCUMENT_CUES
        {
            return Err(PreparedSpeechError::InvalidData);
        }
        let mut bytes = 0usize;
        let mut previous_end = None;
        for cue in &self.cues {
            validate_text(&cue.text)?;
            bytes = bytes
                .checked_add(cue.text.len())
                .filter(|bytes| *bytes <= MAX_ASR_DOCUMENT_BYTES)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let end = cue
                .start_us
                .checked_add(cue.duration_us)
                .filter(|end| cue.duration_us > 0 && *end <= MAX_DECODED_MEDIA_DURATION_US)
                .ok_or(PreparedSpeechError::InvalidData)?;
            if previous_end.is_some_and(|end| cue.start_us < end) {
                return Err(PreparedSpeechError::InvalidData);
            }
            previous_end = Some(end);
        }
        Ok(())
    }
}

struct Slot {
    stamp: PreparedChunkStamp,
    audio_end_us: u64,
    language: SourceLanguage,
    response: Option<PreparedSpeechResponse>,
    clock: Option<Arc<DecodedClockSnapshot>>,
    native_body: Option<NativeMediaInterval>,
}
/// At most four pending metadata/response slots; no request PCM is retained.
/// Any error clears all accumulated text and makes this instance terminal.
pub struct PreparedTranscriptMerger {
    identity: StreamIdentity,
    selected: SourceLanguage,
    last_registered: Option<PreparedChunkStamp>,
    last_registered_audio_end_us: Option<u64>,
    last_registered_clock: Option<Arc<DecodedClockSnapshot>>,
    last_registered_native_body: Option<NativeMediaInterval>,
    clock_scopes: Vec<ResolverCancellation>,
    pending: VecDeque<Slot>,
    cues: Vec<PreparedTranscriptCue>,
    bytes: usize,
    detected: Option<SpeechLanguage>,
    committed_run: Option<u64>,
    run_cue_start: usize,
    terminal: Option<PreparedSpeechError>,
}
impl PreparedTranscriptMerger {
    pub fn new(
        identity: StreamIdentity,
        language: SourceLanguage,
    ) -> Result<Self, PreparedSpeechError> {
        if identity.session.0 == 0 || identity.generation == 0 {
            return Err(PreparedSpeechError::InvalidData);
        }
        Ok(Self {
            identity,
            selected: language,
            last_registered: None,
            last_registered_audio_end_us: None,
            last_registered_clock: None,
            last_registered_native_body: None,
            clock_scopes: Vec::with_capacity(MAX_PREPARED_CLOCK_SCOPES),
            pending: VecDeque::with_capacity(MAX_ASR_BATCH_CHUNKS),
            cues: Vec::new(),
            bytes: 0,
            detected: None,
            committed_run: None,
            run_cue_start: 0,
            terminal: None,
        })
    }
    pub fn register(&mut self, request: &PreparedSpeechRequest) -> Result<(), PreparedSpeechError> {
        let result = self.register_inner(request);
        self.close_on_error(result)
    }
    fn register_inner(
        &mut self,
        request: &PreparedSpeechRequest,
    ) -> Result<(), PreparedSpeechError> {
        self.check_active()?;
        let stamp = request.chunk().stamp();
        if stamp.identity != self.identity || request.language() != self.selected {
            return Err(PreparedSpeechError::Stale);
        }
        if self.pending.len() >= MAX_ASR_BATCH_CHUNKS {
            return Err(PreparedSpeechError::Overloaded);
        }
        let clock = request.chunk().clock_owner().cloned();
        if let Some(clock) = &clock {
            for scope in clock.cancellation_scopes() {
                if scope.is_cancelled() {
                    return Err(PreparedSpeechError::Cancelled);
                }
                if !self.clock_scopes.iter().any(|old| old.same_scope(scope)) {
                    if self.clock_scopes.len() == MAX_PREPARED_CLOCK_SCOPES {
                        return Err(PreparedSpeechError::Overloaded);
                    }
                    self.clock_scopes.push(scope.clone());
                }
            }
        }
        let native_body = clock
            .as_ref()
            .map(|map| {
                map.map_interval(stamp.identity, stamp.body_start_us, stamp.body_end_us)
                    .map_err(map_clock_error)
            })
            .transpose()?;
        if let Some(previous) = self.last_registered {
            match (&self.last_registered_clock, &clock) {
                (Some(old), Some(new)) => {
                    new.validate_successor(old).map_err(map_clock_error)?;
                    let old_body = self
                        .last_registered_native_body
                        .ok_or(PreparedSpeechError::InvalidData)?;
                    let body = native_body.ok_or(PreparedSpeechError::InvalidData)?;
                    if stamp.run_id == previous.run_id {
                        if new.source_run_id() != old.source_run_id()
                            || body.start_us().abs_diff(old_body.end_us()) > 1
                        {
                            return Err(PreparedSpeechError::BoundaryMismatch);
                        }
                    } else if new.source_run_id() <= old.source_run_id()
                        || old_body
                            .end_us()
                            .checked_add(1)
                            .is_none_or(|end| body.start_us() <= end)
                    {
                        return Err(PreparedSpeechError::BoundaryMismatch);
                    }
                }
                (None, None) => {}
                _ => return Err(PreparedSpeechError::InvalidData),
            }
            if previous.sequence.checked_add(1) != Some(stamp.sequence) {
                return Err(PreparedSpeechError::InvalidData);
            }
            if stamp.run_id == previous.run_id {
                if stamp.discontinuity || stamp.body_start_us.abs_diff(previous.body_end_us) > 1 {
                    return Err(PreparedSpeechError::InvalidData);
                }
            } else {
                // Right context is real audio still awaiting its owned body.
                // A run switch must not silently discard that pending speech.
                if self.last_registered_audio_end_us != Some(previous.body_end_us) {
                    return Err(PreparedSpeechError::BoundaryMismatch);
                }
                if previous.run_id.checked_add(1) != Some(stamp.run_id)
                    || !stamp.discontinuity
                    || stamp.audio_start_us != stamp.body_start_us
                    || previous
                        .body_end_us
                        .checked_add(1)
                        .is_none_or(|previous_end| stamp.audio_start_us <= previous_end)
                {
                    return Err(PreparedSpeechError::InvalidData);
                }
            }
        } else if stamp.sequence != 1
            || stamp.run_id != 1
            || stamp.discontinuity
            || stamp.audio_start_us != stamp.body_start_us
        {
            return Err(PreparedSpeechError::InvalidData);
        }
        self.pending.push_back(Slot {
            stamp,
            audio_end_us: request.chunk().audio_end_us(),
            language: request.language(),
            response: None,
            clock: clock.clone(),
            native_body,
        });
        self.last_registered = Some(stamp);
        self.last_registered_audio_end_us = Some(request.chunk().audio_end_us());
        self.last_registered_clock = clock;
        self.last_registered_native_body = native_body;
        Ok(())
    }
    pub fn complete(
        &mut self,
        request: &PreparedSpeechRequest,
        response: PreparedSpeechResponse,
    ) -> Result<(), PreparedSpeechError> {
        let result = self.complete_inner(request, response);
        self.close_on_error(result)
    }
    fn complete_inner(
        &mut self,
        request: &PreparedSpeechRequest,
        mut response: PreparedSpeechResponse,
    ) -> Result<(), PreparedSpeechError> {
        self.check_active()?;
        let stamp = request.chunk().stamp();
        let slot = self
            .pending
            .iter_mut()
            .find(|slot| slot.stamp.sequence == stamp.sequence)
            .ok_or(PreparedSpeechError::Stale)?;
        if slot.stamp != stamp
            || slot.language != request.language()
            || slot.audio_end_us != request.chunk().audio_end_us()
            || slot.response.is_some()
            || !same_clock_owner(slot.clock.as_ref(), request.chunk().clock_owner())
        {
            return Err(PreparedSpeechError::Stale);
        }
        response.validate_for(request)?;
        if let Some(clock) = &slot.clock {
            clock
                .map_interval(stamp.identity, stamp.body_start_us, stamp.body_end_us)
                .map_err(map_clock_error)?;
        }
        // A provider may return a short Vec with arbitrarily large capacity.
        // Boxing discards spare allocation; converting back preserves the
        // public response API with exactly len() usable element capacity.
        response.cues = response.cues.into_boxed_slice().into_vec();
        slot.response = Some(response);
        while self
            .pending
            .front()
            .is_some_and(|slot| slot.response.is_some())
        {
            let mut slot = self
                .pending
                .pop_front()
                .ok_or(PreparedSpeechError::InvalidData)?;
            let response = slot
                .response
                .take()
                .ok_or(PreparedSpeechError::InvalidData)?;
            self.commit(slot, response)?;
        }
        Ok(())
    }
    fn commit(
        &mut self,
        slot: Slot,
        response: PreparedSpeechResponse,
    ) -> Result<(), PreparedSpeechError> {
        let stamp = slot.stamp;
        // Even a genuine empty/silent response must not commit cancelled source
        // evidence after waiting behind an out-of-order earlier response.
        if let Some(clock) = &slot.clock {
            clock
                .map_interval(stamp.identity, stamp.body_start_us, stamp.body_end_us)
                .map_err(map_clock_error)?;
        }
        if self.committed_run != Some(stamp.run_id) {
            self.committed_run = Some(stamp.run_id);
            self.run_cue_start = self.cues.len();
        }
        if !response.cues.is_empty() {
            let language = response.language.ok_or(PreparedSpeechError::InvalidData)?;
            if self.detected.is_some_and(|detected| detected != language) {
                return Err(PreparedSpeechError::MixedLanguages);
            }
            self.detected = Some(language);
        }
        for cue in response.cues {
            self.cues
                .len()
                .checked_add(1)
                .filter(|count| *count <= MAX_ASR_DOCUMENT_CUES)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let next_bytes = self
                .bytes
                .checked_add(cue.text.len())
                .filter(|bytes| *bytes <= MAX_ASR_DOCUMENT_BYTES)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let sample_start_us = stamp
                .audio_start_us
                .checked_add(cue.start_us)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let (start_us, duration_us) = if let Some(clock) = &slot.clock {
                let sample_end_us = sample_start_us
                    .checked_add(cue.duration_us)
                    .ok_or(PreparedSpeechError::InvalidData)?;
                let native = clock
                    .map_interval(stamp.identity, sample_start_us, sample_end_us)
                    .map_err(map_clock_error)?;
                let body = slot.native_body.ok_or(PreparedSpeechError::InvalidData)?;
                if native.start_us() < body.start_us() || native.end_us() > body.end_us() {
                    return Err(PreparedSpeechError::BoundaryMismatch);
                }
                (
                    native.start_us(),
                    native
                        .end_us()
                        .checked_sub(native.start_us())
                        .filter(|duration| *duration > 0)
                        .ok_or(PreparedSpeechError::InvalidData)?,
                )
            } else {
                (sample_start_us, cue.duration_us)
            };
            if self.cues.last().is_some_and(|previous| {
                previous
                    .start_us
                    .checked_add(previous.duration_us)
                    .is_none_or(|previous_end| start_us < previous_end)
            }) {
                return Err(PreparedSpeechError::InvalidData);
            }
            self.cues.push(PreparedTranscriptCue {
                start_us,
                duration_us,
                text: cue.text,
            });
            self.bytes = next_bytes;
        }
        Ok(())
    }
    /// Context contains only already committed text from the requested run.
    /// Out-of-order pending responses can never become context prematurely.
    pub fn context(&self, run_id: u64) -> Vec<Box<str>> {
        if self.check_active().is_err() || self.committed_run != Some(run_id) {
            return Vec::new();
        }
        let start = self
            .cues
            .len()
            .saturating_sub(MAX_ASR_CONTEXT_ITEMS)
            .max(self.run_cue_start);
        let mut bytes = 0usize;
        let mut texts = Vec::with_capacity(MAX_ASR_CONTEXT_ITEMS);
        for cue in self.cues[start..].iter().rev() {
            let Some(next_bytes) = bytes.checked_add(cue.text.len()) else {
                break;
            };
            if next_bytes > MAX_ASR_CONTEXT_BYTES {
                break;
            }
            bytes = next_bytes;
            texts.push(cue.text.clone());
        }
        texts.reverse();
        texts
    }
    pub fn finish(self) -> Result<PreparedTranscript, PreparedSpeechError> {
        self.check_active()?;
        if !self.pending.is_empty() {
            return Err(PreparedSpeechError::InvalidData);
        }
        if self
            .last_registered
            .is_some_and(|stamp| self.last_registered_audio_end_us != Some(stamp.body_end_us))
        {
            return Err(PreparedSpeechError::BoundaryMismatch);
        }
        if let (Some(clock), Some(stamp)) = (&self.last_registered_clock, self.last_registered) {
            clock
                .map_interval(stamp.identity, stamp.body_start_us, stamp.body_end_us)
                .map_err(map_clock_error)?;
        }
        if self.cues.is_empty() {
            return Err(PreparedSpeechError::NoSpeech);
        }
        let transcript = PreparedTranscript {
            identity: self.identity,
            language: self.detected.ok_or(PreparedSpeechError::InvalidData)?,
            cues: self.cues,
        };
        transcript.validate()?;
        // Validation may inspect the bounded whole document. An earlier source
        // scope cancelled during it must also win over final successful handoff.
        if self
            .clock_scopes
            .iter()
            .any(ResolverCancellation::is_cancelled)
        {
            return Err(PreparedSpeechError::Cancelled);
        }
        Ok(transcript)
    }
    fn check_active(&self) -> Result<(), PreparedSpeechError> {
        self.terminal.map_or(Ok(()), Err)?;
        if self
            .clock_scopes
            .iter()
            .any(ResolverCancellation::is_cancelled)
        {
            Err(PreparedSpeechError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn close_on_error(
        &mut self,
        result: Result<(), PreparedSpeechError>,
    ) -> Result<(), PreparedSpeechError> {
        if let Err(error) = result {
            self.terminal = Some(error);
            self.pending.clear();
            self.cues.clear();
            self.bytes = 0;
            self.detected = None;
            self.committed_run = None;
            self.run_cue_start = 0;
            self.last_registered_clock = None;
            self.last_registered_native_body = None;
            self.clock_scopes.clear();
        }
        result
    }
}

fn same_clock_owner(
    left: Option<&Arc<DecodedClockSnapshot>>,
    right: Option<&Arc<DecodedClockSnapshot>>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

#[cfg(test)]
#[path = "prepared_transcript_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "prepared_transcript_validation_tests.rs"]
mod validation_tests;

#[cfg(test)]
#[path = "prepared_clock_tests.rs"]
mod clock_tests;
