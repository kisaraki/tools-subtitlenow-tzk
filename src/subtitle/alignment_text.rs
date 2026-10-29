//! Borrowed source views. Never copy recognized text into an ExistingText.
use super::fuzzy_alignment::{FuzzyAlignmentError, FuzzyTextOrigin};
use crate::{
    ai::{
        prepared_transcript::PreparedTranscript,
        speech::{SpeechLanguage, StreamIdentity},
    },
    resolver::contracts::{ExistingText, ResolverError, TextLanguage},
};

pub(crate) enum AlignmentText<'a> {
    Existing(&'a ExistingText),
    Recognized(&'a PreparedTranscript),
}
pub(crate) struct AlignmentCue<'a> {
    pub(crate) text: &'a str,
    pub(crate) start_us: Option<u64>,
    pub(crate) duration_us: Option<u64>,
}
impl AlignmentText<'_> {
    pub(crate) fn validate(&self, identity: StreamIdentity) -> Result<(), FuzzyAlignmentError> {
        match self {
            Self::Existing(document) => {
                document.validate_document(identity, document.source, &document.content_id)?
            }
            Self::Recognized(document) => {
                if document.identity() != identity {
                    return Err(ResolverError::InvalidData.into());
                }
                // The sealed ASR contract preserves original media timing,
                // order and bounded text. No synthetic subtitle publication.
                document
                    .validate()
                    .map_err(|_| ResolverError::InvalidData)?;
            }
        }
        Ok(())
    }
    pub(crate) fn origin(&self) -> FuzzyTextOrigin {
        match self {
            Self::Existing(_) => FuzzyTextOrigin::ExistingText,
            Self::Recognized(_) => FuzzyTextOrigin::PreparedRecognition,
        }
    }
    pub(crate) fn language(&self) -> Option<SpeechLanguage> {
        match self {
            Self::Existing(document) => match document.language {
                TextLanguage::Source(language) => Some(language),
                _ => None,
            },
            Self::Recognized(document) => Some(document.language()),
        }
    }
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Existing(document) => document.cues.len(),
            Self::Recognized(document) => document.cues().len(),
        }
    }
    pub(crate) fn cue(&self, index: usize) -> Option<AlignmentCue<'_>> {
        match self {
            Self::Existing(document) => document.cues.get(index).map(|cue| AlignmentCue {
                text: &cue.text,
                start_us: cue.start_us,
                duration_us: cue.duration_us,
            }),
            Self::Recognized(document) => document.cues().get(index).map(|cue| AlignmentCue {
                text: &cue.text,
                start_us: Some(cue.start_us),
                duration_us: Some(cue.duration_us),
            }),
        }
    }
}
