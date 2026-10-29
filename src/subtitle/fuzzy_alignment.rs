//! Bounded character-edit proposals for explicitly bound source documents.
//! The fuzzy_worker library dispatches this synchronous core on the shared
//! alignment CPU lane; never call it on capture, UI, or async executor threads.
//! Proposals are hints, not media identification, edition, playback position,
//! alignment evidence, or permission to replace the shared transcript timeline.
use super::alignment_text::AlignmentText;
use crate::{
    ai::speech::{MAX_TRANSCRIPT_BYTES, StreamIdentity, TranscriptEvent, TranscriptKind},
    resolver::contracts::{
        ExistingText, MAX_WINDOW_AGE, ResolverCancellation, ResolverError, WindowStamp,
    },
    resolver::text_session::WeakExistingTextLease,
    subtitle::timeline::TimelineBinding,
};
use std::{
    ops::Range,
    time::{Duration, Instant},
};

/// Reject short/common ASR and selected cues by letters/numbers, not byte count.
pub const MIN_MATCH_ALPHANUMERIC: usize = 16;
/// The event and every cue must fit; no truncation or skipped oversized cue can
/// hide another possible match and manufacture a unique result.
pub const MAX_NORMALIZED_CHARACTERS: usize = 256;
/// Compare every single cue and every adjacent pair, never an arbitrary span.
pub const MAX_MATCH_CUES: usize = 2;
/// Two individually bounded cues plus one explicit word boundary.
pub const MAX_MATCH_CHARACTERS: usize =
    MAX_MATCH_CUES * MAX_NORMALIZED_CHARACTERS + MAX_MATCH_CUES - 1;
/// A winning pair must have complete, non-overlapping times with a short gap.
/// All pairs still compete lexically, including untimed or distant repeats.
pub const MAX_MATCH_GAP_US: u64 = 500_000;
/// Engineering edit-agreement gates, not calibrated probabilities or confidence.
pub const MIN_TEXT_AGREEMENT_PERMILLE: u16 = 900;
pub const MIN_WINNER_MARGIN_PERMILLE: u16 = 50;
/// The band includes below-threshold competitors that can defeat the margin.
pub const MIN_COMPETITOR_AGREEMENT_PERMILLE: u16 =
    MIN_TEXT_AGREEMENT_PERMILLE - MIN_WINNER_MARGIN_PERMILLE;
/// Count actual dynamic-programming recurrence cells across the entire document.
pub const MAX_DP_CELLS: usize = 2 * 1024 * 1024;
pub const FUZZY_MATCH_TIMEOUT: Duration = Duration::from_secs(3);
/// Check long whitespace runs as well as retained text during normalization.
const NORMALIZATION_CHECK_INTERVAL: usize = 32;
const PERMILLE: usize = 1000;
const DISTANCE_SENTINEL: usize = MAX_MATCH_CHARACTERS + 1;

#[path = "fuzzy_paired.rs"]
pub(crate) mod paired;
pub use paired::locate_paired_fuzzy_cue;
pub(super) use paired::{
    locate_paired_parts_with_policy, locate_paired_with_policy, validate_paired_budget,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FuzzyAlignmentError {
    #[error("{0}")]
    Invalid(#[from] ResolverError),
    #[error("模糊定位需要語言一致、時間完整且足夠長的最終原文辨識。")]
    UnsupportedEvidence,
    #[error("字幕未取得足夠明確的唯一文字候選，繼續使用目前字幕。")]
    NoUniqueMatch,
    #[error("字幕或辨識的時間範圍無效，未產生定位候選。")]
    TimingMismatch,
    #[error("模糊定位超過本次文字或運算上限，未交付候選。")]
    BudgetExceeded,
    #[error("字幕定位候選已被新的來源或辨識修訂取代。")]
    Obsolete,
    #[error("字幕模糊定位工作已達容量上限，未排入本次工作。")]
    Overloaded,
    #[error("字幕模糊定位背景工作異常結束。")]
    WorkerFailed,
    #[error("字幕模糊定位需要可用的非同步執行環境。")]
    RuntimeUnavailable,
    #[error("尚未完成可供定位的字幕準備。")]
    NotReady,
}

/// IDs bind two genuinely distinct Sessions without asserting a media match.
/// The future owner must advance epoch when its document publication, source,
/// or seek state changes; the matcher cannot detect an unchanged stale epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignmentScope {
    document_identity: StreamIdentity,
    capture_identity: StreamIdentity,
    epoch: u64,
}
impl AlignmentScope {
    pub fn new(
        document_identity: StreamIdentity,
        capture_identity: StreamIdentity,
        epoch: u64,
    ) -> Result<Self, FuzzyAlignmentError> {
        if document_identity.session.0 == 0
            || document_identity.generation == 0
            || capture_identity.session.0 == 0
            || capture_identity.generation == 0
            || epoch == 0
        {
            return Err(ResolverError::InvalidData.into());
        }
        Ok(Self {
            document_identity,
            capture_identity,
            epoch,
        })
    }
    pub fn document_identity(self) -> StreamIdentity {
        self.document_identity
    }
    pub fn capture_identity(self) -> StreamIdentity {
        self.capture_identity
    }
    pub fn epoch(self) -> u64 {
        self.epoch
    }
}

/// Preserve source provenance without promoting recognition to a subtitle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuzzyTextOrigin {
    ExistingText,
    PreparedRecognition,
    PairedSourceReference,
}

/// Metadata only: no private text, content ID, document owner, serialization or
/// conversion to ExactCueMatch/CandidateEvidence. The two ranges are original
/// observed intervals; their difference is not a validated playback offset.
pub struct FuzzyCueProposal {
    origin: FuzzyTextOrigin,
    scope: AlignmentScope,
    binding: TimelineBinding,
    stamp: WindowStamp,
    cancellation: ResolverCancellation,
    cue_range: Range<usize>,
    paired_primary_cues: Option<Range<usize>>,
    media_range: Range<u64>,
    capture_range: Range<u64>,
    distance: usize,
    agreement: u16,
    runner_up_agreement: Option<u16>,
    publication: Option<WeakExistingTextLease>,
}
impl FuzzyCueProposal {
    pub(crate) fn window_stamp(&self) -> WindowStamp {
        self.stamp
    }
    pub(crate) fn bind_publication(
        mut self,
        publication: WeakExistingTextLease,
    ) -> Result<Self, FuzzyAlignmentError> {
        publication.validate()?;
        self.publication = Some(publication);
        Ok(self)
    }
    pub(crate) fn observation_deadline(&self) -> Result<Instant, FuzzyAlignmentError> {
        let capture = self
            .stamp
            .ended_at
            .checked_add(MAX_WINDOW_AGE)
            .ok_or(ResolverError::Stale)?;
        Ok(self
            .publication
            .as_ref()
            .map(|source| source.deadline())
            .transpose()?
            .map_or(capture, |source| source.min(capture)))
    }
    pub fn origin(&self) -> FuzzyTextOrigin {
        self.origin
    }
    pub fn scope(&self) -> AlignmentScope {
        self.scope
    }
    pub fn binding(&self) -> TimelineBinding {
        self.binding
    }
    pub fn cue_index(&self) -> usize {
        self.cue_range.start
    }
    /// Half-open original cue indices. `cue_index()` is the first cue only.
    pub fn cue_range(&self) -> Range<usize> {
        self.cue_range.clone()
    }
    /// Chinese primary indices, present only for an explicitly paired source
    /// reference. `cue_range()` always indexes the searched source text.
    pub fn paired_primary_cue_range(&self) -> Option<Range<usize>> {
        self.paired_primary_cues.clone()
    }
    pub fn media_range_us(&self) -> Range<u64> {
        self.media_range.clone()
    }
    pub fn capture_range_us(&self) -> Range<u64> {
        self.capture_range.clone()
    }
    pub fn edit_distance(&self) -> usize {
        self.distance
    }
    pub fn text_agreement_permille(&self) -> u16 {
        self.agreement
    }
    /// None means no competitor reached 850 permille under this algorithm,
    /// rather than an assertion that the document has no other cue.
    pub fn runner_up_agreement_permille(&self) -> Option<u16> {
        self.runner_up_agreement
    }
    /// Recheck the caller's explicit epoch/IDs and all ASR binding fields. This
    /// cannot detect document mutations or event text edits under unchanged IDs.
    pub fn validate(
        &self,
        scope: AlignmentScope,
        current_binding: TimelineBinding,
    ) -> Result<(), FuzzyAlignmentError> {
        if let Some(publication) = &self.publication {
            publication.validate()?;
        }
        if self.cancellation.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        if self.scope != scope || self.binding != current_binding {
            return Err(FuzzyAlignmentError::Obsolete);
        }
        self.stamp
            .validate(scope.capture_identity, Instant::now())?;
        Ok(())
    }
}

pub(super) struct MatchPolicy {
    pub(super) cell_limit: usize,
    pub(super) deadline: Option<Instant>,
    #[cfg(test)]
    pub(super) check_hook: Option<Box<dyn Fn()>>,
}
struct WorkCheck<'a> {
    cancel: &'a ResolverCancellation,
    deadline: Instant,
    cells: usize,
    cell_limit: usize,
    #[cfg(test)]
    check_hook: Option<Box<dyn Fn()>>,
}
impl<'a> WorkCheck<'a> {
    fn new(cancel: &'a ResolverCancellation, deadline: Instant, policy: MatchPolicy) -> Self {
        let deadline = policy
            .deadline
            .map_or(deadline, |limit| deadline.min(limit));
        Self {
            cancel,
            deadline,
            cells: 0,
            cell_limit: policy.cell_limit,
            #[cfg(test)]
            check_hook: policy.check_hook,
        }
    }
    fn checkpoint(&self) -> Result<(), FuzzyAlignmentError> {
        #[cfg(test)]
        if let Some(hook) = &self.check_hook {
            hook();
        }
        if self.cancel.is_cancelled() {
            return Err(ResolverError::Cancelled.into());
        }
        if Instant::now() >= self.deadline {
            return Err(ResolverError::Stale.into());
        }
        Ok(())
    }
    fn visit_cell(&mut self) -> Result<(), FuzzyAlignmentError> {
        if self.cells >= self.cell_limit {
            return Err(FuzzyAlignmentError::BudgetExceeded);
        }
        self.cells += 1;
        Ok(())
    }
}

/// Match source-language text only. Chinese, translated text and paired tracks
/// are unsupported here. Ready workers separately borrow sealed recognized
/// originals without constructing an ExistingText. All cues, even
/// untimed or distant repeats, participate before a winner can be returned.
/// Invoke only on a bounded CPU worker; construction creates no worker or I/O.
pub fn locate_fuzzy_cue(
    scope: AlignmentScope,
    document: &ExistingText,
    event: &TranscriptEvent,
    stamp: WindowStamp,
    cancel: &ResolverCancellation,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    locate_with_policy(
        scope,
        document,
        event,
        stamp,
        cancel,
        MatchPolicy {
            cell_limit: MAX_DP_CELLS,
            deadline: None,
            #[cfg(test)]
            check_hook: None,
        },
    )
}

pub(super) fn locate_with_policy(
    scope: AlignmentScope,
    document: &ExistingText,
    event: &TranscriptEvent,
    stamp: WindowStamp,
    cancel: &ResolverCancellation,
    policy: MatchPolicy,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    locate_text_with_policy(
        scope,
        AlignmentText::Existing(document),
        event,
        stamp,
        cancel,
        policy,
    )
}

pub(super) fn locate_text_with_policy(
    scope: AlignmentScope,
    document: AlignmentText<'_>,
    event: &TranscriptEvent,
    stamp: WindowStamp,
    cancel: &ResolverCancellation,
    policy: MatchPolicy,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    if cancel.is_cancelled() {
        return Err(ResolverError::Cancelled.into());
    }
    stamp.validate(scope.capture_identity, Instant::now())?;
    let now = Instant::now();
    let deadline = stamp
        .ended_at
        .checked_add(MAX_WINDOW_AGE)
        .zip(now.checked_add(FUZZY_MATCH_TIMEOUT))
        .map(|(publication, job)| publication.min(job))
        .ok_or(ResolverError::Stale)?;
    let mut work = WorkCheck::new(cancel, deadline, policy);
    work.checkpoint()?;
    // Reuse the original bounded document contract. This one validation scans
    // at most four MiB/8192 cues; subsequent work checks cancellation per cue,
    // small normalization block and DP row rather than duplicating validation.
    document.validate(scope.document_identity)?;
    work.checkpoint()?;
    if stamp.sequence == 0
        || event.identity != scope.capture_identity
        || event.utterance == 0
        || event.sequence == 0
        || event.revision == 0
        || event.kind != TranscriptKind::Final
        || document.language().is_none()
        || document.language() != event.language
        || event.text.is_empty()
        || event.text.len() > MAX_TRANSCRIPT_BYTES
        || event
            .text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(FuzzyAlignmentError::UnsupportedEvidence);
    }
    let capture_range = interval(event.start_us, event.duration_us)?;
    let window = interval(stamp.start_us, Some(stamp.duration_us))?;
    if capture_range.start < window.start || capture_range.end > window.end {
        return Err(FuzzyAlignmentError::TimingMismatch);
    }
    let expected = normalize(&event.text, &work)?;
    if expected.alphanumeric < MIN_MATCH_ALPHANUMERIC {
        return Err(FuzzyAlignmentError::UnsupportedEvidence);
    }
    let mut ranking = Ranking::default();
    let mut previous: Option<Normalized> = None;
    for index in 0..document.len() {
        work.checkpoint()?;
        let cue = document.cue(index).ok_or(ResolverError::InvalidData)?;
        let normalized = normalize(cue.text, &work)?;
        ranking.consider(&expected, &normalized, index..index + 1, &mut work)?;
        if let Some(previous) = &previous {
            let joined = join_cues(previous, &normalized);
            ranking.consider(&expected, &joined, index - 1..index + 1, &mut work)?;
        }
        previous = Some(normalized);
    }
    work.checkpoint()?;
    let best = ranking
        .best
        .filter(|best| best.agreement >= MIN_TEXT_AGREEMENT_PERMILLE)
        .ok_or(FuzzyAlignmentError::NoUniqueMatch)?;
    if ranking.runner_up.as_ref().is_some_and(|runner| {
        best.agreement.saturating_sub(runner.agreement) < MIN_WINNER_MARGIN_PERMILLE
    }) {
        return Err(FuzzyAlignmentError::NoUniqueMatch);
    }
    if best.alphanumeric < MIN_MATCH_ALPHANUMERIC {
        return Err(FuzzyAlignmentError::UnsupportedEvidence);
    }
    let media_range = matched_interval(&document, best.cues.clone())?;
    work.checkpoint()?;
    Ok(FuzzyCueProposal {
        origin: document.origin(),
        scope,
        binding: TimelineBinding::from_event(event),
        stamp,
        cancellation: cancel.clone(),
        cue_range: best.cues,
        paired_primary_cues: None,
        media_range,
        capture_range,
        distance: best.distance,
        agreement: best.agreement,
        runner_up_agreement: ranking.runner_up.map(|runner| runner.agreement),
        publication: None,
    })
}

struct RankedCue {
    cues: Range<usize>,
    distance: usize,
    agreement: u16,
    alphanumeric: usize,
}
#[derive(Default)]
struct Ranking {
    best: Option<RankedCue>,
    runner_up: Option<RankedCue>,
}
impl Ranking {
    fn consider(
        &mut self,
        expected: &Normalized,
        candidate: &Normalized,
        cues: Range<usize>,
        work: &mut WorkCheck<'_>,
    ) -> Result<(), FuzzyAlignmentError> {
        let Some(distance) = edit_distance(&expected.characters, &candidate.characters, work)?
        else {
            return Ok(());
        };
        let length = expected.characters.len().max(candidate.characters.len());
        let agreement = ((length - distance) * PERMILLE / length) as u16;
        let ranked = RankedCue {
            cues,
            distance,
            agreement,
            alphanumeric: candidate.alphanumeric,
        };
        if self
            .best
            .as_ref()
            .is_none_or(|old| agreement > old.agreement)
        {
            self.runner_up = self.best.replace(ranked);
        } else if self
            .runner_up
            .as_ref()
            .is_none_or(|old| agreement > old.agreement)
        {
            self.runner_up = Some(ranked);
        }
        Ok(())
    }
}
struct Normalized {
    characters: Vec<char>,
    alphanumeric: usize,
}
fn join_cues(left: &Normalized, right: &Normalized) -> Normalized {
    // Each input was normalized and bounded independently. Preserve a boundary
    // even for Japanese/Korean; edit distance may account for that one space.
    let mut characters = Vec::with_capacity(MAX_MATCH_CHARACTERS);
    characters.extend_from_slice(&left.characters);
    if !left.characters.is_empty() && !right.characters.is_empty() {
        characters.push(' ');
    }
    characters.extend_from_slice(&right.characters);
    Normalized {
        characters,
        alphanumeric: left.alphanumeric + right.alphanumeric,
    }
}
fn matched_interval(
    document: &AlignmentText<'_>,
    cues: Range<usize>,
) -> Result<Range<u64>, FuzzyAlignmentError> {
    let first = document.cue(cues.start).ok_or(ResolverError::InvalidData)?;
    let mut range = interval(first.start_us, first.duration_us)?;
    for index in cues.start + 1..cues.end {
        let next = document.cue(index).ok_or(ResolverError::InvalidData)?;
        let next = interval(next.start_us, next.duration_us)?;
        if next.start < range.end || next.start - range.end > MAX_MATCH_GAP_US {
            return Err(FuzzyAlignmentError::TimingMismatch);
        }
        range.end = next.end;
    }
    Ok(range)
}
/// Equivalent to split_whitespace + ASCII lowercase + single-space join.
/// Scalar edits preserve punctuation, accents and word boundaries, not bytes.
fn normalize(text: &str, work: &WorkCheck<'_>) -> Result<Normalized, FuzzyAlignmentError> {
    let mut characters = Vec::with_capacity(MAX_NORMALIZED_CHARACTERS);
    let mut alphanumeric = 0;
    let mut pending_space = false;
    for (index, character) in text.chars().enumerate() {
        if index % NORMALIZATION_CHECK_INTERVAL == 0 {
            work.checkpoint()?;
        }
        if character.is_whitespace() {
            pending_space = !characters.is_empty();
            continue;
        }
        if pending_space {
            push_character(&mut characters, ' ')?;
            pending_space = false;
        }
        let character = character.to_ascii_lowercase();
        push_character(&mut characters, character)?;
        alphanumeric += usize::from(character.is_alphanumeric());
    }
    work.checkpoint()?;
    Ok(Normalized {
        characters,
        alphanumeric,
    })
}
fn push_character(characters: &mut Vec<char>, character: char) -> Result<(), FuzzyAlignmentError> {
    if characters.len() == MAX_NORMALIZED_CHARACTERS {
        return Err(FuzzyAlignmentError::BudgetExceeded);
    }
    characters.push(character);
    Ok(())
}
fn interval(start: Option<u64>, duration: Option<u64>) -> Result<Range<u64>, FuzzyAlignmentError> {
    let start = start.ok_or(FuzzyAlignmentError::TimingMismatch)?;
    let duration = duration
        .filter(|duration| *duration > 0)
        .ok_or(FuzzyAlignmentError::TimingMismatch)?;
    let end = start
        .checked_add(duration)
        .ok_or(FuzzyAlignmentError::TimingMismatch)?;
    Ok(start..end)
}

/// Returns the exact distance only when agreement can reach the competitor
/// floor. A length lower bound or the band can reject distant cues without
/// claiming an exact below-band runner-up score. Rows use fixed bounded RAM.
fn edit_distance(
    left: &[char],
    right: &[char],
    work: &mut WorkCheck<'_>,
) -> Result<Option<usize>, FuzzyAlignmentError> {
    work.checkpoint()?;
    if left.len() > MAX_NORMALIZED_CHARACTERS || right.len() > MAX_MATCH_CHARACTERS {
        return Err(FuzzyAlignmentError::BudgetExceeded);
    }
    let length = left.len().max(right.len());
    if length == 0 {
        return Ok(Some(0));
    }
    let band = length * (PERMILLE - usize::from(MIN_COMPETITOR_AGREEMENT_PERMILLE)) / PERMILLE;
    if left.len().abs_diff(right.len()) > band {
        return Ok(None);
    }
    let mut previous = [DISTANCE_SENTINEL; MAX_MATCH_CHARACTERS + 1];
    let mut current = [DISTANCE_SENTINEL; MAX_MATCH_CHARACTERS + 1];
    for (column, value) in previous
        .iter_mut()
        .take(right.len().min(band) + 1)
        .enumerate()
    {
        *value = column;
    }
    for row in 1..=left.len() {
        work.checkpoint()?;
        current.fill(DISTANCE_SENTINEL);
        if row <= band {
            current[0] = row;
        }
        let first = row.saturating_sub(band).max(1);
        let last = (row + band).min(right.len());
        for column in first..=last {
            work.visit_cell()?;
            let substitution = usize::from(left[row - 1] != right[column - 1]);
            current[column] = (previous[column] + 1)
                .min(current[column - 1] + 1)
                .min(previous[column - 1] + substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    work.checkpoint()?;
    Ok((previous[right.len()] <= band).then_some(previous[right.len()]))
}

#[cfg(test)]
#[path = "fuzzy_alignment_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "fuzzy_recognized_tests.rs"]
mod recognized_tests;
