//! Provider-declared same-edition source references; never match Chinese text.
use super::*;
use crate::resolver::contracts::{
    ExistingTextBundle, MAX_CUE_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CUES, SourceTextReference,
    TextLanguage,
};

/// Synchronous CPU-only hint for an explicit atomic Chinese/reference bundle.
/// Search the source reference, then require unique exact time correspondence
/// to the primary Chinese cues. No localization, adoption or position evidence.
pub fn locate_paired_fuzzy_cue(
    scope: AlignmentScope,
    bundle: &ExistingTextBundle,
    event: &TranscriptEvent,
    stamp: WindowStamp,
    cancel: &ResolverCancellation,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    locate_paired_with_policy(
        scope,
        bundle,
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

/// Admission reads lengths only. Bound both tracks together before scanning
/// text, and do not accept an unpaired Chinese document via a guessed source.
pub(in crate::subtitle) fn validate_paired_budget(
    bundle: &ExistingTextBundle,
) -> Result<(), FuzzyAlignmentError> {
    let reference = bundle
        .source_reference
        .as_ref()
        .ok_or(FuzzyAlignmentError::UnsupportedEvidence)?;
    validate_paired_parts(&bundle.text, reference)
}
pub(crate) fn validate_paired_parts(
    primary: &ExistingText,
    reference: &SourceTextReference,
) -> Result<(), FuzzyAlignmentError> {
    if !matches!(
        primary.language,
        TextLanguage::TaiwanTraditional
            | TextLanguage::OtherTraditional
            | TextLanguage::SimplifiedChinese
    ) {
        return Err(FuzzyAlignmentError::UnsupportedEvidence);
    }
    let count = primary.cues.len().checked_add(reference.text.cues.len());
    if primary.cues.is_empty()
        || reference.text.cues.is_empty()
        || count.is_none_or(|count| count > MAX_TEXT_CUES)
    {
        return Err(FuzzyAlignmentError::BudgetExceeded);
    }
    let mut bytes = 0usize;
    for cue in primary.cues.iter().chain(&reference.text.cues) {
        bytes = bytes
            .checked_add(cue.text.len())
            .ok_or(FuzzyAlignmentError::BudgetExceeded)?;
        if cue.text.len() > MAX_CUE_BYTES || bytes > MAX_TEXT_BYTES {
            return Err(FuzzyAlignmentError::BudgetExceeded);
        }
    }
    Ok(())
}

pub(in crate::subtitle) fn locate_paired_with_policy(
    scope: AlignmentScope,
    bundle: &ExistingTextBundle,
    event: &TranscriptEvent,
    stamp: WindowStamp,
    cancel: &ResolverCancellation,
    policy: MatchPolicy,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    let reference = bundle
        .source_reference
        .as_ref()
        .ok_or(FuzzyAlignmentError::UnsupportedEvidence)?;
    locate_paired_parts_with_policy(
        scope,
        (&bundle.text, reference),
        event,
        stamp,
        cancel,
        policy,
    )
}
pub(in crate::subtitle) fn locate_paired_parts_with_policy(
    scope: AlignmentScope,
    (primary, reference): (&ExistingText, &SourceTextReference),
    event: &TranscriptEvent,
    stamp: WindowStamp,
    cancel: &ResolverCancellation,
    mut policy: MatchPolicy,
) -> Result<FuzzyCueProposal, FuzzyAlignmentError> {
    if cancel.is_cancelled() {
        return Err(ResolverError::Cancelled.into());
    }
    stamp.validate(scope.capture_identity(), Instant::now())?;
    let deadline = stamp
        .ended_at
        .checked_add(MAX_WINDOW_AGE)
        .zip(Instant::now().checked_add(FUZZY_MATCH_TIMEOUT))
        .map(|(window, request)| {
            policy
                .deadline
                .map_or(window.min(request), |limit| limit.min(window).min(request))
        })
        .ok_or(ResolverError::Stale)?;
    let work = WorkCheck::new(
        cancel,
        deadline,
        MatchPolicy {
            cell_limit: 0,
            deadline: Some(deadline),
            #[cfg(test)]
            check_hook: None,
        },
    );
    work.checkpoint()?;
    validate_paired_parts(primary, reference)?;
    reference.validate_document(primary, scope.document_identity())?;
    work.checkpoint()?;
    policy.deadline = Some(deadline);
    let mut proposal = locate_with_policy(scope, &reference.text, event, stamp, cancel, policy)?;
    let primary = map_primary(primary, &reference.text, proposal.cue_range(), &work)?;
    proposal.origin = FuzzyTextOrigin::PairedSourceReference;
    proposal.paired_primary_cues = Some(primary);
    work.checkpoint()?;
    proposal.validate(scope, TimelineBinding::from_event(event))?;
    Ok(proposal)
}

fn map_primary(
    primary: &ExistingText,
    reference: &ExistingText,
    selected: Range<usize>,
    work: &WorkCheck<'_>,
) -> Result<Range<usize>, FuzzyAlignmentError> {
    let mut first = None;
    let mut previous: Option<usize> = None;
    for index in selected {
        work.checkpoint()?;
        let source = reference
            .cues
            .get(index)
            .ok_or(ResolverError::InvalidData)?;
        let mut matched = None;
        // Scan the whole primary track: repeated timing cannot become unique
        // merely because the first matching cue had the expected index.
        for (candidate, cue) in primary.cues.iter().enumerate() {
            work.checkpoint()?;
            if cue.start_us == source.start_us
                && cue.duration_us == source.duration_us
                && matched.replace(candidate).is_some()
            {
                return Err(FuzzyAlignmentError::NoUniqueMatch);
            }
        }
        let matched = matched.ok_or(FuzzyAlignmentError::TimingMismatch)?;
        if previous.is_some_and(|old| old.checked_add(1) != Some(matched)) {
            return Err(FuzzyAlignmentError::TimingMismatch);
        }
        first.get_or_insert(matched);
        previous = Some(matched);
    }
    Ok(first.ok_or(FuzzyAlignmentError::TimingMismatch)?
        ..previous.ok_or(FuzzyAlignmentError::TimingMismatch)? + 1)
}

#[cfg(test)]
#[path = "fuzzy_paired_tests.rs"]
pub(crate) mod tests;
