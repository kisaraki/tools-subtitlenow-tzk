//! Owned synthetic MP4 structures/text only; no playable codec, key or cloud.
use super::*;
use crate::{
    ai::{
        speech::SpeechLanguage,
        translation::{
            TranslationCancellation, TranslationCapabilities, TranslationEmitter,
            TranslationFuture, TranslationRequest,
        },
    },
    media::{
        embedded_mp4::Mp4SubtitleSelection,
        input::{LocalSubtitleInput, MediaInput},
        local_audio_format::mp4_prepared_fixture,
        planning::MAX_SUBTITLE_CHOICES,
    },
    resolver::timed_subtitle::SubtitleFormat,
};
use std::{fs, path::PathBuf};
struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn new(language: TextLanguage) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic-embedded.data");
        fs::write(&path, mp4_prepared_fixture(language)).unwrap();
        Self {
            _directory: directory,
            path,
        }
    }
    fn media(&self) -> MediaInput {
        MediaInput::local(self.path.clone()).unwrap()
    }
    fn request(&self, language: TextLanguage) -> PreparedRequest {
        PreparedRequest::new(self.media(), vec![])
            .unwrap()
            .with_embedded_subtitle(Mp4SubtitleSelection::new(3, language).unwrap())
            .unwrap()
    }
    fn sidecar(&self, index: usize, language: TextLanguage) -> LocalSubtitleInput {
        LocalSubtitleInput::new(
            self._directory.path().join(format!("unread-{index}.srt")),
            SubtitleFormat::Srt,
            language,
        )
        .unwrap()
    }
}
fn owner() -> PreparedSession {
    let mut owner = PreparedSession::new();
    owner.workers = Arc::new(Semaphore::new(1));
    owner
}
struct NoCloud;
struct NoSpeech;
impl PreparedSpeechRecognizer for NoSpeech {
    fn capabilities(&self) -> crate::ai::prepared_speech::PreparedSpeechCapabilities {
        panic!("embedded text must not inspect speech capabilities")
    }
    fn recognize<'a>(
        &'a self,
        _: &'a crate::ai::prepared_speech::PreparedSpeechRequest,
        _: ResolverCancellation,
    ) -> crate::ai::prepared_speech::PreparedSpeechFuture<'a> {
        panic!("embedded text must not send audio to ASR")
    }
}
impl Translator for NoCloud {
    fn capabilities(&self) -> TranslationCapabilities {
        panic!("Chinese must not inspect provider capabilities")
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        _: &'a mut TranslationEmitter,
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        panic!("must not translate Chinese")
    }
}
#[test]
fn embedded_prepared_plan_shares_priority_with_sidecars_and_rejects_ties() {
    let fixture = Fixture::new(TextLanguage::TaiwanTraditional);
    let languages = [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
        TextLanguage::Source(SpeechLanguage::English),
    ];
    for (embedded_rank, embedded) in languages.iter().copied().enumerate() {
        for (sidecar_rank, sidecar) in languages.iter().copied().enumerate() {
            let request = PreparedRequest::new(fixture.media(), vec![fixture.sidecar(0, sidecar)])
                .unwrap()
                .with_embedded_subtitle(Mp4SubtitleSelection::new(3, embedded).unwrap())
                .unwrap();
            if embedded_rank == sidecar_rank {
                assert_eq!(request.plan(), Err(MediaError::AmbiguousSubtitle));
                assert!(matches!(
                    request.text_language(),
                    Err(MediaError::AmbiguousSubtitle)
                ));
            } else {
                assert_eq!(
                    request.plan(),
                    Ok(if embedded_rank < sidecar_rank {
                        PreparationPlan::EmbeddedMp4
                    } else {
                        PreparationPlan::Subtitle(0)
                    })
                );
                assert!(
                    request.text_language().unwrap()
                        == Some(if embedded_rank < sidecar_rank {
                            embedded
                        } else {
                            sidecar
                        })
                );
            }
        }
    }
}
#[test]
fn embedded_prepared_total_choice_budget_includes_the_embedded_selection() {
    let fixture = Fixture::new(TextLanguage::TaiwanTraditional);
    for (count, valid) in [
        (MAX_SUBTITLE_CHOICES - 1, true),
        (MAX_SUBTITLE_CHOICES, false),
    ] {
        let sidecars = (0..count)
            .map(|i| fixture.sidecar(i, TextLanguage::Source(SpeechLanguage::English)))
            .collect();
        let result = PreparedRequest::new(fixture.media(), sidecars)
            .unwrap()
            .with_embedded_subtitle(
                Mp4SubtitleSelection::new(3, TextLanguage::TaiwanTraditional).unwrap(),
            );
        if valid {
            assert_eq!(result.unwrap().plan(), Ok(PreparationPlan::EmbeddedMp4));
        } else {
            assert!(matches!(result, Err(MediaError::TooLarge)));
        }
    }
    assert!(matches!(
        fixture
            .request(TextLanguage::TaiwanTraditional)
            .with_embedded_subtitle(
                Mp4SubtitleSelection::new(4, TextLanguage::TaiwanTraditional).unwrap()
            ),
        Err(MediaError::AmbiguousSubtitle)
    ));
}
#[tokio::test]
async fn embedded_prepared_chinese_ready_uses_one_worker_and_keeps_pin_timing_and_original() {
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        let fixture = Fixture::new(language);
        let before = fs::read(&fixture.path).unwrap();
        let mut owner = owner();
        let completion = owner
            .start(fixture.request(language))
            .unwrap()
            .run_with_translator(&NoCloud, TranslationQuality::High, None)
            .await;
        assert!(owner.apply(completion).unwrap());
        let doc = owner.ready().unwrap();
        let original = doc.original().existing().unwrap();
        assert!(original.language == language);
        assert_eq!(original.identity, owner.identity().unwrap());
        assert_eq!(original.cues[0].text.as_ref(), "软件与内存");
        assert_eq!(doc.translated()[0].as_ref(), "軟體與記憶體");
        assert_eq!(doc.translated()[1].as_ref(), "滑鼠與伺服器");
        assert_eq!(original.cues[0].duration_us, Some(700_000));
        assert_eq!(original.cues[1].start_us, Some(1_100_000));
        assert_eq!(original.cues[1].duration_us, Some(1_000_000));
        assert_eq!(owner.app_state(), AppState::PreparedReady);
        assert_eq!(doc.media_byte_len(), before.len() as u64);
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.path)
                .is_err()
        );
        assert!(fs::remove_file(&fixture.path).is_err());
        assert_eq!(fs::read(&fixture.path).unwrap(), before);
        assert_eq!(fs::read_dir(fixture._directory.path()).unwrap().count(), 1);
        owner.stop();
        assert!(owner.ready().is_none());
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(&fixture.path)
                .is_ok()
        );
    }
}
#[tokio::test]
async fn embedded_prepared_missing_track_never_falls_back_to_sidecar_or_asr() {
    let fixture = Fixture::new(TextLanguage::TaiwanTraditional);
    let mut owner = owner();
    let request = PreparedRequest::new(
        fixture.media(),
        vec![fixture.sidecar(0, TextLanguage::Source(SpeechLanguage::English))],
    )
    .unwrap()
    .with_embedded_subtitle(Mp4SubtitleSelection::new(4, TextLanguage::TaiwanTraditional).unwrap())
    .unwrap();
    let completion = owner
        .start(request)
        .unwrap()
        .run_with_translator(&NoCloud, TranslationQuality::Standard, None)
        .await;
    assert_eq!(owner.apply(completion), Err(MediaError::UnsupportedMedia));
    assert!(owner.ready().is_none());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
}
#[tokio::test]
async fn embedded_prepared_source_requires_explicit_translation_without_publication() {
    let fixture = Fixture::new(TextLanguage::Source(SpeechLanguage::English));
    let mut owner = owner();
    let completion = owner
        .start(fixture.request(TextLanguage::Source(SpeechLanguage::English)))
        .unwrap()
        .run()
        .await;
    assert_eq!(
        owner.apply(completion),
        Err(MediaError::TranslationRequired)
    );
    assert!(owner.ready().is_none());
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
}
#[tokio::test]
async fn embedded_prepared_late_completion_cannot_replace_new_ready_and_stop_releases_pin() {
    let fixture = Fixture::new(TextLanguage::TaiwanTraditional);
    let mut owner = owner();
    let old = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap()
        .run()
        .await;
    let old_id = old.identity;
    owner.stop();
    let new = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap()
        .run()
        .await;
    assert!(owner.apply(new).unwrap());
    let new_id = owner.identity().unwrap();
    assert_ne!(new_id, old_id);
    assert_eq!(owner.apply(old), Err(MediaError::Stale));
    assert_eq!(owner.ready().unwrap().identity(), new_id);
    owner.stop();
    assert!(
        fs::OpenOptions::new()
            .write(true)
            .open(&fixture.path)
            .is_ok()
    );
}
#[tokio::test]
async fn embedded_prepared_saturated_worker_can_recover_without_a_second_decoder_lane() {
    let fixture = Fixture::new(TextLanguage::TaiwanTraditional);
    let mut owner = owner();
    let permit = owner.workers.clone().acquire_owned().await.unwrap();
    let completion = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap()
        .run()
        .await;
    assert_eq!(owner.apply(completion), Err(MediaError::Overloaded));
    drop(permit);
    let completion = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap()
        .run()
        .await;
    assert!(owner.apply(completion).unwrap());
    assert_eq!(owner.workers.available_permits(), 1);
}

#[tokio::test]
async fn embedded_prepared_text_preference_bypasses_supplied_recognizer_even_with_audio_auto() {
    let fixture = Fixture::new(TextLanguage::TaiwanTraditional);
    let mut owner = owner();
    let completion = owner
        .start(fixture.request(TextLanguage::TaiwanTraditional))
        .unwrap()
        .run_with_recognizer(
            &NoSpeech,
            &NoCloud,
            SourceLanguage::Auto,
            PreparedSampleRate::Hz16000,
            TranslationQuality::Standard,
            None,
        )
        .await;
    assert!(owner.apply(completion).unwrap());
    assert_eq!(owner.ready().unwrap().cue_count(), 2);
}
