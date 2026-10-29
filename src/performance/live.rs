//! Session-only numeric correlation. Called downstream of capture, never by its callback.
use super::{DurationDistribution, DurationHistogram};
use crate::{
    ai::{
        speech::{StreamIdentity, TranscriptEvent, TranscriptKind},
        translation::{TranslationPass, TranslationUpdate},
    },
    audio::preprocess::NormalizedAudio,
    subtitle::timeline::{TimelineBinding, TimelineItem},
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
pub const AUDIO_ANCHORS: usize = 2048;
pub const CORRELATIONS: usize = 64;
const OUTPUT_MARKS: usize = 128;
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Latency {
    SpeechFirstPartial,
    SpeechFinal,
    AudioToSpeechConsumer,
    SpeechOutputQueue,
    TranslationFirstOutput,
    TranslationRequestFinal,
    TranslationApply,
    TranslationToRenderSubmission,
    CaptureToFirstUsefulSubmission,
}
impl Latency {
    pub const ALL: [Self; 9] = [
        Self::SpeechFirstPartial,
        Self::SpeechFinal,
        Self::AudioToSpeechConsumer,
        Self::SpeechOutputQueue,
        Self::TranslationFirstOutput,
        Self::TranslationRequestFinal,
        Self::TranslationApply,
        Self::TranslationToRenderSubmission,
        Self::CaptureToFirstUsefulSubmission,
    ];
}
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Activity {
    SpeechOperation,
    TranslationOperation,
    SpeechAudioQueue,
    SpeechEventQueue,
    TranslationUpdates,
    TranslationPending,
    TranslationPermits,
    ResolverWorkers,
}
impl Activity {
    pub const ALL: [Self; 8] = [
        Self::SpeechOperation,
        Self::TranslationOperation,
        Self::SpeechAudioQueue,
        Self::SpeechEventQueue,
        Self::TranslationUpdates,
        Self::TranslationPending,
        Self::TranslationPermits,
        Self::ResolverWorkers,
    ];
}
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Count {
    AudioRejected,
    AudioExpired,
    SpeechEventsRejected,
    TranslationSuperseded,
    TranslationExpired,
    TranslationUpdatesRejected,
    CorrelationMissing,
    CorrelationEvicted,
    DiagnosticLockFailure,
    Reconnects,
    UiReceiptRetired,
    FinalTranscriptsRejected,
    FinalSegmentsExpired,
    FinalSegmentsRetiredBeforeCompletion,
    FinalSegmentsUnfinishedAtStop,
}
impl Count {
    pub const ALL: [Self; 15] = [
        Self::AudioRejected,
        Self::AudioExpired,
        Self::SpeechEventsRejected,
        Self::TranslationSuperseded,
        Self::TranslationExpired,
        Self::TranslationUpdatesRejected,
        Self::CorrelationMissing,
        Self::CorrelationEvicted,
        Self::DiagnosticLockFailure,
        Self::Reconnects,
        Self::UiReceiptRetired,
        Self::FinalTranscriptsRejected,
        Self::FinalSegmentsExpired,
        Self::FinalSegmentsRetiredBeforeCompletion,
        Self::FinalSegmentsUnfinishedAtStop,
    ];
}
#[derive(Default)]
struct Gauge {
    current: AtomicU64,
    peak: AtomicU64,
}
#[derive(Clone, Copy, Debug)]
pub struct GaugeSnapshot {
    pub current: u64,
    pub observed_high_water: u64,
}
#[derive(Clone, Copy)]
struct Anchor {
    start: u64,
    end: u64,
    at: Instant,
}
struct Entry {
    binding: TimelineBinding,
    origin: Option<Instant>,
    partial: bool,
    final_seen: bool,
    useful: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct OutputKey {
    binding: TimelineBinding,
    job: u64,
    pass: TranslationPass,
    revision: u64,
}
pub(crate) struct TranslationObservation {
    pub job: u64,
    pub pass: TranslationPass,
    pub revision: u64,
    pub started: Instant,
    pub first: bool,
    pub complete: bool,
}
#[derive(Clone, Copy)]
struct Output {
    key: OutputKey,
    ready: Instant,
    applied: bool,
    submitted: bool,
}
#[derive(Default)]
struct Correlation {
    anchors: VecDeque<Anchor>,
    entries: VecDeque<Entry>,
    outputs: VecDeque<Output>,
    speech_marks: VecDeque<(TimelineBinding, Instant)>,
}
pub struct LiveDiagnostics {
    identity: StreamIdentity,
    retired: AtomicBool,
    durations: [DurationHistogram; 9],
    gauges: [Gauge; 8],
    counts: [AtomicU64; 15],
    correlation: Mutex<Correlation>,
}
impl LiveDiagnostics {
    pub fn scope(self: &Arc<Self>) -> DiagnosticScope {
        DiagnosticScope(self.clone())
    }
    pub fn new(identity: StreamIdentity) -> Arc<Self> {
        Arc::new(Self {
            identity,
            retired: AtomicBool::new(false),
            durations: std::array::from_fn(|_| DurationHistogram::default()),
            gauges: std::array::from_fn(|_| Gauge::default()),
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            correlation: Mutex::new(Correlation {
                anchors: VecDeque::with_capacity(AUDIO_ANCHORS),
                entries: VecDeque::with_capacity(CORRELATIONS),
                outputs: VecDeque::with_capacity(OUTPUT_MARKS),
                speech_marks: VecDeque::with_capacity(OUTPUT_MARKS),
            }),
        })
    }
    pub fn identity(&self) -> StreamIdentity {
        self.identity
    }
    pub fn distribution(&self, key: Latency) -> DurationDistribution {
        self.durations[key as usize].snapshot()
    }
    pub fn count(&self, key: Count) -> u64 {
        self.counts[key as usize].load(Ordering::Relaxed)
    }
    pub fn increment(&self, key: Count) {
        let _ = self.counts[key as usize].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            Some(v.saturating_add(1))
        });
    }
    pub fn observe(&self, key: Activity, value: usize) {
        let gauge = &self.gauges[key as usize];
        let value = u64::try_from(value).unwrap_or(u64::MAX);
        gauge.current.store(value, Ordering::Relaxed);
        gauge.peak.fetch_max(value, Ordering::Relaxed);
    }
    pub fn gauge(&self, key: Activity) -> GaugeSnapshot {
        let gauge = &self.gauges[key as usize];
        GaugeSnapshot {
            current: gauge.current.load(Ordering::Relaxed),
            observed_high_water: gauge.peak.load(Ordering::Relaxed),
        }
    }
    pub fn operation(self: &Arc<Self>, key: Activity) -> Operation {
        let gauge = &self.gauges[key as usize];
        let current = gauge.current.fetch_add(1, Ordering::Relaxed) + 1;
        gauge.peak.fetch_max(current, Ordering::Relaxed);
        Operation {
            owner: self.clone(),
            key,
        }
    }
    fn lock(&self) -> Option<std::sync::MutexGuard<'_, Correlation>> {
        self.correlation
            .lock()
            .map_err(|_| self.increment(Count::DiagnosticLockFailure))
            .ok()
    }
    fn record(&self, key: Latency, at: Instant, now: Instant) {
        if let Some(duration) = now.checked_duration_since(at) {
            self.durations[key as usize].record(duration);
        } else {
            self.increment(Count::CorrelationMissing);
        }
    }
    pub fn observe_audio(&self, audio: &NormalizedAudio) {
        if self.retired.load(Ordering::Acquire) || audio.stamp.session != self.identity.session {
            return;
        }
        let Some(start) = audio.start_us() else {
            return;
        };
        let Some(end) = start.checked_add(audio.duration_us()) else {
            return;
        };
        if let Some(mut state) = self.lock() {
            if audio.stamp.discontinuity {
                state.anchors.clear();
            }
            if state.anchors.len() == AUDIO_ANCHORS {
                state.anchors.pop_front();
            }
            state.anchors.push_back(Anchor {
                start,
                end,
                at: audio.stamp.received,
            });
        }
    }
    pub(crate) fn speech_emitted(&self, event: &TranscriptEvent) {
        if event.identity != self.identity || self.retired.load(Ordering::Acquire) {
            return;
        }
        let now = Instant::now();
        let binding = TimelineBinding::from_event(event);
        if let Some(mut state) = self.lock() {
            if state.speech_marks.len() == OUTPUT_MARKS {
                state.speech_marks.pop_front();
                self.increment(Count::CorrelationEvicted);
            }
            state.speech_marks.push_back((binding, now));
            let index = state
                .entries
                .iter()
                .position(|entry| entry.binding.utterance == binding.utterance);
            let index = if let Some(index) = index {
                if state.entries[index].binding.sequence != binding.sequence
                    || state.entries[index].binding.revision >= binding.revision
                    || state.entries[index].final_seen
                {
                    return;
                }
                index
            } else {
                let origin = event.start_us.and_then(|start| {
                    state
                        .anchors
                        .iter()
                        .rev()
                        .find(|anchor| anchor.start <= start && start < anchor.end)
                        .and_then(|anchor| {
                            anchor
                                .at
                                .checked_add(std::time::Duration::from_micros(start - anchor.start))
                        })
                });
                if state.entries.len() == CORRELATIONS {
                    state.entries.pop_front();
                    self.increment(Count::CorrelationEvicted);
                }
                state.entries.push_back(Entry {
                    binding,
                    origin,
                    partial: false,
                    final_seen: false,
                    useful: false,
                });
                state.entries.len() - 1
            };
            let entry = &mut state.entries[index];
            entry.binding = binding;
            let key = match event.kind {
                TranscriptKind::Partial if !entry.partial => {
                    entry.partial = true;
                    Some(Latency::SpeechFirstPartial)
                }
                TranscriptKind::Final if !entry.final_seen => {
                    entry.final_seen = true;
                    Some(Latency::SpeechFinal)
                }
                _ => None,
            };
            if let Some(key) = key {
                if let Some(origin) = entry.origin {
                    self.record(key, origin, now);
                } else {
                    self.increment(Count::CorrelationMissing);
                }
            }
        }
    }
    pub(crate) fn speech_consumed(&self, event: &TranscriptEvent) {
        if event.identity != self.identity {
            return;
        }
        if let Some(mut state) = self.lock()
            && let Some(index) = state
                .speech_marks
                .iter()
                .position(|(binding, _)| *binding == TimelineBinding::from_event(event))
            && let Some((_, emitted)) = state.speech_marks.remove(index)
        {
            self.record(Latency::SpeechOutputQueue, emitted, Instant::now());
        }
    }
    pub(crate) fn audio_consumed(&self, audio: &NormalizedAudio) {
        if audio.stamp.session == self.identity.session {
            self.record(
                Latency::AudioToSpeechConsumer,
                audio.stamp.received,
                Instant::now(),
            );
        }
    }
    pub(crate) fn translation_emitted(
        &self,
        event: &TranscriptEvent,
        observation: TranslationObservation,
    ) {
        let TranslationObservation {
            job,
            pass,
            revision,
            started,
            first,
            complete,
        } = observation;
        if event.identity != self.identity || self.retired.load(Ordering::Acquire) {
            return;
        }
        let now = Instant::now();
        if first {
            self.record(Latency::TranslationFirstOutput, started, now);
        }
        if complete {
            self.record(Latency::TranslationRequestFinal, started, now);
        }
        if let Some(mut state) = self.lock() {
            if state.outputs.len() == OUTPUT_MARKS {
                state.outputs.pop_front();
                self.increment(Count::CorrelationEvicted);
            }
            state.outputs.push_back(Output {
                key: OutputKey {
                    binding: TimelineBinding::from_event(event),
                    job,
                    pass,
                    revision,
                },
                ready: now,
                applied: false,
                submitted: false,
            });
        }
    }
    pub(crate) fn translation_applied(&self, update: &TranslationUpdate, source: &TranscriptEvent) {
        if update.identity != self.identity {
            return;
        }
        let key = OutputKey {
            binding: TimelineBinding::from_event(source),
            job: update.job_id,
            pass: update.pass,
            revision: update.revision,
        };
        if let Some(mut state) = self.lock() {
            if let Some(output) = state.outputs.iter_mut().find(|o| o.key == key) {
                self.record(Latency::TranslationApply, output.ready, Instant::now());
                output.applied = true;
            } else {
                self.increment(Count::CorrelationMissing);
            }
        }
    }
    pub(crate) fn ticket(self: &Arc<Self>, item: &TimelineItem) -> Option<Arc<RenderTicket>> {
        if item.source.identity != self.identity
            || item.translated.is_empty()
            || item.existing_display().is_some()
            || self.retired.load(Ordering::Acquire)
        {
            return None;
        }
        let (revision, job, pass, output_revision) = item.translation_order?;
        if revision != item.source.revision {
            return None;
        }
        let key = OutputKey {
            binding: TimelineBinding::from_event(&item.source),
            job,
            pass,
            revision: output_revision,
        };
        let state = self.lock()?;
        let output = state
            .outputs
            .iter()
            .find(|o| o.key == key && o.applied && !o.submitted)?;
        Some(Arc::new(RenderTicket {
            owner: self.clone(),
            key,
            ready: output.ready,
            used: AtomicBool::new(false),
        }))
    }
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        if let Some(mut state) = self.lock() {
            state.anchors.clear();
            state.entries.clear();
            state.outputs.clear();
            state.speech_marks.clear();
        }
    }
}
pub struct Operation {
    owner: Arc<LiveDiagnostics>,
    key: Activity,
}
pub struct DiagnosticScope(Arc<LiveDiagnostics>);
impl Drop for DiagnosticScope {
    fn drop(&mut self) {
        self.0.retire();
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        let _ = self.owner.gauges[self.key as usize].current.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |v| v.checked_sub(1),
        );
    }
}
pub(crate) struct RenderTicket {
    owner: Arc<LiveDiagnostics>,
    key: OutputKey,
    ready: Instant,
    used: AtomicBool,
}
impl RenderTicket {
    pub(crate) fn pending(&self) -> bool {
        !self.owner.retired.load(Ordering::Acquire) && !self.used.load(Ordering::Acquire)
    }
    pub(crate) fn submitted(&self) {
        if self.owner.retired.load(Ordering::Acquire) || self.used.swap(true, Ordering::AcqRel) {
            return;
        }
        let now = Instant::now();
        if let Some(mut state) = self.owner.lock() {
            let Some(index) = state
                .outputs
                .iter()
                .position(|output| output.key == self.key && output.applied && !output.submitted)
            else {
                return;
            };
            if !state
                .entries
                .iter()
                .any(|entry| entry.binding == self.key.binding)
            {
                self.owner.increment(Count::UiReceiptRetired);
                return;
            }
            state.outputs[index].submitted = true;
            self.owner
                .record(Latency::TranslationToRenderSubmission, self.ready, now);
            if let Some(entry) = state
                .entries
                .iter_mut()
                .find(|e| e.binding == self.key.binding)
                && !entry.useful
            {
                entry.useful = true;
                if let Some(origin) = entry.origin {
                    self.owner
                        .record(Latency::CaptureToFirstUsefulSubmission, origin, now);
                } else {
                    self.owner.increment(Count::CorrelationMissing);
                }
            }
        }
    }
}
#[cfg(test)]
#[path = "live_tests.rs"]
pub(crate) mod tests;
