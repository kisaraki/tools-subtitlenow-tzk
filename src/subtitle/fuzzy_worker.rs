//! Owned or Ready-scoped, cancellable fuzzy proposals; no UI/playback adoption.
use super::alignment_text::AlignmentText;
use super::fuzzy_alignment::{
    AlignmentScope, FUZZY_MATCH_TIMEOUT, FuzzyAlignmentError, FuzzyCueProposal, MAX_DP_CELLS,
    MatchPolicy, locate_paired_parts_with_policy, locate_paired_with_policy,
    locate_text_with_policy, validate_paired_budget,
};
use crate::{
    ai::speech::{MAX_TRANSCRIPT_BYTES, TranscriptEvent},
    resolver::contracts::{
        ExistingText, ExistingTextBundle, MAX_CUE_BYTES, MAX_METADATA_BYTES, MAX_TEXT_BYTES,
        MAX_TEXT_CUES, MAX_WINDOW_AGE, ResolverCancellation, ResolverError, WindowStamp,
    },
    resolver::text_session::{BorrowedLookupText, ExistingTextLease, WeakExistingTextLease},
    session::prepared::{PreparedAlignmentLease, PreparedTextOwner},
    subtitle::timeline::TimelineBinding,
};
use std::{future::Future, sync::Arc, time::Instant};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};

const MAX_OWNER_JOBS: usize = 1;

struct CancelOnDrop {
    token: ResolverCancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.token.cancel();
        }
    }
}
struct Worker<T>(JoinHandle<T>);
impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        // Abort queued work; a started CPU closure owns its permits and checks
        // the request token cooperatively until its inputs are really released.
        self.0.abort();
    }
}

/// One immutable bounded source document and one active request per owner.
/// Prepared inputs are weak, revocable text borrows; only an admitted job may
/// hold text through cleanup, and no media/native owner crosses this boundary.
/// Move a new document into a new owner for each publication/epoch. Stop/Drop
/// invalidates all outstanding futures and already delivered proposals.
/// Prepared Session/controller expose an explicit Ready bridge; source adoption
/// and playback still require separate verified evidence.
pub struct FuzzyCueLocator {
    scope: AlignmentScope,
    document: Option<AlignmentDocument>,
    stopped: ResolverCancellation,
    workers: Arc<Semaphore>,
    local: Arc<Semaphore>,
}
/// Private source binding for explicit alignment pause/reset. Both supported
/// sources are weak; this retains no Ready media or lookup text owner.
pub(crate) struct FuzzyLocatorBinding {
    document: AlignmentDocument,
    identity: crate::ai::speech::StreamIdentity,
    workers: Arc<Semaphore>,
    local: Arc<Semaphore>,
}
/// Opaque Ready handoff. Cloning retains neither text nor media. Each
/// controller created from this handle owns separate local work capacity.
pub struct AlignmentSource {
    document: PreparedAlignmentLease,
}
impl Clone for AlignmentSource {
    fn clone(&self) -> Self {
        Self {
            document: self.document.clone(),
        }
    }
}
impl AlignmentSource {
    pub(crate) fn is_available(&self) -> bool {
        !self.document.publication.is_cancelled() && self.document.text.strong_count() > 0
    }
    pub(crate) fn prepared(document: PreparedAlignmentLease) -> Result<Self, FuzzyAlignmentError> {
        check(&document.publication)?;
        if document.text.strong_count() == 0 {
            return Err(ResolverError::Cancelled.into());
        }
        Ok(Self { document })
    }
    pub fn identity(&self) -> crate::ai::speech::StreamIdentity {
        self.document.identity
    }
    pub(crate) fn locator(
        &self,
        capture: crate::ai::speech::StreamIdentity,
        epoch: u64,
    ) -> Result<FuzzyCueLocator, FuzzyAlignmentError> {
        check(&self.document.publication)?;
        FuzzyCueLocator::from_prepared(
            AlignmentScope::new(self.identity(), capture, epoch)?,
            self.document.clone(),
        )
    }
    pub(crate) fn display(
        &self,
        alignment: &super::alignment_verification::VerifiedAlignment,
    ) -> Result<AlignedCueText, FuzzyAlignmentError> {
        alignment
            .validate(alignment.scope(), alignment.binding())
            .map_err(|_| FuzzyAlignmentError::Obsolete)?;
        check(&self.document.publication)?;
        if alignment.scope().document_identity() != self.identity() {
            return Err(FuzzyAlignmentError::Obsolete);
        }
        let owner = self
            .document
            .text
            .upgrade()
            .ok_or(ResolverError::Cancelled)?;
        let mut text = owner.display(alignment.cues(), alignment.reference_cues())?;
        if text.media != alignment.media_range() {
            return Err(FuzzyAlignmentError::TimingMismatch);
        }
        text.text_sequence = self.document.sequence;
        Ok(text)
    }
}
pub(crate) struct AlignedCueText {
    pub(crate) original: Box<str>,
    pub(crate) translated: Box<str>,
    pub(crate) content_id: Box<str>,
    pub(crate) cue_index: usize,
    pub(crate) media: std::ops::Range<u64>,
    pub(crate) quality: Option<crate::domain::TranslationQuality>,
    pub(crate) text_sequence: u64,
}
impl FuzzyLocatorBinding {
    pub(crate) fn validate(&self) -> Result<(), FuzzyAlignmentError> {
        match &self.document {
            AlignmentDocument::Prepared(lease) => {
                check(&lease.publication)?;
                if lease.text.strong_count() == 0 {
                    return Err(ResolverError::Cancelled.into());
                }
                Ok(())
            }
            AlignmentDocument::Lookup(lease) => Ok(lease.validate()?),
            _ => Err(FuzzyAlignmentError::UnsupportedEvidence),
        }
    }
    pub(crate) fn locator(
        &self,
        capture: crate::ai::speech::StreamIdentity,
        epoch: u64,
    ) -> Result<FuzzyCueLocator, FuzzyAlignmentError> {
        self.validate()?;
        Ok(FuzzyCueLocator {
            scope: AlignmentScope::new(self.identity, capture, epoch)?,
            document: Some(self.document.clone()),
            stopped: ResolverCancellation::default(),
            workers: self.workers.clone(),
            local: self.local.clone(),
        })
    }
    /// A joined driver alone is insufficient after an abort: a started blocking
    /// closure retains input and permits until cooperative cleanup really ends.
    pub(crate) async fn wait_idle(&self) -> Result<(), FuzzyAlignmentError> {
        let _permit = self
            .local
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| FuzzyAlignmentError::WorkerFailed)?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn isolate(&mut self, workers: Arc<Semaphore>) {
        self.workers = workers;
    }
    #[cfg(test)]
    pub(crate) fn local(&self) -> Arc<Semaphore> {
        self.local.clone()
    }
}
impl FuzzyCueLocator {
    pub(crate) fn is_prepared(&self) -> bool {
        matches!(self.document, Some(AlignmentDocument::Prepared(_)))
    }
    /// Controller constructors call this only with a fresh private locator.
    pub(crate) fn into_binding(mut self) -> Result<FuzzyLocatorBinding, FuzzyAlignmentError> {
        let binding = FuzzyLocatorBinding {
            document: self.document.take().ok_or(ResolverError::Cancelled)?,
            identity: self.scope.document_identity(),
            workers: self.workers.clone(),
            local: self.local.clone(),
        };
        binding.validate()?;
        Ok(binding)
    }
    /// Borrow the immutable original lookup snapshot. Renewal requires a new
    /// locator; retirement/expiry invalidates work and delivered metadata.
    pub fn from_lookup(
        scope: AlignmentScope,
        lease: &ExistingTextLease,
    ) -> Result<Self, FuzzyAlignmentError> {
        lease.validate(scope.document_identity())?;
        if !matches!(
            lease.text().language,
            crate::resolver::contracts::TextLanguage::Source(_)
        ) && lease.source_reference().is_none()
        {
            return Err(FuzzyAlignmentError::UnsupportedEvidence);
        }
        let publication = lease.downgrade();
        publication.validate()?;
        Ok(Self {
            scope,
            document: Some(AlignmentDocument::Lookup(publication)),
            stopped: ResolverCancellation::default(),
            workers: super::alignment_work::workers(),
            local: Arc::new(Semaphore::new(MAX_OWNER_JOBS)),
        })
    }
    pub(crate) fn publication(&self) -> Option<WeakExistingTextLease> {
        match &self.document {
            Some(AlignmentDocument::Lookup(source)) => Some(source.clone()),
            _ => None,
        }
    }
    pub(crate) fn source_deadline(&self) -> Option<Instant> {
        self.publication().and_then(|source| source.deadline().ok())
    }
    /// Explicit immutable Chinese/reference publication. No provider lookup,
    /// guessed reference, Chinese-to-ASR matching or subtitle adoption.
    pub fn new_paired(
        scope: AlignmentScope,
        bundle: ExistingTextBundle,
    ) -> Result<Self, FuzzyAlignmentError> {
        validate_paired_budget(&bundle)?;
        if bundle.text.identity != scope.document_identity() {
            return Err(ResolverError::InvalidData.into());
        }
        Ok(Self {
            scope,
            document: Some(AlignmentDocument::Paired(Arc::new(bundle))),
            stopped: ResolverCancellation::default(),
            workers: super::alignment_work::workers(),
            local: Arc::new(Semaphore::new(MAX_OWNER_JOBS)),
        })
    }
    pub fn new(scope: AlignmentScope, document: ExistingText) -> Result<Self, FuzzyAlignmentError> {
        // Admission checks sizes/identity only (at most 8192 cue lengths).
        // Text scanning and full semantic validation belong to the CPU job.
        if document.identity != scope.document_identity()
            || document.content_id.len() > MAX_METADATA_BYTES
            || document.cues.is_empty()
            || document.cues.len() > MAX_TEXT_CUES
        {
            return Err(ResolverError::InvalidData.into());
        }
        let mut bytes = 0usize;
        for cue in &document.cues {
            bytes = bytes
                .checked_add(cue.text.len())
                .ok_or(ResolverError::InvalidData)?;
            if cue.text.len() > MAX_CUE_BYTES || bytes > MAX_TEXT_BYTES {
                return Err(ResolverError::InvalidData.into());
            }
        }
        Ok(Self {
            scope,
            document: Some(AlignmentDocument::Owned(Arc::new(document))),
            stopped: ResolverCancellation::default(),
            workers: super::alignment_work::workers(),
            local: Arc::new(Semaphore::new(MAX_OWNER_JOBS)),
        })
    }
    pub(crate) fn from_prepared(
        scope: AlignmentScope,
        document: PreparedAlignmentLease,
    ) -> Result<Self, FuzzyAlignmentError> {
        if scope.document_identity() != document.identity {
            return Err(ResolverError::InvalidData.into());
        }
        check(&document.publication)?;
        Ok(Self {
            scope,
            document: Some(AlignmentDocument::Prepared(document)),
            stopped: ResolverCancellation::default(),
            workers: super::alignment_work::workers(),
            local: Arc::new(Semaphore::new(MAX_OWNER_JOBS)),
        })
    }
    pub fn scope(&self) -> AlignmentScope {
        self.scope
    }
    #[cfg(test)]
    pub(crate) fn isolate_workers(&mut self, workers: Arc<Semaphore>) {
        self.workers = workers;
    }
    pub(crate) fn availability(&self) -> Result<ResolverCancellation, FuzzyAlignmentError> {
        let scope = match &self.document {
            Some(AlignmentDocument::Prepared(lease)) => ResolverCancellation::linked_many(&[
                self.stopped.clone(),
                lease.publication.clone(),
            ])?,
            Some(AlignmentDocument::Lookup(lease)) => {
                lease.validate()?;
                ResolverCancellation::linked_many(&[self.stopped.clone(), lease.availability()])?
            }
            _ => ResolverCancellation::linked(self.stopped.clone())?,
        };
        check(&scope)?;
        Ok(scope)
    }
    pub fn stop(&mut self) {
        self.stopped.cancel();
        self.document = None;
    }
    /// Passive until polled; no hidden task or input copy. Poll with a Tokio
    /// timer-enabled runtime and keep that runtime alive through CPU cleanup.
    /// Explicit cancellation/timeout waits for actual worker exit. Future Drop
    /// signals cancellation and aborts queued work; started work retains both
    /// permits and its document until cooperative exit. Success preserves the
    /// caller and owner cancellation links for proposal.validate().
    pub fn locate(
        &self,
        event: TranscriptEvent,
        stamp: WindowStamp,
        cancel: ResolverCancellation,
    ) -> impl Future<Output = Result<FuzzyCueProposal, FuzzyAlignmentError>> + Send + 'static {
        self.locate_using(event, stamp, cancel, run_match)
    }
    pub(crate) fn locate_before(
        &self,
        event: TranscriptEvent,
        stamp: WindowStamp,
        cancel: ResolverCancellation,
        deadline: Instant,
    ) -> impl Future<Output = Result<FuzzyCueProposal, FuzzyAlignmentError>> + Send + 'static {
        self.dispatch(event, stamp, cancel, Some(deadline), run_match)
    }
    fn locate_using<F>(
        &self,
        event: TranscriptEvent,
        stamp: WindowStamp,
        cancel: ResolverCancellation,
        operation: F,
    ) -> impl Future<Output = Result<FuzzyCueProposal, FuzzyAlignmentError>> + Send + 'static
    where
        F: FnOnce(&WorkInput) -> Result<FuzzyCueProposal, FuzzyAlignmentError> + Send + 'static,
    {
        self.dispatch(event, stamp, cancel, None, operation)
    }
    fn dispatch<F>(
        &self,
        event: TranscriptEvent,
        stamp: WindowStamp,
        cancel: ResolverCancellation,
        requested_deadline: Option<Instant>,
        operation: F,
    ) -> impl Future<Output = Result<FuzzyCueProposal, FuzzyAlignmentError>> + Send + 'static
    where
        F: FnOnce(&WorkInput) -> Result<FuzzyCueProposal, FuzzyAlignmentError> + Send + 'static,
    {
        let document = self.document.clone();
        let scope = self.scope;
        let workers = self.workers.clone();
        let local = self.local.clone();
        let scopes = match &document {
            Some(AlignmentDocument::Prepared(lease)) => ResolverCancellation::linked_many(&[
                self.stopped.clone(),
                cancel,
                lease.publication.clone(),
            ]),
            Some(AlignmentDocument::Lookup(lease)) => ResolverCancellation::linked_many(&[
                self.stopped.clone(),
                cancel,
                lease.availability(),
            ]),
            _ => ResolverCancellation::linked_many(&[self.stopped.clone(), cancel]),
        };
        let guard = scopes.map(|token| CancelOnDrop { token, armed: true });
        let source_deadline = self.source_deadline();
        async move {
            let mut guard = guard?;
            check(&guard.token)?;
            if event.text.len() > MAX_TRANSCRIPT_BYTES {
                return Err(FuzzyAlignmentError::UnsupportedEvidence);
            }
            stamp.validate(scope.capture_identity(), Instant::now())?;
            let deadline = stamp
                .ended_at
                .checked_add(MAX_WINDOW_AGE)
                .zip(Instant::now().checked_add(FUZZY_MATCH_TIMEOUT))
                .map(|(publication, request)| publication.min(request))
                .ok_or(ResolverError::Stale)?;
            let deadline = requested_deadline.map_or(deadline, |limit| deadline.min(limit));
            let deadline = source_deadline.map_or(deadline, |limit| deadline.min(limit));
            if Instant::now() >= deadline {
                return Err(ResolverError::Stale.into());
            }
            let runtime =
                Handle::try_current().map_err(|_| FuzzyAlignmentError::RuntimeUnavailable)?;
            let local = local
                .try_acquire_owned()
                .map_err(|_| FuzzyAlignmentError::Overloaded)?;
            let permit = workers
                .try_acquire_owned()
                .map_err(|_| FuzzyAlignmentError::Overloaded)?;
            let binding = TimelineBinding::from_event(&event);
            let input = WorkInput {
                scope,
                document: document.ok_or(ResolverError::Cancelled)?.acquire()?,
                event,
                stamp,
                cancel: guard.token.clone(),
                deadline,
            };
            let job = AdmittedWork {
                input,
                _permit: permit,
                _local: local,
            };
            let mut worker = Worker(runtime.spawn_blocking(move || job.execute(operation)));
            let (result, joined) = tokio::select! { biased;
                _ = guard.token.cancelled() => (Err(ResolverError::Cancelled.into()), false),
                _ = tokio::time::sleep_until(deadline.into()) => (Err(ResolverError::Stale.into()), false),
                result = &mut worker.0 => (result.map_err(|_| FuzzyAlignmentError::WorkerFailed).and_then(|value| value), true),
            };
            if result.is_err() {
                guard.token.cancel();
                if !joined {
                    worker.0.abort();
                    let _ = (&mut worker.0).await;
                }
                return result;
            }
            let proposal = result?;
            check(&guard.token)?;
            if Instant::now() >= deadline {
                return Err(ResolverError::Stale.into());
            }
            proposal.validate(scope, binding)?;
            guard.armed = false;
            Ok(proposal)
        }
    }
}
impl Drop for FuzzyCueLocator {
    fn drop(&mut self) {
        self.stop();
    }
}

struct WorkInput {
    scope: AlignmentScope,
    document: AlignmentInput,
    event: TranscriptEvent,
    stamp: WindowStamp,
    cancel: ResolverCancellation,
    deadline: Instant,
}
#[derive(Clone)]
enum AlignmentDocument {
    Owned(Arc<ExistingText>),
    Paired(Arc<ExistingTextBundle>),
    Prepared(PreparedAlignmentLease),
    Lookup(WeakExistingTextLease),
}
impl AlignmentDocument {
    fn acquire(self) -> Result<AlignmentInput, FuzzyAlignmentError> {
        match self {
            Self::Owned(document) => Ok(AlignmentInput::Owned(document)),
            Self::Paired(document) => Ok(AlignmentInput::Paired(document)),
            Self::Lookup(lease) => {
                let document = lease.acquire()?;
                Ok(AlignmentInput::Lookup(document, lease))
            }
            Self::Prepared(lease) => {
                check(&lease.publication)?;
                Ok(AlignmentInput::Prepared(
                    lease.text.upgrade().ok_or(ResolverError::Cancelled)?,
                ))
            }
        }
    }
}
enum AlignmentInput {
    Owned(Arc<ExistingText>),
    Paired(Arc<ExistingTextBundle>),
    Prepared(Arc<PreparedTextOwner>),
    Lookup(BorrowedLookupText, WeakExistingTextLease),
}
impl AlignmentInput {
    fn original(&self) -> Result<AlignmentText<'_>, FuzzyAlignmentError> {
        match self {
            Self::Owned(document) => Ok(AlignmentText::Existing(document)),
            Self::Paired(_) => Err(FuzzyAlignmentError::UnsupportedEvidence),
            Self::Prepared(document) => document.alignment_source(),
            Self::Lookup(document, _) => Ok(AlignmentText::Existing(&document.text)),
        }
    }
}
// Rust drops struct fields in declaration order. Keep input before permits,
// including when Tokio drops a queued closure or unwinds a started one.
struct AdmittedWork {
    input: WorkInput,
    _permit: OwnedSemaphorePermit,
    _local: OwnedSemaphorePermit,
}
impl AdmittedWork {
    fn execute<F>(self, operation: F) -> Result<FuzzyCueProposal, FuzzyAlignmentError>
    where
        F: FnOnce(&WorkInput) -> Result<FuzzyCueProposal, FuzzyAlignmentError>,
    {
        check(&self.input.cancel)?;
        if Instant::now() >= self.input.deadline {
            return Err(ResolverError::Stale.into());
        }
        operation(&self.input)
    }
}
fn run_match(input: &WorkInput) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    let policy = MatchPolicy {
        cell_limit: MAX_DP_CELLS,
        deadline: Some(input.deadline),
        #[cfg(test)]
        check_hook: None,
    };
    if let AlignmentInput::Lookup(document, publication) = &input.document {
        publication.validate()?;
        let proposal = match &document.source_reference {
            Some(reference) => locate_paired_parts_with_policy(
                input.scope,
                (&document.text, reference),
                &input.event,
                input.stamp,
                &input.cancel,
                policy,
            ),
            None => locate_text_with_policy(
                input.scope,
                AlignmentText::Existing(&document.text),
                &input.event,
                input.stamp,
                &input.cancel,
                policy,
            ),
        }?;
        return proposal.bind_publication(publication.clone());
    }
    if let AlignmentInput::Paired(bundle) = &input.document {
        return locate_paired_with_policy(
            input.scope,
            bundle,
            &input.event,
            input.stamp,
            &input.cancel,
            policy,
        );
    }
    if let AlignmentInput::Prepared(document) = &input.document
        && let Some(pair) = document.alignment_pair()
    {
        return locate_paired_parts_with_policy(
            input.scope,
            pair,
            &input.event,
            input.stamp,
            &input.cancel,
            policy,
        );
    }
    locate_text_with_policy(
        input.scope,
        input.document.original()?,
        &input.event,
        input.stamp,
        &input.cancel,
        policy,
    )
}
fn check(cancel: &ResolverCancellation) -> Result<(), FuzzyAlignmentError> {
    if cancel.is_cancelled() {
        Err(ResolverError::Cancelled.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "fuzzy_worker_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "fuzzy_prepared_tests.rs"]
mod prepared_tests;

#[cfg(test)]
#[path = "fuzzy_paired_worker_tests.rs"]
mod paired_tests;

#[cfg(test)]
#[path = "fuzzy_lookup_tests.rs"]
pub(crate) mod lookup_tests;
