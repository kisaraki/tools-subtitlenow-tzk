//! Explicit per-run text choices, not filename/language inference or discovery.
use super::{
    MediaError,
    embedded_mp4::Mp4SubtitleSelection,
    input::{LocalSubtitleInput, MediaInput},
    remote_subtitle::PreparedSubtitleUrl,
};
use crate::resolver::contracts::{MAX_METADATA_BYTES, TextLanguage};

/// Bounded explicit choices; no subtitle database or persistent inventory.
pub const MAX_SUBTITLE_CHOICES: usize = 8;
pub struct PreparedRequest {
    pub(crate) media: MediaInput,
    pub(crate) subtitles: Vec<LocalSubtitleInput>,
    pub(crate) embedded: Option<Mp4SubtitleSelection>,
    pub(crate) remote_subtitles: Vec<PreparedSubtitleUrl>,
    pub(crate) reference: Option<PreparedSourceReference>,
}
/// Explicit same-edition assertion for one local source-language reference.
/// Never inferred from names, paths, language, URLs or matching durations.
pub(crate) struct PreparedSourceReference {
    primary: PreparationPlan,
    pub(crate) input: LocalSubtitleInput,
    pub(crate) edition: Box<str>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationPlan {
    Subtitle(usize),
    EmbeddedMp4,
    RemoteSubtitle(usize),
    AsrRequired,
}
impl PreparedRequest {
    pub fn new(media: MediaInput, subtitles: Vec<LocalSubtitleInput>) -> Result<Self, MediaError> {
        if subtitles.len() > MAX_SUBTITLE_CHOICES {
            return Err(MediaError::TooLarge);
        }
        Ok(Self {
            media,
            subtitles,
            embedded: None,
            remote_subtitles: Vec::new(),
            reference: None,
        })
    }
    /// One explicit embedded track can compete with sidecars under the same
    /// language priority. Equal ranks require the caller to remove ambiguity.
    pub fn with_embedded_subtitle(
        mut self,
        selection: Mp4SubtitleSelection,
    ) -> Result<Self, MediaError> {
        if self.embedded.is_some() {
            return Err(MediaError::AmbiguousSubtitle);
        }
        if self.choice_count() == MAX_SUBTITLE_CHOICES {
            return Err(MediaError::TooLarge);
        }
        self.embedded = Some(selection);
        Ok(self)
    }
    /// Explicit HTTPS choices share the existing language ranking and total
    /// budget. Construction never fetches text or grants a media association.
    pub fn with_remote_subtitle(
        mut self,
        subtitle: PreparedSubtitleUrl,
    ) -> Result<Self, MediaError> {
        if self.choice_count() == MAX_SUBTITLE_CHOICES {
            return Err(MediaError::TooLarge);
        }
        self.remote_subtitles.push(subtitle);
        Ok(self)
    }
    fn choice_count(&self) -> usize {
        self.subtitles.len()
            + self.remote_subtitles.len()
            + usize::from(self.embedded.is_some())
            + usize::from(self.reference.is_some())
    }
    pub fn with_source_reference(
        mut self,
        input: LocalSubtitleInput,
        edition: &str,
    ) -> Result<Self, MediaError> {
        if self.choice_count() >= MAX_SUBTITLE_CHOICES {
            return Err(MediaError::TooLarge);
        }
        if self.reference.is_some()
            || !matches!(input.language, TextLanguage::Source(_))
            || !matches!(
                self.text_language()?,
                Some(
                    TextLanguage::TaiwanTraditional
                        | TextLanguage::OtherTraditional
                        | TextLanguage::SimplifiedChinese
                )
            )
            || edition.trim().is_empty()
            || edition.len() > MAX_METADATA_BYTES
            || edition.chars().any(char::is_control)
        {
            return Err(MediaError::InvalidSubtitle);
        }
        self.reference = Some(PreparedSourceReference {
            primary: self.plan()?,
            input,
            edition: edition.into(),
        });
        Ok(self)
    }
    /// Metadata-only service planning; never opens media or reads a credential.
    pub fn text_language(&self) -> Result<Option<TextLanguage>, MediaError> {
        match self.plan()? {
            PreparationPlan::Subtitle(index) => self
                .subtitles
                .get(index)
                .map(|subtitle| Some(subtitle.language))
                .ok_or(MediaError::InvalidInput),
            PreparationPlan::EmbeddedMp4 => self
                .embedded
                .map(|subtitle| Some(subtitle.language()))
                .ok_or(MediaError::InvalidInput),
            PreparationPlan::RemoteSubtitle(index) => self
                .remote_subtitles
                .get(index)
                .map(|subtitle| Some(subtitle.language()))
                .ok_or(MediaError::InvalidInput),
            PreparationPlan::AsrRequired => Ok(None),
        }
    }
    /// Plan only. A chosen document must still be read, parsed and validated;
    /// its declared association/language is not an independent media match.
    pub fn plan(&self) -> Result<PreparationPlan, MediaError> {
        let mut best = None;
        let mut ties = 0;
        let choices = self
            .subtitles
            .iter()
            .enumerate()
            .map(|(index, subtitle)| (PreparationPlan::Subtitle(index), subtitle.language))
            .chain(
                self.embedded
                    .map(|subtitle| (PreparationPlan::EmbeddedMp4, subtitle.language())),
            );
        let choices = choices.chain(self.remote_subtitles.iter().enumerate().map(
            |(index, subtitle)| (PreparationPlan::RemoteSubtitle(index), subtitle.language()),
        ));
        for (plan, language) in choices {
            let rank = match language {
                TextLanguage::TaiwanTraditional => 0,
                TextLanguage::OtherTraditional => 1,
                TextLanguage::SimplifiedChinese => 2,
                TextLanguage::Source(_) => 3,
            };
            match best {
                Some((_, previous)) if previous < rank => {}
                Some((_, previous)) if previous == rank => ties += 1,
                _ => {
                    best = Some((plan, rank));
                    ties = 1;
                }
            }
        }
        if ties > 1 {
            return Err(MediaError::AmbiguousSubtitle);
        }
        let selected = best.map_or(PreparationPlan::AsrRequired, |(plan, _)| plan);
        if self
            .reference
            .as_ref()
            .is_some_and(|reference| reference.primary != selected)
        {
            return Err(MediaError::InvalidSubtitle);
        }
        Ok(selected)
    }
}

#[cfg(test)]
#[path = "planning_remote_tests.rs"]
mod remote_tests;
