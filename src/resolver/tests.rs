//! Explicit synthetic fixtures only; no real music, media, keys or network.
use super::{candidates::*, contracts::*, session::*, window::*};
use crate::{
    ai::speech::StreamIdentity,
    audio::{
        capture::{AudioStamp, SessionId},
        resample::ResampledAudio,
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn identity(id: u64) -> StreamIdentity {
    StreamIdentity {
        session: SessionId(id),
        generation: 1,
    }
}
fn score(value: u16) -> Confidence {
    Confidence::new(value).unwrap()
}
fn song(id: &str, value: u16) -> SongCandidate {
    SongCandidate {
        id: id.into(),
        title: "synthetic title".into(),
        artist: None,
        confidence: score(value),
        match_offset_us: None,
    }
}
fn music(ids: &[(&str, u16)]) -> ContentEvidence {
    ContentEvidence {
        kind: ContentType::Music,
        confidence: score(900),
        songs: ids.iter().map(|(id, value)| song(id, *value)).collect(),
        movies: Vec::new(),
    }
}
fn stamp(sequence: u64, at: Instant) -> WindowStamp {
    WindowStamp {
        identity: identity(1),
        sequence,
        ended_at: at,
        start_us: None,
        duration_us: 12_000_000,
    }
}
fn window(sequence: u64) -> Arc<AudioWindow> {
    Arc::new(AudioWindow {
        stamp: stamp(sequence, Instant::now()),
        pcm: vec![0; WINDOW_FRAMES].into(),
    })
}
fn block(sequence: u64, position: Option<u64>, at: Instant) -> ResampledAudio {
    ResampledAudio {
        stamp: AudioStamp {
            session: SessionId(1),
            sequence,
            frame_position: position,
            qpc_100ns: None,
            received: at,
            discontinuity: false,
        },
        sample_rate: 16000,
        frames: 320,
        samples: [100; 480],
        first_input_offset: 0,
    }
}
#[test]
fn sliding_windows_preserve_pcm_order_interval_boundedness_and_unknown_clock() {
    let now = Instant::now();
    let mut assembler = SlidingAudioWindow::new(identity(1));
    let mut windows = Vec::new();
    for sequence in 0..1050 {
        let mut audio = block(sequence, None, now);
        audio.samples[..320].fill(sequence as i16);
        if let Some(window) = assembler.push(&audio, now).unwrap() {
            windows.push(window);
        }
        assert!(assembler.buffered_frames() <= WINDOW_FRAMES);
    }
    assert_eq!(windows.len(), 4); // first at 12 s, then 15/18/21 s
    for (index, window) in windows.iter().enumerate() {
        assert_eq!(window.stamp.sequence, 599 + index as u64 * 150);
        assert!(window.stamp.start_us.is_none());
        assert_eq!(window.samples().len(), WINDOW_FRAMES);
        assert_eq!(window.samples()[0], (index * 150) as i16);
        assert_eq!(
            window.samples()[WINDOW_FRAMES - 1],
            (599 + index * 150) as i16
        );
    }
}
#[test]
fn window_gap_staleness_foreign_session_and_bad_dimensions_cannot_mix_audio() {
    let now = Instant::now();
    let mut assembler = SlidingAudioWindow::new(identity(1));
    assembler.push(&block(0, Some(960), now), now).unwrap();
    assembler.push(&block(2, Some(2880), now), now).unwrap();
    assert_eq!(assembler.reset_count(), 1);
    assert_eq!(assembler.buffered_frames(), 320);
    let mut foreign = block(3, None, now);
    foreign.stamp.session = SessionId(2);
    assert!(matches!(
        assembler.push(&foreign, now),
        Err(ResolverError::InvalidData)
    ));
    let mut bad = block(3, None, now);
    bad.frames = 481;
    assert!(matches!(
        assembler.push(&bad, now),
        Err(ResolverError::InvalidData)
    ));
    assert!(matches!(
        assembler.push(&block(3, None, now - Duration::from_secs(1)), now),
        Err(ResolverError::Stale)
    ));
    assert_eq!(assembler.buffered_frames(), 0);
    assert!(assembler.push(&block(4, None, now), now).unwrap().is_none());
}
#[test]
fn known_window_clock_tracks_the_retained_head_and_native_discontinuity_resets() {
    let now = Instant::now();
    let mut assembler = SlidingAudioWindow::new(identity(1));
    let mut latest = None;
    for sequence in 0..750 {
        latest = assembler
            .push(&block(sequence, Some(960 + sequence * 960), now), now)
            .unwrap()
            .or(latest);
    }
    let latest = latest.unwrap();
    // First retained packet is seq 150; existing resampler subtracts 63 source frames.
    assert_eq!(
        latest.stamp.start_us,
        Some((960 + 150 * 960 - 63) * 1_000_000 / 48000)
    );
    let mut gap = block(750, Some(960 + 750 * 960), now);
    gap.stamp.discontinuity = true;
    assert!(assembler.push(&gap, now).unwrap().is_none());
    assert_eq!(assembler.buffered_frames(), 320);
}
#[test]
fn song_votes_require_three_windows_and_crossfade_does_not_force_a_match() {
    let now = Instant::now();
    let mut tracker = CandidateTracker::new(identity(1));
    assert_eq!(
        tracker
            .observe(stamp(1, now), music(&[("A", 900)]), now)
            .unwrap()
            .song_state,
        SongState::Candidate
    );
    assert_eq!(
        tracker
            .observe(stamp(2, now), music(&[("A", 900)]), now)
            .unwrap()
            .song_state,
        SongState::Candidate
    );
    assert_eq!(
        tracker
            .observe(stamp(3, now), music(&[("A", 900)]), now)
            .unwrap()
            .song_state,
        SongState::Confirmed
    );
    let transition = tracker
        .observe(stamp(4, now), music(&[("B", 900)]), now)
        .unwrap();
    assert_eq!(transition.kind, ContentType::Mixed);
    assert!(transition.song.is_none());
    let ambiguous = tracker
        .observe(stamp(5, now), music(&[("A", 900), ("B", 880)]), now)
        .unwrap();
    assert_eq!(ambiguous.song_state, SongState::Transition);
    assert!(ambiguous.song.is_none());
    tracker
        .observe(stamp(6, now), music(&[("B", 950)]), now)
        .unwrap();
    let adopted = tracker
        .observe(stamp(7, now), music(&[("B", 950)]), now)
        .unwrap();
    assert_eq!(adopted.song_state, SongState::Confirmed);
    assert_eq!(adopted.song.as_ref().unwrap().id.as_ref(), "B");
}

#[test]
fn phase11_repeated_medley_mixed_boundaries_require_new_votes_and_never_leak_the_previous_song() {
    let began = Instant::now();
    let mut tracker = CandidateTracker::new(identity(1));
    let mut sequence = 0;
    for transition in 0..120 {
        let id = format!("explicit-synthetic-song-{transition}");
        let now = began + Duration::from_secs(transition);
        for vote in 1..=3 {
            sequence += 1;
            let result = tracker
                .observe(stamp(sequence, now), music(&[(&id, 950)]), now)
                .unwrap();
            assert_eq!(
                result.song_state,
                if vote == 3 {
                    SongState::Confirmed
                } else {
                    SongState::Candidate
                }
            );
            assert_eq!(result.song.unwrap().id.as_ref(), id);
        }
        sequence += 1;
        let mut mixed = music(&[(&id, 950), ("another-synthetic-song", 940)]);
        mixed.kind = ContentType::Mixed;
        let result = tracker.observe(stamp(sequence, now), mixed, now).unwrap();
        assert_eq!(result.kind, ContentType::Mixed);
        assert_eq!(result.song_state, SongState::Transition);
        assert!(result.song.is_none());
    }
}
#[test]
fn weak_misses_expiry_and_speech_clear_song_without_reusing_old_session() {
    let now = Instant::now();
    let mut tracker = CandidateTracker::new(identity(1));
    for seq in 1..=3 {
        tracker
            .observe(stamp(seq, now), music(&[("A", 900)]), now)
            .unwrap();
    }
    assert_eq!(
        tracker
            .observe(stamp(4, now), music(&[("B", 400)]), now)
            .unwrap()
            .song_state,
        SongState::Confirmed
    );
    assert!(
        tracker
            .observe(stamp(5, now), music(&[]), now)
            .unwrap()
            .song
            .is_none()
    );
    let future = now + Duration::from_secs(16);
    assert_eq!(
        tracker
            .observe(stamp(6, future), music(&[("A", 900)]), future)
            .unwrap()
            .song_state,
        SongState::Candidate
    );
    let speech = ContentEvidence {
        kind: ContentType::Speech,
        confidence: score(900),
        songs: Vec::new(),
        movies: Vec::new(),
    };
    assert_eq!(
        tracker
            .observe(stamp(7, future), speech, future)
            .unwrap()
            .kind,
        ContentType::Speech
    );
    assert!(matches!(
        tracker.observe(stamp(6, future), music(&[("A", 900)]), future),
        Err(ResolverError::Stale)
    ));
    let mut foreign = stamp(8, future);
    foreign.identity = identity(2);
    assert!(matches!(
        tracker.observe(foreign, music(&[("A", 900)]), future),
        Err(ResolverError::InvalidData)
    ));
    assert_eq!(
        tracker
            .observe(stamp(8, future), music(&[("A", 900)]), future)
            .unwrap()
            .song_state,
        SongState::Candidate
    );
}
#[test]
fn contracts_reject_forged_scores_metadata_duplicates_and_oversized_fingerprint() {
    assert_eq!(Confidence::new(1001), Err(ResolverError::InvalidData));
    assert!(music(&[("A", 900), ("A", 900)]).validate().is_err());
    assert!(music(&[("private\0key", 900)]).validate().is_err());
    assert!(
        music(&[("A", 900), ("B", 900), ("C", 900), ("D", 900), ("E", 900)])
            .validate()
            .is_err()
    );
    assert!(Fingerprint::new(vec![0; MAX_FINGERPRINT_BYTES + 1].into()).is_err());
    assert!(Fingerprint::new(Vec::new().into()).is_err());
    assert_eq!(
        Fingerprint::new(vec![1, 2, 3].into()).unwrap().bytes(),
        &[1, 2, 3]
    );
    assert!(!ResolverError::Credential.retryable());
    assert!(ResolverError::Timeout.retryable());
    assert!(!ResolverError::InvalidData.to_string().contains("private"));
}
#[test]
fn movie_metadata_stays_a_bounded_candidate_and_does_not_adopt_subtitles() {
    let now = Instant::now();
    let movie = MovieCandidate {
        id: "movie-test".into(),
        title: "synthetic film".into(),
        year: Some(2000),
        confidence: score(950),
        signals: vec![
            MovieSignal::WindowTitle,
            MovieSignal::Dialogue,
            MovieSignal::RemoteMetadata,
        ],
    };
    let evidence = ContentEvidence {
        kind: ContentType::MovieOrShow,
        confidence: score(900),
        songs: Vec::new(),
        movies: vec![movie],
    };
    let mut tracker = CandidateTracker::new(identity(1));
    let result = tracker.observe(stamp(1, now), evidence, now).unwrap();
    assert_eq!(result.kind, ContentType::MovieOrShow);
    assert_eq!(result.movies.len(), 1);
    assert!(result.song.is_none());
    // It only returns metadata: no TranscriptTimeline or renderer is available here.
}
#[test]
fn existing_text_contract_bounds_timing_languages_and_session_without_disk_cache() {
    let query = TextQuery {
        stamp: stamp(1, Instant::now()),
        source: SubtitleSource::ExistingLyrics,
        content_id: "A".into(),
    };
    let mut text = ExistingText {
        identity: identity(1),
        source: SubtitleSource::ExistingLyrics,
        content_id: "A".into(),
        language: TextLanguage::SimplifiedChinese,
        cues: vec![TextCue {
            start_us: None,
            duration_us: None,
            text: "测试歌词".into(),
        }],
    };
    query.validate(Instant::now()).unwrap();
    text.validate(&query).unwrap();
    // Source text is retained as supplied; Phase 7 owns Taiwan localization.
    assert_eq!(text.cues[0].text.as_ref(), "测试歌词");
    text.identity = identity(2);
    assert!(text.validate(&query).is_err());
    text.identity = identity(1);
    text.cues[0].start_us = Some(0);
    assert!(text.validate(&query).is_err());
    text.cues[0].duration_us = Some(1);
    text.validate(&query).unwrap();
    text.cues[0].text = "a".repeat(MAX_CUE_BYTES + 1).into();
    assert!(text.validate(&query).is_err());
    let invalid = TextQuery {
        stamp: query.stamp,
        source: SubtitleSource::LiveAi,
        content_id: "A".into(),
    };
    assert!(invalid.validate(Instant::now()).is_err());
}

fn capabilities() -> ResolverCapabilities {
    ResolverCapabilities {
        classification: true,
        songs: true,
        movies: true,
        cancellation: true,
    }
}
struct MockIdentifier {
    calls: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    cancelled: Arc<Mutex<Vec<ResolverCancellation>>>,
    behavior: fn(u64) -> Option<Result<ContentEvidence, ResolverError>>,
}
struct ActiveGuard(Arc<AtomicUsize>);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl ContentIdentifier for MockIdentifier {
    fn capabilities(&self) -> ResolverCapabilities {
        capabilities()
    }
    fn identify<'a>(
        &'a self,
        window: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(self.active.fetch_add(1, Ordering::SeqCst), 0);
            let _active = ActiveGuard(self.active.clone());
            self.cancelled.lock().unwrap().push(cancel);
            match (self.behavior)(window.stamp.sequence) {
                Some(result) => result,
                None => std::future::pending().await,
            }
        })
    }
}
fn mock(behavior: fn(u64) -> Option<Result<ContentEvidence, ResolverError>>) -> MockIdentifier {
    MockIdentifier {
        calls: Arc::new(AtomicUsize::new(0)),
        active: Arc::new(AtomicUsize::new(0)),
        cancelled: Arc::new(Mutex::new(Vec::new())),
        behavior,
    }
}
async fn wait_calls(provider: &MockIdentifier, count: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while provider.calls.load(Ordering::SeqCst) < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn wait_result(
    session: &mut ResolverSession,
    predicate: impl Fn(&ResolverUpdate) -> bool,
) -> Arc<ResolverUpdate> {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(result) = session.poll()
                && predicate(&result)
            {
                return result;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn slow_identifier_is_superseded_and_does_not_block_independent_live_work() {
    let provider = Arc::new(mock(|seq| {
        if seq == 1 {
            None
        } else {
            Some(Ok(music(&[("A", 900)])))
        }
    }));
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let adapter = provider.clone();
    let job = tokio::spawn(async move { task.run(adapter.as_ref()).await });
    session.try_submit(window(1)).unwrap();
    wait_calls(&provider, 1).await;
    // Exercise the real SpeechSession boundary while the resolver never completes.
    use crate::{ai::speech::*, audio::preprocess::NormalizedAudio, domain::SourceLanguage};
    struct MockSpeech;
    impl SpeechRecognizer for MockSpeech {
        fn capabilities(&self) -> SpeechCapabilities {
            SpeechCapabilities {
                streaming: true,
                partials: true,
                cancellation: true,
                supported_languages: [true; 5],
                constrained_auto_detection: true,
            }
        }
        fn run<'a>(
            &'a self,
            _: SourceLanguage,
            mut audio: SpeechAudio,
            output: TranscriptOutput,
            cancel: Cancellation,
        ) -> SpeechFuture<'a> {
            Box::pin(async move {
                let input = audio.recv().await?;
                for (revision, kind) in [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)] {
                    output.emit(TranscriptEvent {
                        identity: identity(1),
                        sequence: input.stamp.sequence,
                        utterance: 1,
                        revision,
                        start_us: None,
                        duration_us: None,
                        language: Some(SpeechLanguage::English),
                        kind,
                        text: "synthetic independent STT".into(),
                    })?;
                }
                Err(cancel.cancelled().await)
            })
        }
    }
    let speech_provider = MockSpeech;
    let (mut speech, speech_task) = SpeechSession::new(
        identity(1),
        SourceLanguage::English,
        speech_provider.capabilities(),
    )
    .unwrap();
    let speech_job = tokio::spawn(async move { speech_task.run(&speech_provider).await });
    speech
        .input
        .try_push(NormalizedAudio {
            stamp: AudioStamp {
                session: SessionId(1),
                sequence: 1,
                frame_position: None,
                qpc_100ns: None,
                received: Instant::now(),
                discontinuity: false,
            },
            frames: 960,
            samples: [0.0; 960],
        })
        .unwrap();
    for expected in [TranscriptKind::Partial, TranscriptKind::Final] {
        let event = tokio::time::timeout(Duration::from_secs(1), speech.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.kind, expected);
    }
    assert_eq!(provider.active.load(Ordering::SeqCst), 1);
    session.try_submit(window(2)).unwrap();
    let result = wait_result(&mut session, |result| {
        result.status == ResolverStatus::Ready
    })
    .await;
    assert_eq!(result.stamp.sequence, 2);
    assert!(provider.cancelled.lock().unwrap()[0].is_cancelled());
    session.cancellation().cancel();
    job.await.unwrap().unwrap();
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(session.poll().unwrap().status, ResolverStatus::Stopped);
    assert!(!speech.cancellation().reason().is_some());
    speech.cancellation().cancel();
    assert_eq!(speech_job.await.unwrap(), Err(SpeechError::Cancelled));
}

#[tokio::test]
async fn request_timeout_cancels_remote_work_without_cancelling_session() {
    let provider = Arc::new(mock(|_| None));
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    session.try_submit(window(1)).unwrap();
    let adapter = provider.clone();
    let job = tokio::spawn(async move { task.run(adapter.as_ref()).await });
    let failure = tokio::time::timeout(REQUEST_TIMEOUT + Duration::from_secs(1), async {
        loop {
            if let Some(update) = session.poll()
                && matches!(update.status, ResolverStatus::Failed(_))
            {
                return update;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        failure.status,
        ResolverStatus::Failed(ResolverError::Timeout)
    );
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(provider.cancelled.lock().unwrap()[0].is_cancelled());
    assert!(!session.cancellation().is_cancelled());
    session.cancellation().cancel();
    job.await.unwrap().unwrap();
}

#[tokio::test]
async fn session_drop_and_precancel_release_requests_without_starting_another_provider() {
    let provider = Arc::new(mock(|_| None));
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let input = window(1);
    let weak = Arc::downgrade(&input);
    session.try_submit(input).unwrap();
    let adapter = provider.clone();
    let job = tokio::spawn(async move { task.run(adapter.as_ref()).await });
    wait_calls(&provider, 1).await;
    drop(session);
    job.await.unwrap().unwrap();
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(weak.upgrade().is_none());
    let (mut next, task) = ResolverSession::new(identity(2), capabilities()).unwrap();
    next.cancellation().cancel();
    assert_eq!(next.try_submit(window(2)), Err(ResolverError::Cancelled));
    task.run(provider.as_ref()).await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn queue_saturation_keeps_only_latest_window_and_slow_results_coalesce() {
    let provider = Arc::new(mock(|_| Some(Ok(music(&[("A", 900)])))));
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let mut weak = Vec::new();
    for seq in 1..=50 {
        let input = window(seq);
        weak.push(Arc::downgrade(&input));
        session.try_submit(input).unwrap();
    }
    assert_eq!(
        weak.iter().filter(|item| item.upgrade().is_some()).count(),
        1
    );
    let adapter = provider.clone();
    let job = tokio::spawn(async move { task.run(adapter.as_ref()).await });
    let first = wait_result(&mut session, |result| {
        result.status == ResolverStatus::Ready
    })
    .await;
    assert_eq!(first.stamp.sequence, 50);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    session.try_submit(window(51)).unwrap();
    wait_calls(&provider, 2).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    session.try_submit(window(52)).unwrap();
    wait_calls(&provider, 3).await;
    let latest = wait_result(&mut session, |result| {
        result.status == ResolverStatus::Ready
    })
    .await;
    assert_eq!(latest.stamp.sequence, 52);
    assert_eq!(latest.resolution.song_state, SongState::Confirmed);
    session.cancellation().cancel();
    job.await.unwrap().unwrap();
    assert!(weak.iter().all(|item| item.upgrade().is_none()));
}
#[tokio::test]
async fn provider_failure_stays_explicit_and_other_session_input_is_rejected() {
    let provider = Arc::new(mock(|_| Some(Err(ResolverError::Credential))));
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let mut foreign = window(1);
    Arc::get_mut(&mut foreign).unwrap().stamp.identity = identity(2);
    assert_eq!(session.try_submit(foreign), Err(ResolverError::InvalidData));
    session.try_submit(window(1)).unwrap();
    let adapter = provider.clone();
    let job = tokio::spawn(async move { task.run(adapter.as_ref()).await });
    let failure = wait_result(&mut session, |update| {
        matches!(update.status, ResolverStatus::Failed(_))
    })
    .await;
    assert_eq!(
        failure.status,
        ResolverStatus::Failed(ResolverError::Credential)
    );
    assert!(failure.resolution.song.is_none());
    assert!(!session.cancellation().is_cancelled());
    session.cancellation().cancel();
    job.await.unwrap().unwrap();
}
#[tokio::test]
async fn dropping_worker_future_cancels_request_and_releases_audio() {
    let provider = mock(|_| None);
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let input = window(1);
    let weak = Arc::downgrade(&input);
    session.try_submit(input).unwrap();
    let mut future = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(future.as_mut()).is_pending());
    assert_eq!(provider.active.load(Ordering::SeqCst), 1);
    drop(future);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(weak.upgrade().is_none());
    assert!(provider.cancelled.lock().unwrap()[0].is_cancelled());
    assert_eq!(session.try_submit(window(2)), Err(ResolverError::Cancelled));
}

#[tokio::test]
async fn audio_invalidation_cancels_request_before_a_replacement_window_exists() {
    let provider = mock(|_| None);
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let input = window(1);
    let weak = Arc::downgrade(&input);
    session.try_submit(input).unwrap();
    let mut job = Box::pin(task.run(&provider));
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    session.invalidate().unwrap();
    let reset = session.poll().unwrap();
    assert_eq!(
        reset.status,
        ResolverStatus::Failed(ResolverError::AudioDiscontinuity)
    );
    assert!(reset.resolution.song.is_none());
    assert!(provider.cancelled.lock().unwrap()[0].is_cancelled());
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    assert!(session.poll().is_none()); // The owner already delivered the reset once.
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert!(provider.cancelled.lock().unwrap()[0].is_cancelled());
    assert!(weak.upgrade().is_none());
    assert!(!session.cancellation().is_cancelled());
    drop(job);
}

#[tokio::test]
async fn coalesced_reset_and_new_window_cannot_reuse_confirmed_votes() {
    let provider = mock(|_| Some(Ok(music(&[("A", 900)]))));
    let (mut session, task) = ResolverSession::new(identity(1), capabilities()).unwrap();
    let mut job = Box::pin(task.run(&provider));
    for seq in 1..=3 {
        session.try_submit(window(seq)).unwrap();
        assert!(futures_util::poll!(job.as_mut()).is_pending());
        let result = session.next().await.unwrap();
        assert_eq!(result.status, ResolverStatus::Ready);
        if seq == 3 {
            assert_eq!(result.resolution.song_state, SongState::Confirmed);
        }
    }
    session.invalidate().unwrap();
    session.invalidate().unwrap();
    session.try_submit(window(4)).unwrap(); // Replaces both reset notices before polling.
    assert!(session.poll().is_none());
    assert!(futures_util::poll!(job.as_mut()).is_pending());
    let result = session.next().await.unwrap();
    assert_eq!(result.stamp.sequence, 4);
    assert_eq!(result.resolution.song_state, SongState::Candidate);
    assert_eq!(result.resolution.song.as_ref().unwrap().id.as_ref(), "A");
    assert_eq!(session.try_submit(window(3)), Err(ResolverError::Stale));
    drop(job);
}
