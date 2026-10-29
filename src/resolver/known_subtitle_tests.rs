//! Explicit synthetic source text and literal-loopback transport only.
use super::*;
use crate::{
    audio::capture::SessionId,
    resolver::{movie::MovieMetadataResolver, timed_subtitle::SubtitleFormat},
};
use std::{sync::atomic::AtomicUsize, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const TITLE: &str = "Synthetic Lantern";
const CUE: &str = "THE synthetic lantern, will remain beside this empty doorway.";
const DIALOGUE: &str = "the synthetic lantern will remain beside this empty doorway";
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(1141),
        generation: 3,
    }
}
fn movie(sequence: u64) -> MovieQuery {
    MovieQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: Some(40_000_000),
            duration_us: 12_000_000,
        },
        window_title: Some(format!("{TITLE} - Synthetic Player").into()),
        filename: None,
        dialogue: Some(DIALOGUE.into()),
        language: Some(SpeechLanguage::English),
        year: None,
        duration_us: None,
    }
}
fn text(owner: &SessionKnownSubtitle, sequence: u64) -> TextQuery {
    TextQuery {
        stamp: movie(sequence).stamp,
        source: SubtitleSource::ExistingSubtitle,
        content_id: owner.content_id().into(),
    }
}
fn document(query: &TextQuery, cues: &[&str]) -> ExistingTextBundle {
    ExistingTextBundle {
        text: ExistingText {
            identity: query.stamp.identity,
            source: query.source,
            content_id: query.content_id.clone(),
            language: TextLanguage::Source(SpeechLanguage::English),
            cues: cues
                .iter()
                .enumerate()
                .map(|(index, cue)| TextCue {
                    start_us: Some(index as u64 * 2_000_000),
                    duration_us: Some(1_000_000),
                    text: (*cue).into(),
                })
                .collect(),
        },
        source_reference: None,
    }
}
#[derive(Clone, Copy)]
enum Behavior {
    Ready,
    Duplicate,
    LongDuplicate,
    Missing,
    PanicOnce,
    PendingOnce,
    BadOnce,
}
struct MockProvider {
    calls: AtomicUsize,
    behavior: Behavior,
    entered: tokio::sync::Notify,
}
impl ExistingTextProvider for MockProvider {
    fn supports(&self, source: SubtitleSource) -> bool {
        source == SubtitleSource::ExistingSubtitle
    }
    fn lookup<'a>(&'a self, query: &'a TextQuery, cancel: ResolverCancellation) -> TextFuture<'a> {
        Box::pin(async move {
            Ok(self
                .lookup_bundle(query, cancel)
                .await?
                .map(|bundle| bundle.text))
        })
    }
    fn lookup_bundle<'a>(
        &'a self,
        query: &'a TextQuery,
        _cancel: ResolverCancellation,
    ) -> TextBundleFuture<'a> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            match self.behavior {
                Behavior::PanicOnce if call == 0 => panic!("synthetic provider failure"),
                Behavior::PendingOnce if call == 0 => std::future::pending::<()>().await,
                Behavior::Missing => return Ok(None),
                _ => (),
            }
            let mut bundle = if matches!(self.behavior, Behavior::Duplicate) {
                document(query, &[CUE, CUE])
            } else if matches!(self.behavior, Behavior::LongDuplicate) {
                document(
                    query,
                    &[CUE, &format!("{}{}", " ".repeat(MAX_DIALOGUE_BYTES), CUE)],
                )
            } else {
                document(query, &[CUE])
            };
            if matches!(self.behavior, Behavior::BadOnce) && call == 0 {
                bundle.text.language = TextLanguage::TaiwanTraditional;
            }
            Ok(Some(bundle))
        })
    }
}
fn mock(behavior: Behavior) -> (SessionKnownSubtitle, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider {
        calls: AtomicUsize::new(0),
        behavior,
        entered: tokio::sync::Notify::new(),
    });
    (
        SessionKnownSubtitle::build(
            identity(),
            TITLE.into(),
            SpeechLanguage::English,
            content_id(identity()).unwrap(),
            provider.clone(),
            Arc::new(Semaphore::new(1)),
        ),
        provider,
    )
}
fn track(language: TextLanguage) -> SubtitleTrack {
    SubtitleTrack::new(
        "https://example.invalid/source.srt?ticket=synthetic",
        SubtitleFormat::Srt,
        language,
    )
    .unwrap()
}

#[test]
fn explicit_title_and_source_language_only_with_unique_opaque_ids() {
    for language in [
        TextLanguage::TaiwanTraditional,
        TextLanguage::OtherTraditional,
        TextLanguage::SimplifiedChinese,
    ] {
        assert_eq!(
            KnownSubtitleSource::new(TITLE.into(), track(language)).err(),
            Some(ResolverError::InvalidData)
        );
        assert_eq!(
            SessionKnownSubtitle::new(identity(), TITLE.into(), track(language)).err(),
            Some(ResolverError::InvalidData)
        );
    }
    for title in [
        "",
        " ",
        "synthetic\nprivate",
        &"x".repeat(MAX_METADATA_BYTES + 1),
    ] {
        assert!(
            KnownSubtitleSource::new(
                title.into(),
                track(TextLanguage::Source(SpeechLanguage::English))
            )
            .is_err()
        );
    }
    let source = KnownSubtitleSource::new(
        TITLE.into(),
        track(TextLanguage::Source(SpeechLanguage::English)),
    )
    .unwrap();
    assert_eq!(source.title(), TITLE);
    assert_eq!(source.language(), SpeechLanguage::English);
    let first = source.start(identity()).unwrap();
    let second = source.start(identity()).unwrap();
    assert_ne!(first.content_id(), second.content_id());
    assert!(!first.content_id().contains(TITLE));
    assert!(!first.content_id().contains("ticket"));
}

#[tokio::test]
async fn catalog_uses_unique_full_downloaded_cue_and_preserves_original_reference() {
    let (owner, provider) = mock(Behavior::Ready);
    let catalog = owner.catalog();
    let reply = catalog
        .lookup(&movie(1), ResolverCancellation::default())
        .await
        .unwrap();
    assert_eq!(reply.records.len(), 1);
    let record = &reply.records[0];
    assert_eq!(record.id.as_ref(), owner.content_id());
    assert_eq!(record.title.as_ref(), TITLE);
    assert_eq!(record.confidence.permille(), 1000);
    assert_eq!(record.dialogue_references[0].as_ref(), CUE);
    assert_ne!(record.dialogue_references[0].as_ref(), DIALOGUE);
    assert_eq!(record.language, Some(SpeechLanguage::English));
    assert!(record.year.is_none() && record.duration_us.is_none() && record.aliases.is_empty());
    let first = owner.provider();
    let second = owner.provider();
    assert!(Arc::ptr_eq(&first, &second));
    let bundle = first
        .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bundle.text.cues[0].text.as_ref(), CUE);
    assert!(bundle.source_reference.is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ambiguous_short_partial_or_different_tokens_do_not_identify() {
    let (owner, _) = mock(Behavior::Duplicate);
    assert!(
        owner
            .catalog()
            .lookup(&movie(1), ResolverCancellation::default())
            .await
            .unwrap()
            .records
            .is_empty()
    );
    let (owner, provider) = mock(Behavior::Ready);
    for (sequence, dialogue) in [
        "short",
        "the synthetic lantern will remain beside this empty",
        "the synthetic lantern will remain beside this empty doorway additional",
        "the synthetic lantern will remainer beside this empty doorway",
    ]
    .into_iter()
    .enumerate()
    {
        let mut query = movie(sequence as u64 + 1);
        query.dialogue = Some(dialogue.into());
        assert!(
            owner
                .catalog()
                .lookup(&query, ResolverCancellation::default())
                .await
                .unwrap()
                .records
                .is_empty()
        );
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn oversized_raw_duplicate_still_blocks_unique_identification() {
    let (owner, _) = mock(Behavior::LongDuplicate);
    assert!(
        owner
            .catalog()
            .lookup(&movie(1), ResolverCancellation::default())
            .await
            .unwrap()
            .records
            .is_empty()
    );
}

#[tokio::test]
async fn resolver_keeps_independent_caption_language_year_and_runtime_gates() {
    let (owner, _) = mock(Behavior::Ready);
    let resolver = MovieMetadataResolver::new(identity(), owner.catalog()).unwrap();
    assert_eq!(
        resolver
            .resolve(&movie(1), ResolverCancellation::default())
            .await
            .unwrap()
            .kind,
        ContentType::MovieOrShow
    );
    let mut wrong = movie(2);
    wrong.window_title = Some("Another Synthetic Work".into());
    assert_eq!(
        resolver
            .resolve(&wrong, ResolverCancellation::default())
            .await
            .unwrap()
            .kind,
        ContentType::Unknown
    );
    let mut wrong = movie(3);
    wrong.language = Some(SpeechLanguage::Japanese);
    assert_eq!(
        resolver
            .resolve(&wrong, ResolverCancellation::default())
            .await
            .unwrap()
            .kind,
        ContentType::Unknown
    );
    let mut wrong = movie(4);
    wrong.year = Some(2020);
    assert_eq!(
        resolver
            .resolve(&wrong, ResolverCancellation::default())
            .await
            .unwrap()
            .kind,
        ContentType::Unknown
    );
    let mut wrong = movie(5);
    wrong.duration_us = Some(100_000_000);
    assert_eq!(
        resolver
            .resolve(&wrong, ResolverCancellation::default())
            .await
            .unwrap()
            .kind,
        ContentType::Unknown
    );
}

#[tokio::test]
async fn foreign_session_content_and_source_are_rejected_before_io() {
    let (owner, provider) = mock(Behavior::Ready);
    let handle = owner.provider();
    let mut query = text(&owner, 1);
    query.stamp.identity.generation += 1;
    assert_eq!(
        handle
            .lookup_bundle(&query, ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::InvalidData)
    );
    let mut query = text(&owner, 1);
    query.content_id = "different synthetic work".into();
    assert_eq!(
        handle
            .lookup_bundle(&query, ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::ContentMismatch)
    );
    let mut query = text(&owner, 1);
    query.source = SubtitleSource::ExistingLyrics;
    assert_eq!(
        handle
            .lookup_bundle(&query, ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::UnsupportedCapability)
    );
    let mut query = movie(1);
    query.stamp.identity.session = SessionId(1142);
    assert_eq!(
        owner
            .catalog()
            .lookup(&query, ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::InvalidData)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn no_dialogue_unknown_language_or_already_cancelled_request_does_no_io() {
    let (owner, provider) = mock(Behavior::Ready);
    let mut query = movie(1);
    query.dialogue = None;
    assert!(
        owner
            .catalog()
            .lookup(&query, ResolverCancellation::default())
            .await
            .unwrap()
            .records
            .is_empty()
    );
    let mut query = movie(2);
    query.language = Some(SpeechLanguage::Korean);
    assert!(
        owner
            .catalog()
            .lookup(&query, ResolverCancellation::default())
            .await
            .unwrap()
            .records
            .is_empty()
    );
    let mut query = movie(3);
    query.language = None;
    assert!(
        owner
            .catalog()
            .lookup(&query, ResolverCancellation::default())
            .await
            .unwrap()
            .records
            .is_empty()
    );
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    assert_eq!(
        owner
            .provider()
            .lookup_bundle(&text(&owner, 3), cancel)
            .await
            .err(),
        Some(ResolverError::Cancelled)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn canceled_or_dropped_fetch_can_retry_and_does_not_publish_a_cache() {
    for drop_request in [false, true] {
        let (owner, provider) = mock(Behavior::PendingOnce);
        let handle = owner.provider();
        let query = text(&owner, 1);
        let cancel = ResolverCancellation::default();
        let mut request = Box::pin(handle.lookup_bundle(&query, cancel.clone()));
        tokio::select! { biased;
            _ = provider.entered.notified() => (),
            result = &mut request => panic!("pending fixture returned: {}", result.is_ok()),
        }
        assert_eq!(
            handle
                .lookup_bundle(&query, ResolverCancellation::default())
                .await
                .err(),
            Some(ResolverError::TextOverloaded)
        );
        if !drop_request {
            cancel.cancel();
            assert_eq!(request.await.err(), Some(ResolverError::Cancelled));
        } else {
            drop(request);
        }
        assert!(owner.inner.document.lock().unwrap().is_none());
        assert!(
            handle
                .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn panic_and_invalid_document_do_not_poison_subsequent_fetch() {
    for (behavior, error) in [
        (Behavior::PanicOnce, ResolverError::WorkerPanicked),
        (Behavior::BadOnce, ResolverError::ContentMismatch),
    ] {
        let (owner, provider) = mock(behavior);
        let handle = owner.provider();
        assert_eq!(
            handle
                .lookup_bundle(&text(&owner, 1), ResolverCancellation::default())
                .await
                .err(),
            Some(error)
        );
        assert!(owner.inner.document.lock().unwrap().is_none());
        assert!(
            handle
                .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn stop_and_owner_drop_wipe_ram_and_retire_held_readers() {
    for explicit_stop in [false, true] {
        let (owner, _) = mock(Behavior::Ready);
        let inner = owner.inner.clone();
        let handle = owner.provider();
        let catalog = owner.catalog();
        let query = text(&owner, 1);
        handle
            .lookup_bundle(&query, ResolverCancellation::default())
            .await
            .unwrap();
        let guard = handle.publication_guard().unwrap();
        assert!(inner.document.lock().unwrap().is_some());
        if explicit_stop {
            owner.stop();
        }
        drop(owner);
        assert!(guard.is_cancelled());
        assert!(inner.document.lock().unwrap().is_none());
        assert_eq!(
            handle
                .lookup_bundle(&query, ResolverCancellation::default())
                .await
                .err(),
            Some(ResolverError::Cancelled)
        );
        assert_eq!(
            catalog
                .lookup(&movie(2), ResolverCancellation::default())
                .await
                .err(),
            Some(ResolverError::Cancelled)
        );
    }
}

#[tokio::test]
async fn cpu_saturation_has_no_queue_and_does_not_poison_document() {
    let (owner, provider) = mock(Behavior::Ready);
    let permit = owner.inner.workers.clone().try_acquire_owned().unwrap();
    let handle = owner.provider();
    assert_eq!(
        handle
            .lookup_bundle(&text(&owner, 1), ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::TextOverloaded)
    );
    assert!(owner.inner.document.lock().unwrap().is_none());
    drop(permit);
    assert!(
        handle
            .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn running_cpu_keeps_permits_after_future_drop_until_cooperative_exit() {
    let (owner, _) = mock(Behavior::Ready);
    let query = text(&owner, 1);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new(std::sync::Barrier::new(2));
    let worker_release = release.clone();
    let mut request = Box::pin(owner.inner.run(
        &query,
        ResolverCancellation::default(),
        move |_, check| {
            let _ = entered_tx.send(());
            worker_release.wait();
            check.run()?;
            Ok(())
        },
    ));
    tokio::select! { biased;
        _ = entered_rx => (),
        _ = &mut request => panic!("worker fixture returned early"),
    }
    drop(request);
    assert_eq!(owner.inner.requests.available_permits(), 0);
    assert_eq!(owner.inner.workers.available_permits(), 0);
    let handle = owner.provider();
    assert_eq!(
        handle
            .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::TextOverloaded)
    );
    release.wait();
    tokio::time::timeout(Duration::from_secs(2), async {
        while owner.inner.requests.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(owner.inner.document.lock().unwrap().is_none());
    assert_eq!(owner.inner.workers.available_permits(), 1);
}

#[tokio::test]
async fn deadline_rejects_pending_work_without_retaining_it() {
    let (owner, _) = mock(Behavior::PendingOnce);
    let handle = owner.provider();
    let mut query = text(&owner, 1);
    query.stamp.ended_at = Instant::now() - (MAX_WINDOW_AGE - Duration::from_millis(50));
    assert_eq!(
        handle
            .lookup_bundle(&query, ResolverCancellation::default())
            .await
            .err(),
        Some(ResolverError::Stale)
    );
    assert!(owner.inner.document.lock().unwrap().is_none());
    assert!(
        handle
            .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn unavailable_document_remains_unknown() {
    let (owner, _) = mock(Behavior::Missing);
    assert!(
        owner
            .catalog()
            .lookup(&movie(1), ResolverCancellation::default())
            .await
            .unwrap()
            .records
            .is_empty()
    );
    assert!(owner.inner.document.lock().unwrap().is_none());
}

#[tokio::test]
async fn measured_movie_candidate_reaches_live_subtitle_lease_and_exact_asr_proof() {
    use crate::{
        ai::speech::{TranscriptEvent, TranscriptKind},
        app::subtitles::{
            LiveSubtitles, SubtitleConfiguration, SubtitleMovieBinder, SubtitleServiceFactory,
        },
        resolver::candidates::{Resolution, SongState},
        subtitle::{exact_alignment::ExactCueVerifier, source_arbiter::SubtitleSourceArbiter},
        ui::{
            content::{ContentDisclosure, ContentTransport},
            subtitles::UiSubtitleState,
        },
    };
    struct FixtureFactory(Arc<SessionKnownSubtitle>);
    impl SubtitleServiceFactory for FixtureFactory {
        fn create(
            &self,
            id: StreamIdentity,
        ) -> Result<Arc<dyn SubtitleMovieBinder>, ResolverError> {
            if id != identity() {
                return Err(ResolverError::InvalidData);
            }
            Ok(Arc::new(Self(self.0.clone())))
        }
    }
    impl SubtitleMovieBinder for FixtureFactory {
        fn bind(
            &self,
            candidate: &MovieCandidate,
        ) -> Result<Arc<dyn ExistingTextProvider>, ResolverError> {
            if candidate.id.as_ref() != self.0.content_id() {
                return Err(ResolverError::ContentMismatch);
            }
            Ok(self.0.provider())
        }
    }
    let (owner, provider) = mock(Behavior::Ready);
    let owner = Arc::new(owner);
    let resolver = MovieMetadataResolver::new(identity(), owner.catalog()).unwrap();
    let mut query = movie(1);
    query.dialogue = Some(CUE.to_ascii_lowercase().into());
    let evidence = resolver
        .resolve(&query, ResolverCancellation::default())
        .await
        .unwrap();
    let resolution = Resolution {
        kind: evidence.kind,
        song_state: SongState::Unknown,
        song: None,
        movies: evidence.movies.into_iter().map(Arc::new).collect(),
    };
    let config = SubtitleConfiguration::new(
        ContentDisclosure {
            service: "明確測試字幕來源",
            transport: ContentTransport::Remote,
        },
        Arc::new(FixtureFactory(owner.clone())),
    )
    .unwrap();
    let mut subtitles = LiveSubtitles::new(identity(), Some(&config));
    subtitles.select(movie(2).stamp, &resolution).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), subtitles.changed())
            .await
            .unwrap(),
        UiSubtitleState::Available
    );
    let input = subtitles.ready_text().unwrap();
    assert_eq!(input.text().cues[0].text.as_ref(), CUE);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let version =
        SubtitleSourceArbiter::new(identity(), crate::domain::TranslationQuality::Standard)
            .version();
    let mut verifier = ExactCueVerifier::isolated(version);
    let proof = verifier
        .verify(
            input.clone(),
            TranscriptEvent {
                identity: identity(),
                sequence: 1,
                utterance: 1,
                revision: 1,
                kind: TranscriptKind::Final,
                language: Some(SpeechLanguage::English),
                start_us: Some(42_000_000),
                duration_us: Some(1_000_000),
                text: CUE.to_ascii_lowercase().into(),
            },
            movie(3).stamp,
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(proof.source_range(), 0..1);
    assert_eq!(proof.timing_agreement().permille(), 1000);
    owner.stop();
    assert_eq!(
        input.validate(identity()).err(),
        Some(ResolverError::Cancelled)
    );
    assert!(proof.validate(identity()).is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), subtitles.changed())
            .await
            .unwrap(),
        UiSubtitleState::Failed(ResolverError::Cancelled)
    );
    assert!(subtitles.ready_text().is_none());
    subtitles.stop();
}

async fn read_request(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 8192);
        assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}
async fn respond(server: &TcpListener) -> String {
    let (mut socket, _) = server.accept().await.unwrap();
    let request = read_request(&mut socket).await;
    let body = format!("1\n00:00:05,000 --> 00:00:06,000\n{CUE}\n");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-subrip; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
    request
}
fn local_owner(server: &TcpListener) -> SessionKnownSubtitle {
    let track = SubtitleTrack::local(
        &format!(
            "http://{}/source.srt?ticket=synthetic",
            server.local_addr().unwrap()
        ),
        SubtitleFormat::Srt,
        TextLanguage::Source(SpeechLanguage::English),
    )
    .unwrap();
    SessionKnownSubtitle::new(identity(), TITLE.into(), track).unwrap()
}

#[tokio::test]
async fn real_timed_transport_downloads_once_for_catalog_and_text_with_original_timing() {
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let owner = local_owner(&server);
    let catalog = owner.catalog();
    let input = movie(1);
    let (request, reply) = tokio::join!(
        respond(&server),
        catalog.lookup(&input, ResolverCancellation::default())
    );
    assert!(request.starts_with("GET /source.srt?ticket=synthetic "));
    assert_eq!(
        reply.unwrap().records[0].dialogue_references[0].as_ref(),
        CUE
    );
    let bundle = owner
        .provider()
        .lookup_bundle(&text(&owner, 2), ResolverCancellation::default())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bundle.text.cues[0].start_us, Some(5_000_000));
    assert_eq!(bundle.text.cues[0].duration_us, Some(1_000_000));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), server.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn real_pending_download_cancel_closes_transport_and_does_not_cache() {
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let owner = local_owner(&server);
    let handle = owner.provider();
    let input = text(&owner, 1);
    let cancel = ResolverCancellation::default();
    let mut request = Box::pin(handle.lookup_bundle(&input, cancel.clone()));
    let mut socket = tokio::select! {
        accepted = server.accept() => accepted.unwrap().0,
        _ = &mut request => panic!("pending HTTP fixture returned early"),
    };
    {
        let read = read_request(&mut socket);
        tokio::pin!(read);
        tokio::select! {
            _ = &mut read => (),
            _ = &mut request => panic!("pending HTTP fixture returned early"),
        }
    }
    cancel.cancel();
    assert_eq!(request.await.err(), Some(ResolverError::Cancelled));
    assert!(owner.inner.document.lock().unwrap().is_none());
    let mut byte = [0; 1];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap(),
        Ok(0) | Err(_)
    ));
}
