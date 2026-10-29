//! Synthetic private local files only; no decoder, real audio, keys or cloud.
use super::*;
use crate::{
    ai::speech::SpeechLanguage,
    media::{
        input::{LocalSubtitleInput, MediaInput},
        planning::MAX_SUBTITLE_CHOICES,
    },
    resolver::contracts::SubtitleSource,
    resolver::timed_subtitle::SubtitleFormat,
};
use std::{fs, path::PathBuf};
struct Fixture {
    directory: tempfile::TempDir,
    media: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("synthetic.media");
        fs::write(&media, b"opaque synthetic test source, not captured audio").unwrap();
        Self { directory, media }
    }
    fn subtitle(&self, name: &str, body: &str, language: TextLanguage) -> LocalSubtitleInput {
        let path = self.directory.path().join(name);
        fs::write(&path, body).unwrap();
        LocalSubtitleInput::new(path, SubtitleFormat::Srt, language).unwrap()
    }
    fn request(&self, inputs: Vec<LocalSubtitleInput>) -> PreparedRequest {
        PreparedRequest::new(MediaInput::local(self.media.clone()).unwrap(), inputs).unwrap()
    }
    fn chinese(&self) -> PreparedRequest {
        self.request(vec![self.subtitle(
            "source.srt",
            "1\n01:02:03,004 --> 01:02:05,006\n软件与服务器\n",
            TextLanguage::SimplifiedChinese,
        )])
    }
}
fn owner() -> PreparedSession {
    let mut owner = PreparedSession::new();
    owner.workers = Arc::new(Semaphore::new(1));
    owner
}
#[tokio::test]
async fn real_local_read_parse_and_taiwan_localization_publish_ram_ready_without_playing() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.media).unwrap();
    let mut owner = owner();
    let work = owner.start(fixture.chinese()).unwrap();
    assert_eq!(owner.state(), PreparedState::Preparing);
    assert_eq!(owner.app_state(), AppState::PreparingMedia);
    assert!(owner.ready().is_none());
    assert!(owner.apply(work.run().await).unwrap());
    assert_eq!(owner.state(), PreparedState::Ready);
    assert_eq!(owner.app_state(), AppState::PreparedReady);
    let document = owner.ready().unwrap();
    assert_eq!(document.identity(), owner.identity().unwrap());
    assert_eq!(
        document.original().existing().unwrap().source,
        SubtitleSource::ExistingSubtitle
    );
    assert_eq!(
        document.original().existing().unwrap().cues[0]
            .text
            .as_ref(),
        "软件与服务器"
    );
    assert!(document.translated()[0].contains("軟體"));
    assert!(document.translated()[0].contains("伺服器"));
    assert_eq!(document.target_language(), "zh-TW");
    assert_eq!(
        document.original().existing().unwrap().cues[0].start_us,
        Some(3_723_004_000)
    );
    assert_eq!(
        document.original().existing().unwrap().cues[0].duration_us,
        Some(2_002_000)
    );
    assert_eq!(document.media_byte_len(), before.len() as u64);
    assert_eq!(fs::read(&fixture.media).unwrap(), before);
    // Only caller-created input fixtures exist; product created no temp or output.
    assert_eq!(fs::read_dir(fixture.directory.path()).unwrap().count(), 2);
    owner.stop();
    assert!(owner.ready().is_none());
    assert!(owner.identity().is_none());
}
#[test]
fn priority_is_independent_of_input_order_and_does_not_infer_from_names() {
    let fixture = Fixture::new();
    let request = fixture.request(vec![
        fixture.subtitle(
            "zh-TW.srt",
            "",
            TextLanguage::Source(SpeechLanguage::English),
        ),
        fixture.subtitle("b.srt", "", TextLanguage::SimplifiedChinese),
        fixture.subtitle("c.srt", "", TextLanguage::OtherTraditional),
        fixture.subtitle("english.srt", "", TextLanguage::TaiwanTraditional),
    ]);
    assert_eq!(request.plan().unwrap(), PreparationPlan::Subtitle(3));
    assert_eq!(
        fixture.request(vec![]).plan().unwrap(),
        PreparationPlan::AsrRequired
    );
    assert_eq!(
        fixture
            .request(vec![
                fixture.subtitle("a.srt", "", TextLanguage::SimplifiedChinese),
                fixture.subtitle("b.srt", "", TextLanguage::OtherTraditional),
            ])
            .plan()
            .unwrap(),
        PreparationPlan::Subtitle(1)
    );
}
#[test]
fn ties_require_explicit_selection_and_input_count_is_bounded() {
    let fixture = Fixture::new();
    let request = fixture.request(vec![
        fixture.subtitle("one.srt", "", TextLanguage::TaiwanTraditional),
        fixture.subtitle("two.srt", "", TextLanguage::TaiwanTraditional),
    ]);
    assert_eq!(request.plan().err(), Some(MediaError::AmbiguousSubtitle));
    let choices = (0..=MAX_SUBTITLE_CHOICES)
        .map(|i| fixture.subtitle(&format!("{i}.srt"), "", TextLanguage::TaiwanTraditional))
        .collect();
    assert_eq!(
        PreparedRequest::new(MediaInput::local(fixture.media.clone()).unwrap(), choices).err(),
        Some(MediaError::TooLarge)
    );
}
#[tokio::test]
async fn chosen_invalid_native_subtitle_does_not_silently_fall_back_to_another_track() {
    let fixture = Fixture::new();
    let request = fixture.request(vec![
        fixture.subtitle(
            "valid.srt",
            "1\n00:00:00,000 --> 00:00:01,000\n软件\n",
            TextLanguage::SimplifiedChinese,
        ),
        fixture.subtitle(
            "chosen.srt",
            "invalid selected track",
            TextLanguage::TaiwanTraditional,
        ),
    ]);
    let mut owner = owner();
    let work = owner.start(request).unwrap();
    assert_eq!(
        owner.apply(work.run().await).err(),
        Some(MediaError::InvalidSubtitle)
    );
    assert_eq!(
        owner.state(),
        PreparedState::Failed(MediaError::InvalidSubtitle)
    );
    assert!(owner.ready().is_none());
}
#[tokio::test]
async fn unavailable_asr_translation_and_url_are_typed_errors_never_ready() {
    let fixture = Fixture::new();
    let requests = [
        (fixture.request(vec![]), MediaError::AsrUnavailable),
        (
            fixture.request(vec![fixture.subtitle(
                "source.srt",
                "",
                TextLanguage::Source(SpeechLanguage::English),
            )]),
            MediaError::TranslationRequired,
        ),
        (
            PreparedRequest::new(
                MediaInput::remote("https://example.invalid/media?signature=synthetic").unwrap(),
                vec![],
            )
            .unwrap(),
            MediaError::UnsupportedUrl,
        ),
    ];
    for (request, error) in requests {
        let mut owner = owner();
        let work = owner.start(request).unwrap();
        assert_eq!(owner.apply(work.run().await).err(), Some(error));
        assert_eq!(owner.state(), PreparedState::Failed(error));
        assert_eq!(owner.app_state(), AppState::Error);
        assert!(owner.ready().is_none());
    }
}
#[tokio::test]
async fn delayed_old_completion_cannot_replace_new_run_or_clear_its_ready_document() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let old_task = owner.start(fixture.chinese()).unwrap();
    let old_identity = owner.identity().unwrap();
    let old = old_task.run().await;
    let new_task = owner.start(fixture.chinese()).unwrap();
    assert_ne!(owner.identity().unwrap(), old_identity);
    owner.apply(new_task.run().await).unwrap();
    let new_identity = owner.ready().unwrap().identity();
    assert_eq!(owner.apply(old).err(), Some(MediaError::Stale));
    assert_eq!(owner.ready().unwrap().identity(), new_identity);
    assert_eq!(owner.state(), PreparedState::Ready);
}
#[tokio::test]
async fn replacing_unstarted_work_cancels_old_request_without_io() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let old = owner.start(fixture.chinese()).unwrap();
    let old_token = old.cancellation.clone();
    let current = owner.start(fixture.chinese()).unwrap();
    assert!(old_token.is_cancelled());
    assert_eq!(old.run().await.result.err(), Some(MediaError::Cancelled));
    owner.apply(current.run().await).unwrap();
    assert_eq!(owner.state(), PreparedState::Ready);
}
#[test]
fn task_drop_and_owner_drop_cancel_without_leaving_a_ready_document() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let task = owner.start(fixture.chinese()).unwrap();
    drop(task);
    assert_eq!(owner.state(), PreparedState::Cancelled);
    assert!(owner.ready().is_none());
    let task = owner.start(fixture.chinese()).unwrap();
    let token = task.cancellation.clone();
    drop(owner);
    assert!(token.is_cancelled());
}
#[tokio::test]
async fn caller_dropping_unpolled_run_future_cancels_owned_work() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let task = owner.start(fixture.chinese()).unwrap();
    let future = task.run();
    drop(future);
    assert_eq!(owner.state(), PreparedState::Cancelled);
    assert!(owner.ready().is_none());
}
#[tokio::test]
async fn stop_rejects_a_completed_but_unapplied_publication() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let task = owner.start(fixture.chinese()).unwrap();
    let completion = task.run().await;
    owner.stop();
    assert_eq!(owner.apply(completion).err(), Some(MediaError::Stale));
    assert!(owner.ready().is_none());
}
#[tokio::test]
async fn worker_capacity_has_no_queue_and_failure_can_restart() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let permit = owner.workers.clone().try_acquire_owned().unwrap();
    let work = owner.start(fixture.chinese()).unwrap();
    assert_eq!(
        owner.apply(work.run().await).err(),
        Some(MediaError::Overloaded)
    );
    drop(permit);
    let work = owner.start(fixture.chinese()).unwrap();
    owner.apply(work.run().await).unwrap();
    assert_eq!(owner.state(), PreparedState::Ready);
}
#[tokio::test]
async fn prepared_larger_than_live_window_is_processed_without_capture_freshness() {
    let fixture = Fixture::new();
    let body = (0..1100u64)
        .map(|i| {
            format!(
                "{}\n00:{:02}:{:02},000 --> 00:{:02}:{:02},500\n臺灣字幕\n\n",
                i + 1,
                i / 60,
                i % 60,
                i / 60,
                i % 60
            )
        })
        .collect::<String>();
    let mut owner = owner();
    let task = owner
        .start(fixture.request(vec![fixture.subtitle(
            "long.srt",
            &body,
            TextLanguage::TaiwanTraditional,
        )]))
        .unwrap();
    owner.apply(task.run().await).unwrap();
    assert_eq!(owner.ready().unwrap().translated().len(), 1100);
    assert_eq!(owner.ready().unwrap().cue_count(), 1100);
}
#[tokio::test]
async fn explicit_webvtt_preserves_original_timing_and_localizes_other_traditional() {
    let fixture = Fixture::new();
    let path = fixture.directory.path().join("declared.vtt");
    fs::write(&path, "WEBVTT\n\n01:02.000 --> 01:03.000\n軟件與服務器\n").unwrap();
    let subtitle =
        LocalSubtitleInput::new(path, SubtitleFormat::WebVtt, TextLanguage::OtherTraditional)
            .unwrap();
    let mut owner = owner();
    let task = owner.start(fixture.request(vec![subtitle])).unwrap();
    owner.apply(task.run().await).unwrap();
    let document = owner.ready().unwrap();
    assert_eq!(
        document.original().existing().unwrap().cues[0].start_us,
        Some(62_000_000)
    );
    assert!(document.translated()[0].contains("軟體"));
    assert!(document.translated()[0].contains("伺服器"));
}
#[test]
fn live_and_prepared_runs_share_one_unique_session_namespace() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let live_id = SessionId::allocate().unwrap();
    let first = owner.start(fixture.chinese()).unwrap();
    let prepared_id = owner.identity().unwrap().session;
    assert_ne!(live_id, prepared_id);
    drop(first);
    let next = SessionId::allocate().unwrap();
    assert_ne!(prepared_id, next);
    assert!(next.0 > prepared_id.0 && prepared_id.0 > live_id.0);
}

#[tokio::test]
async fn started_worker_keeps_capacity_after_future_drop_until_cooperative_exit() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let task = owner.start(fixture.chinese()).unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let release = Arc::new(std::sync::Barrier::new(2));
    let worker_release = release.clone();
    let mut future = Box::pin(task.run_with(move |_, _, _, cancel| {
        let _ = entered.send(());
        worker_release.wait();
        check(cancel)?;
        Err(MediaError::WorkerFailed)
    }));
    tokio::select! { biased;
        _ = ready => (),
        _ = &mut future => panic!("explicit mock worker completed before release"),
    }
    drop(future);
    assert_eq!(owner.state(), PreparedState::Cancelled);
    assert_eq!(owner.workers.available_permits(), 0);
    let rejected = owner.start(fixture.chinese()).unwrap();
    assert_eq!(
        owner.apply(rejected.run().await).err(),
        Some(MediaError::Overloaded)
    );
    release.wait();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while owner.workers.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(owner.workers.available_permits(), 1);
    let current = owner.start(fixture.chinese()).unwrap();
    owner.apply(current.run().await).unwrap();
    assert_eq!(owner.state(), PreparedState::Ready);
}

#[tokio::test]
async fn explicit_worker_panic_is_typed_and_releases_capacity_for_next_real_read() {
    let fixture = Fixture::new();
    let mut owner = owner();
    let task = owner.start(fixture.chinese()).unwrap();
    let failed = task
        .run_with(|_, _, _, _| panic!("explicit mock Prepared worker panic"))
        .await;
    assert_eq!(owner.apply(failed).err(), Some(MediaError::WorkerFailed));
    assert_eq!(owner.workers.available_permits(), 1);
    let current = owner.start(fixture.chinese()).unwrap();
    owner.apply(current.run().await).unwrap();
    assert_eq!(owner.state(), PreparedState::Ready);
}
