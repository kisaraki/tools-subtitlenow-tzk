//! Latest-only proposal orchestration; no playback projection or source adoption.
use crate::{
    ai::speech::{MAX_TRANSCRIPT_BYTES, TranscriptEvent, TranscriptKind},
    resolver::contracts::{MAX_WINDOW_AGE, ResolverCancellation, ResolverError, WindowStamp},
    resolver::text_session::WeakExistingTextLease,
    subtitle::{
        fuzzy_alignment::{
            AlignmentScope, FUZZY_MATCH_TIMEOUT, FuzzyAlignmentError, FuzzyCueProposal,
        },
        fuzzy_track::{FuzzyTrack, FuzzyTrackProposal, FuzzyTrackStatus},
        fuzzy_worker::FuzzyCueLocator,
        timeline::TimelineBinding,
    },
};
use std::{sync::Arc, time::Instant};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalState {
    Waiting,
    Locating,
    Proposed,
    Unmatched,
    Failed(FuzzyAlignmentError),
    Cancelled,
    Stopped,
}
struct Request {
    ticket: u64,
    event: TranscriptEvent,
    stamp: WindowStamp,
    deadline: Instant,
    cancel: ResolverCancellation,
}
struct Completion {
    ticket: u64,
    scope: AlignmentScope,
    binding: TimelineBinding,
    result: Result<FuzzyCueProposal, FuzzyAlignmentError>,
}

/// One current request and one latest replacement in the driver, never a FIFO.
/// Each ASR snapshot is at most 8192 bytes, RAM-only, and never Debug/Serialize.
/// Caller owns/polls the returned task and stops this session on source/epoch
/// changes. Construct a new locator/session for a new publication or seek epoch.
pub struct FuzzyProposalSession {
    scope: AlignmentScope,
    input: watch::Sender<Option<Arc<Request>>>,
    output: watch::Receiver<Option<Arc<Completion>>>,
    reset_output: watch::Sender<Option<Arc<Completion>>>,
    cancel: ResolverCancellation,
    availability: ResolverCancellation,
    active: Option<Arc<Request>>,
    accepted: Option<Arc<Completion>>,
    ticket: u64,
    seen: u64,
    status: ProposalState,
    track: FuzzyTrack,
    track_notice: FuzzyTrackStatus,
    publication: Option<WeakExistingTextLease>,
    source_deadline: Option<Instant>,
}
pub struct FuzzyProposalTask {
    locator: FuzzyCueLocator,
    input: watch::Receiver<Option<Arc<Request>>>,
    output: watch::Sender<Option<Arc<Completion>>>,
    cancel: ResolverCancellation,
    availability: ResolverCancellation,
    source_deadline: Option<Instant>,
}
impl FuzzyProposalSession {
    pub(crate) fn new_prepared_history(
        locator: FuzzyCueLocator,
    ) -> Result<(Self, FuzzyProposalTask), FuzzyAlignmentError> {
        if !locator.is_prepared() {
            return Err(FuzzyAlignmentError::UnsupportedEvidence);
        }
        let (mut owner, task) = Self::new(locator)?;
        owner.track.enable_prepared_history();
        Ok((owner, task))
    }
    pub(crate) fn await_new_input(&mut self) {
        self.retire(ProposalState::Waiting, true);
    }
    pub fn new(locator: FuzzyCueLocator) -> Result<(Self, FuzzyProposalTask), FuzzyAlignmentError> {
        let scope = locator.scope();
        let availability = locator.availability()?;
        let publication = locator.publication();
        let source_deadline = locator.source_deadline();
        let cancel = ResolverCancellation::default();
        let (input, receiver) = watch::channel(None);
        let (output, updates) = watch::channel(None);
        Ok((
            Self {
                scope,
                input,
                output: updates,
                reset_output: output.clone(),
                cancel: cancel.clone(),
                availability: availability.clone(),
                active: None,
                accepted: None,
                ticket: 0,
                seen: 0,
                status: ProposalState::Waiting,
                track: FuzzyTrack::new(scope, availability.clone())
                    .with_publication(publication.clone()),
                track_notice: FuzzyTrackStatus::Collecting(0),
                publication,
                source_deadline,
            },
            FuzzyProposalTask {
                locator,
                input: receiver,
                output,
                cancel,
                availability,
                source_deadline,
            },
        ))
    }
    pub fn scope(&self) -> AlignmentScope {
        self.scope
    }
    pub fn binding(&self) -> Option<TimelineBinding> {
        self.active
            .as_ref()
            .map(|request| TimelineBinding::from_event(&request.event))
    }
    pub fn track_status(&self) -> FuzzyTrackStatus {
        self.track.status(Instant::now())
    }
    pub fn track_proposal(&self) -> Option<FuzzyTrackProposal> {
        self.track.proposal(Instant::now())
    }
    /// Same binding/payload is idempotent even with a newer window: duplicates
    /// never refresh deadlines or count as independent observations. A changed
    /// payload under unchanged revision withdraws that request as invalid.
    pub fn offer(
        &mut self,
        event: TranscriptEvent,
        stamp: WindowStamp,
    ) -> Result<bool, FuzzyAlignmentError> {
        if self.status == ProposalState::Stopped {
            return Err(ResolverError::Cancelled.into());
        }
        if let Some(publication) = &self.publication
            && let Err(error) = publication.validate()
        {
            self.retire(
                if error == ResolverError::Cancelled {
                    ProposalState::Cancelled
                } else {
                    ProposalState::Failed(error.into())
                },
                true,
            );
            return Err(error.into());
        }
        if self.cancel.is_cancelled() || self.availability.is_cancelled() {
            self.retire(ProposalState::Cancelled, true);
            return Err(ResolverError::Cancelled.into());
        }
        let duplicate = self.active.as_ref().is_some_and(|old| {
            TimelineBinding::from_event(&old.event) == TimelineBinding::from_event(&event)
                && same_payload(&old.event, &event)
        });
        // Older utterances retain metadata only. Any further message for them
        // conservatively withdraws support; only the exact current duplicate is
        // idempotent. Job rejection never legitimizes potentially revised data.
        if event.identity == self.scope.capture_identity() && !duplicate {
            self.track.withdraw_utterance(event.utterance);
            self.track_notice = self.track_status();
        }
        // An unchanged binding cannot silently change its evidence, including
        // changes that would fail admission (oversized text or Partial kind).
        if self.active.as_ref().is_some_and(|previous| {
            TimelineBinding::from_event(&previous.event) == TimelineBinding::from_event(&event)
                && !same_payload(&previous.event, &event)
        }) {
            self.retire(
                ProposalState::Failed(ResolverError::InvalidData.into()),
                false,
            );
            return Err(ResolverError::InvalidData.into());
        }
        validate_event(self.scope, &event, stamp)?;
        if let Some(previous) = &self.active {
            let old = &previous.event;
            if event.sequence < old.sequence || event.utterance < old.utterance {
                return Err(FuzzyAlignmentError::Obsolete);
            }
            if event.sequence == old.sequence || event.utterance == old.utterance {
                if event.sequence != old.sequence || event.utterance != old.utterance {
                    return Err(ResolverError::InvalidData.into());
                }
                if event.revision < old.revision {
                    return Err(FuzzyAlignmentError::Obsolete);
                }
                if event.revision == old.revision {
                    return Ok(false);
                }
            }
            if stamp.sequence < previous.stamp.sequence || stamp.ended_at < previous.stamp.ended_at
            {
                return Err(FuzzyAlignmentError::Obsolete);
            }
        }
        let Some(ticket) = self.ticket.checked_add(1) else {
            self.stop();
            return Err(ResolverError::InvalidData.into());
        };
        let now = Instant::now();
        let deadline = now
            .checked_add(FUZZY_MATCH_TIMEOUT)
            .zip(stamp.ended_at.checked_add(MAX_WINDOW_AGE))
            .map(|(request, window)| request.min(window))
            .ok_or(ResolverError::Stale)?;
        self.retire(ProposalState::Locating, true);
        self.ticket = ticket;
        let request = Arc::new(Request {
            ticket,
            event,
            stamp,
            deadline,
            cancel: ResolverCancellation::default(),
        });
        self.active = Some(request.clone());
        self.input.send_replace(Some(request));
        Ok(true)
    }
    pub fn state(&self) -> ProposalState {
        if self.status == ProposalState::Stopped {
            return self.status;
        }
        if let Some(publication) = &self.publication
            && let Err(error) = publication.validate()
        {
            return if error == ResolverError::Cancelled {
                ProposalState::Cancelled
            } else {
                ProposalState::Failed(error.into())
            };
        }
        if self.cancel.is_cancelled() || self.availability.is_cancelled() {
            return ProposalState::Cancelled;
        }
        if let Some(request) = &self.active {
            if let Err(error) = request
                .stamp
                .validate(self.scope.capture_identity(), Instant::now())
            {
                return ProposalState::Failed(error.into());
            }
            if self.status == ProposalState::Locating && Instant::now() >= request.deadline {
                return ProposalState::Failed(ResolverError::Stale.into());
            }
        }
        if let Some(proposal) = self
            .accepted
            .as_ref()
            .and_then(|value| value.result.as_ref().ok())
        {
            let Some(binding) = self.binding() else {
                return ProposalState::Failed(FuzzyAlignmentError::Obsolete);
            };
            if let Err(error) = proposal.validate(self.scope, binding) {
                return ProposalState::Failed(error);
            }
        }
        self.status
    }
    pub fn proposal(&self) -> Option<&FuzzyCueProposal> {
        if self.state() != ProposalState::Proposed {
            return None;
        }
        self.accepted.as_ref()?.result.as_ref().ok()
    }
    /// Consume at most the latest completion. Obsolete output cannot alter a
    /// newer request's state or release its proposal. No text leaves the owner.
    pub fn poll(&mut self) -> ProposalState {
        self.track.refresh(Instant::now());
        let state = self.poll_proposal();
        self.track_notice = self.track_status();
        state
    }
    fn poll_proposal(&mut self) -> ProposalState {
        let state = self.state();
        if state == ProposalState::Cancelled {
            self.retire(state, true);
            return state;
        }
        if state == ProposalState::Failed(ResolverError::Stale.into()) {
            self.retire(state, false);
            return state;
        }
        let update = self.output.borrow_and_update().clone();
        if let (Some(update), Some(request)) = (update, self.active.as_ref())
            && update.ticket == request.ticket
            && update.ticket != self.seen
            && update.scope == self.scope
            && update.binding == TimelineBinding::from_event(&request.event)
            && !request.cancel.is_cancelled()
        {
            self.seen = update.ticket;
            self.status = match &update.result {
                Ok(proposal) => match proposal.validate(self.scope, update.binding) {
                    Ok(()) => {
                        if let Err(error) = self.track.admit(proposal, Instant::now()) {
                            self.track.reject(error);
                        }
                        ProposalState::Proposed
                    }
                    Err(error) => {
                        self.track.clear();
                        ProposalState::Failed(error)
                    }
                },
                Err(FuzzyAlignmentError::NoUniqueMatch) => {
                    self.track.clear();
                    ProposalState::Unmatched
                }
                Err(error) => {
                    self.track.clear();
                    ProposalState::Failed(*error)
                }
            };
            self.accepted = Some(update);
        }
        self.state()
    }
    /// Cancellation-safe wait; the caller may select this alongside new ASR.
    pub async fn changed(&mut self) -> ProposalState {
        loop {
            let seen = self.seen;
            let track_notice = self.track_notice;
            let state = self.poll();
            if self.seen != seen
                || self.track_notice != track_notice
                || matches!(state, ProposalState::Stopped | ProposalState::Cancelled)
                || state == ProposalState::Failed(ResolverError::Stale.into())
            {
                return state;
            }
            let expires = self.active.as_ref().and_then(|request| {
                if self.status == ProposalState::Locating {
                    Some(request.deadline)
                } else {
                    request.stamp.ended_at.checked_add(MAX_WINDOW_AGE)
                }
            });
            let expires = self
                .track
                .next_expiry()
                .map(|track| expires.map_or(track, |request| request.min(track)))
                .or(expires);
            let expires = self
                .source_deadline
                .map(|source| expires.map_or(source, |old| source.min(old)))
                .or(expires);
            tokio::select! { biased;
                _ = self.cancel.cancelled() => {},
                _ = self.availability.cancelled() => {},
                update = self.output.changed() => if update.is_err() { self.retire(ProposalState::Cancelled, true); return ProposalState::Cancelled; },
                _ = async { if let Some(deadline) = expires { tokio::time::sleep_until(deadline.into()).await } else { std::future::pending::<()>().await } } => {},
            }
        }
    }
    pub fn stop(&mut self) {
        self.cancel.cancel();
        self.retire(ProposalState::Stopped, true);
    }
    fn retire(&mut self, status: ProposalState, clear: bool) {
        if status == ProposalState::Stopped {
            self.track.stop();
        } else if matches!(status, ProposalState::Cancelled | ProposalState::Failed(_))
            && !(self.track.retains_prepared_history()
                && status == ProposalState::Failed(ResolverError::Stale.into()))
        {
            self.track.clear();
        }
        if let Some(request) = &self.active {
            request.cancel.cancel();
        }
        self.input.send_replace(None);
        self.reset_output.send_replace(None);
        self.accepted = None;
        if clear {
            self.active = None;
        }
        self.status = status;
        self.track_notice = self.track_status();
    }
}
impl Drop for FuzzyProposalSession {
    fn drop(&mut self) {
        self.stop();
    }
}

impl FuzzyProposalTask {
    /// Run on the caller's timer-enabled Tokio runtime. No task or thread is
    /// detached; stop waits for a started locator job's cooperative cleanup.
    pub async fn run(mut self) -> Result<(), FuzzyAlignmentError> {
        let mut last = 0;
        loop {
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            if self
                .source_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(ResolverError::Stale.into());
            }
            if self.availability.is_cancelled() {
                return Err(ResolverError::Cancelled.into());
            }
            let current = self.input.borrow_and_update().clone();
            let Some(request) =
                current.filter(|request| request.ticket > last && !request.cancel.is_cancelled())
            else {
                tokio::select! { biased;
                    _ = self.cancel.cancelled() => return Ok(()),
                    _ = self.availability.cancelled() => return Err(ResolverError::Cancelled.into()),
                    _ = source_expired(self.source_deadline) => return Err(ResolverError::Stale.into()),
                    update = self.input.changed() => if update.is_err() { return Ok(()); },
                }
                continue;
            };
            last = request.ticket;
            let job = self.locator.locate_before(
                request.event.clone(),
                request.stamp,
                request.cancel.clone(),
                request.deadline,
            );
            tokio::pin!(job);
            let result = tokio::select! { biased;
                _ = self.cancel.cancelled() => { request.cancel.cancel(); let _ = job.await; return Ok(()); },
                _ = self.availability.cancelled() => { request.cancel.cancel(); let _ = job.await; return Err(ResolverError::Cancelled.into()); },
                result = &mut job => result,
            };
            if request.cancel.is_cancelled() {
                continue;
            }
            let latest = self
                .input
                .borrow()
                .as_ref()
                .is_some_and(|latest| latest.ticket == request.ticket);
            if latest {
                self.output.send_replace(Some(Arc::new(Completion {
                    ticket: request.ticket,
                    scope: self.locator.scope(),
                    binding: TimelineBinding::from_event(&request.event),
                    result,
                })));
            }
        }
    }
}
async fn source_expired(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline.into()).await
    } else {
        std::future::pending::<()>().await
    }
}
fn same_payload(left: &TranscriptEvent, right: &TranscriptEvent) -> bool {
    left.text == right.text
        && left.language == right.language
        && left.start_us == right.start_us
        && left.duration_us == right.duration_us
        && left.kind == right.kind
}
fn validate_event(
    scope: AlignmentScope,
    event: &TranscriptEvent,
    stamp: WindowStamp,
) -> Result<(), FuzzyAlignmentError> {
    stamp.validate(scope.capture_identity(), Instant::now())?;
    if event.identity != scope.capture_identity()
        || event.utterance == 0
        || event.sequence == 0
        || event.revision == 0
        || stamp.sequence == 0
        || event.sequence > stamp.sequence
        || event.kind != TranscriptKind::Final
        || event.language.is_none()
        || event.text.is_empty()
        || event.text.len() > MAX_TRANSCRIPT_BYTES
    {
        return Err(FuzzyAlignmentError::UnsupportedEvidence);
    }
    let start = event.start_us.ok_or(FuzzyAlignmentError::TimingMismatch)?;
    let end = start
        .checked_add(
            event
                .duration_us
                .filter(|duration| *duration > 0)
                .ok_or(FuzzyAlignmentError::TimingMismatch)?,
        )
        .ok_or(FuzzyAlignmentError::TimingMismatch)?;
    let window = stamp.start_us.ok_or(FuzzyAlignmentError::TimingMismatch)?;
    let limit = window
        .checked_add(stamp.duration_us)
        .ok_or(FuzzyAlignmentError::TimingMismatch)?;
    if start < window || end > limit {
        return Err(FuzzyAlignmentError::TimingMismatch);
    }
    Ok(())
}

#[cfg(test)]
#[path = "fuzzy_proposals_tests.rs"]
mod tests;
