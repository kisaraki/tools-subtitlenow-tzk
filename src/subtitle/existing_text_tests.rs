//! Synthetic cues only; the real bundled dictionaries/CPU lane are exercised.
use super::*;
use crate::resolver::text_session::MAX_SESSION_TEXT_CUES;
use crate::{
    audio::capture::SessionId,
    resolver::contracts::{SubtitleSource, TextCue},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(901),
        generation: 1,
    }
}
fn fixture(sequence: u64, language: TextLanguage, text: &str) -> (TextQuery, ExistingText) {
    let query = TextQuery {
        stamp: WindowStamp {
            identity: identity(),
            sequence,
            ended_at: Instant::now(),
            start_us: None,
            duration_us: 12_000_000,
        },
        source: SubtitleSource::ExistingSubtitle,
        content_id: "synthetic subtitle".into(),
    };
    let text = ExistingText {
        identity: identity(),
        source: query.source,
        content_id: query.content_id.clone(),
        language,
        cues: vec![TextCue {
            start_us: Some(5_000_000),
            duration_us: Some(1_000_000),
            text: text.into(),
        }],
    };
    (query, text)
}
async fn run(
    owner: &mut ChineseTextLocalizer,
    query: TextQuery,
    text: ExistingText,
) -> ConversionResult {
    tokio::time::timeout(
        Duration::from_secs(5),
        owner.normalize(query, text, ResolverCancellation::default()),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn chinese_routes_preserve_native_text_and_localize_taiwan_terms() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    for (index, language, source, expected) in [
        (
            1,
            TextLanguage::TaiwanTraditional,
            "原生臺灣字幕：計程車\n與影片。",
            "原生臺灣字幕：計程車\n與影片。",
        ),
        (
            2,
            TextLanguage::SimplifiedChinese,
            "软件、服务器、网络、文件夹、鼠标、出租车、打印机、硬盘",
            "軟體、伺服器、網路、資料夾、滑鼠、計程車、印表機、硬碟",
        ),
        (
            3,
            TextLanguage::OtherTraditional,
            "軟件、服務器、網絡、文件夾、鼠標、出租車、打印機、硬盤",
            "軟體、伺服器、網路、資料夾、滑鼠、計程車、印表機、硬碟",
        ),
    ] {
        let (query, text) = fixture(index, language, source);
        let result = run(&mut owner, query, text).await.unwrap();
        assert_eq!(result.target_language(), "zh-TW");
        assert_eq!(result.original().cues[0].text.as_ref(), source);
        assert_eq!(result.translated()[0].as_ref(), expected);
        assert_eq!(result.original().cues[0].start_us, Some(5_000_000));
        assert_eq!(result.original().cues[0].duration_us, Some(1_000_000));
        assert_eq!(result.stamp().identity, identity());
        assert_eq!(result.stamp().sequence, index);
        assert!(result.stamp().start_us.is_none());
    }
}
#[tokio::test]
async fn mixed_timing_lyrics_preserve_cue_order_without_inventing_alignment() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    let (mut query, mut text) = fixture(1, TextLanguage::SimplifiedChinese, "软件");
    query.source = SubtitleSource::ExistingLyrics;
    text.source = query.source;
    text.cues.insert(
        0,
        TextCue {
            start_us: None,
            duration_us: None,
            text: "原創测试".into(),
        },
    );
    let result = run(&mut owner, query, text).await.unwrap();
    assert_eq!(result.translated().len(), 2);
    assert_eq!(result.translated()[0].as_ref(), "原創測試");
    assert_eq!(result.translated()[1].as_ref(), "軟體");
    assert!(result.original().cues[0].start_us.is_none());
    assert!(result.original().cues[0].duration_us.is_none());
}
#[tokio::test]
async fn all_five_source_languages_require_translation_and_are_never_relabelled() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    for (index, language) in SpeechLanguage::ALL.into_iter().enumerate() {
        let (query, text) = fixture(
            index as u64,
            TextLanguage::Source(language),
            "synthetic source text",
        );
        assert!(
            matches!(run(&mut owner, query, text).await, Err(TextLocalizationError::TranslationRequired(found)) if found == language)
        );
    }
}
#[tokio::test]
async fn session_content_and_timestamp_mismatches_are_rejected() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    for sequence in 1..=5 {
        let (mut query, mut text) = fixture(sequence, TextLanguage::TaiwanTraditional, "測試");
        match sequence {
            1 => query.stamp.identity.generation += 1,
            2 => text.identity.generation += 1,
            3 => text.content_id = "wrong content".into(),
            4 => text.cues[0].duration_us = Some(0),
            _ => {
                text.cues[0].start_us = Some(u64::MAX);
            }
        }
        assert!(matches!(
            run(&mut owner, query, text).await,
            Err(TextLocalizationError::Invalid(ResolverError::InvalidData))
        ));
    }
}
#[tokio::test]
async fn stale_future_and_reordered_requests_never_return_candidates() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    let (query, text) = fixture(3, TextLanguage::TaiwanTraditional, "測試");
    run(&mut owner, query, text).await.unwrap();
    for sequence in [2, 3, 4, 5] {
        let (mut query, text) = fixture(sequence, TextLanguage::TaiwanTraditional, "測試");
        if sequence == 4 {
            query.stamp.ended_at -= Duration::from_secs(6);
        }
        if sequence == 5 {
            query.stamp.ended_at += Duration::from_secs(1);
        }
        assert!(matches!(
            run(&mut owner, query, text).await,
            Err(TextLocalizationError::Invalid(ResolverError::Stale))
        ));
    }
}
#[tokio::test]
async fn cue_count_total_bytes_controls_and_missing_subtitle_times_are_bounded() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    for sequence in 1..=4 {
        let (query, mut text) = fixture(sequence, TextLanguage::TaiwanTraditional, "測試");
        match sequence {
            1 => {
                for _ in 0..MAX_SESSION_TEXT_CUES {
                    text.cues.push(TextCue {
                        start_us: Some(5_000_000),
                        duration_us: Some(1),
                        text: "測試".into(),
                    });
                }
            }
            2 => {
                for _ in 0..20 {
                    text.cues.push(TextCue {
                        start_us: Some(5_000_000),
                        duration_us: Some(1),
                        text: "a".repeat(8192).into(),
                    });
                }
            }
            3 => text.cues[0].text = "private\0synthetic".into(),
            _ => {
                text.cues[0].start_us = None;
                text.cues[0].duration_us = None;
            }
        }
        assert!(matches!(
            run(&mut owner, query, text).await,
            Err(TextLocalizationError::Invalid(ResolverError::InvalidData))
        ));
    }
}
#[tokio::test]
async fn cancellation_before_admission_and_stopped_worker_refuse_work() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    let cancel = ResolverCancellation::default();
    cancel.cancel();
    let (query, text) = fixture(1, TextLanguage::TaiwanTraditional, "測試");
    assert!(matches!(
        owner.normalize(query, text, cancel).await,
        Err(TextLocalizationError::Cancelled)
    ));
    assert!(owner.previous.is_none());
    owner.stop();
    owner.stop();
    let (query, text) = fixture(1, TextLanguage::TaiwanTraditional, "測試");
    assert!(matches!(
        run(&mut owner, query, text).await,
        Err(TextLocalizationError::Cancelled)
    ));
    assert!(owner.worker.is_none());
}
#[tokio::test]
async fn dropped_request_signals_cpu_work_and_cannot_return_into_replacement() {
    let (entered, mut receive) = tokio::sync::mpsc::channel(1);
    let mut first = true;
    let mut owner =
        ChineseTextLocalizer::with_converter(identity(), move |id, query, text, cancel| {
            if first {
                first = false;
                entered.try_send(cancel.clone()).unwrap();
                let until = Instant::now() + Duration::from_secs(2);
                while !cancel.is_cancelled() {
                    assert!(Instant::now() < until);
                    thread::sleep(Duration::from_millis(1));
                }
            }
            normalize(id, query, text, cancel)
        })
        .unwrap();
    let (query, text) = fixture(1, TextLanguage::TaiwanTraditional, "old synthetic cue");
    let mut old = Box::pin(owner.normalize(query, text, ResolverCancellation::default()));
    let cancel = tokio::select! {
        _ = &mut old => panic!("old conversion should still be blocked"),
        token = tokio::time::timeout(Duration::from_secs(2), receive.recv()) => token.unwrap().unwrap(),
    };
    drop(old);
    assert!(cancel.is_cancelled());
    let (query, text) = fixture(2, TextLanguage::SimplifiedChinese, "软件");
    let result = run(&mut owner, query, text).await.unwrap();
    assert_eq!(result.stamp().sequence, 2);
    assert_eq!(result.translated()[0].as_ref(), "軟體");
}
#[tokio::test]
async fn active_plus_one_pending_job_is_the_hard_queue_limit() {
    let release = Arc::new(AtomicBool::new(false));
    let released = release.clone();
    let (entered, mut receive) = tokio::sync::mpsc::channel(1);
    let mut first = true;
    let mut owner =
        ChineseTextLocalizer::with_converter(identity(), move |id, query, text, cancel| {
            if first {
                first = false;
                entered.try_send(()).unwrap();
                let until = Instant::now() + Duration::from_secs(2);
                while !released.load(Ordering::Acquire) {
                    assert!(Instant::now() < until);
                    thread::sleep(Duration::from_millis(1));
                }
            }
            normalize(id, query, text, cancel)
        })
        .unwrap();
    let (query, text) = fixture(1, TextLanguage::TaiwanTraditional, "一");
    let mut active = Box::pin(owner.normalize(query, text, ResolverCancellation::default()));
    tokio::select! {
        _ = &mut active => panic!("expected blocked CPU conversion"),
        event = tokio::time::timeout(Duration::from_secs(2), receive.recv()) => event.unwrap().unwrap(),
    }
    drop(active);
    let (query, text) = fixture(2, TextLanguage::TaiwanTraditional, "二");
    let mut pending = Box::pin(owner.normalize(query, text, ResolverCancellation::default()));
    assert!(futures_util::poll!(pending.as_mut()).is_pending());
    drop(pending);
    let (query, text) = fixture(3, TextLanguage::TaiwanTraditional, "三");
    let result = run(&mut owner, query, text).await;
    release.store(true, Ordering::Release);
    assert!(matches!(result, Err(TextLocalizationError::Overloaded)));
    owner.stop();
}
#[tokio::test]
async fn provider_neutral_worker_panic_is_typed_and_lane_can_recover() {
    let mut first = true;
    let mut owner =
        ChineseTextLocalizer::with_converter(identity(), move |id, query, text, cancel| {
            if first {
                first = false;
                panic!("synthetic conversion panic");
            }
            normalize(id, query, text, cancel)
        })
        .unwrap();
    let (query, text) = fixture(1, TextLanguage::TaiwanTraditional, "一");
    assert!(matches!(
        run(&mut owner, query, text).await,
        Err(TextLocalizationError::WorkerPanicked)
    ));
    let (query, text) = fixture(2, TextLanguage::TaiwanTraditional, "二");
    assert_eq!(
        run(&mut owner, query, text).await.unwrap().translated()[0].as_ref(),
        "二"
    );
}
#[test]
fn repeated_session_creation_and_drop_joins_every_worker() {
    for generation in 1..=20 {
        let mut owner = ChineseTextLocalizer::new(StreamIdentity {
            generation,
            ..identity()
        })
        .unwrap();
        owner.stop();
        assert!(owner.worker.is_none());
        assert!(owner.sender.is_none());
    }
}

#[tokio::test]
async fn external_cancellation_interrupts_active_conversion_and_joins_worker() {
    let (entered, mut receive) = tokio::sync::mpsc::channel(1);
    let mut owner =
        ChineseTextLocalizer::with_converter(identity(), move |id, query, text, cancel| {
            entered.try_send(cancel.clone()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !cancel.is_cancelled() {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            normalize(id, query, text, cancel)
        })
        .unwrap();
    let external = ResolverCancellation::default();
    let (query, text) = fixture(1, TextLanguage::TaiwanTraditional, "測試");
    let mut work = Box::pin(owner.normalize(query, text, external.clone()));
    let internal = tokio::select! {
        _ = &mut work => panic!("expected active conversion"),
        token = tokio::time::timeout(Duration::from_secs(2), receive.recv()) => token.unwrap().unwrap(),
    };
    external.cancel();
    assert!(matches!(work.await, Err(TextLocalizationError::Cancelled)));
    assert!(internal.is_cancelled());
    owner.stop();
    assert!(owner.worker.is_none());
}

#[tokio::test]
async fn terminology_expansion_cannot_exceed_cue_or_total_output_budget() {
    assert_eq!(taiwan_phrase_conversion("内存").unwrap(), "記憶體");
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    for (sequence, repeats, cues) in [(1, 1000, 1), (2, 900, 17)] {
        let (query, mut text) = fixture(sequence, TextLanguage::SimplifiedChinese, "測試");
        text.cues = (0..cues)
            .map(|index| TextCue {
                start_us: Some(index * 1_000_000),
                duration_us: Some(1_000_000),
                text: "内存".repeat(repeats).into(),
            })
            .collect();
        assert!(matches!(
            run(&mut owner, query, text).await,
            Err(TextLocalizationError::Invalid(ResolverError::InvalidData))
        ));
    }
}

#[tokio::test]
async fn translated_vectors_must_match_source_cues_and_keep_source_language_provenance() {
    let mut owner = ChineseTextLocalizer::new(identity()).unwrap();
    let language = TextLanguage::Source(SpeechLanguage::English);
    let (query, text) = fixture(1, language, "synthetic source");
    assert!(matches!(
        owner
            .normalize_translations(query, text, vec![], ResolverCancellation::default())
            .await,
        Err(TextLocalizationError::Invalid(ResolverError::InvalidData))
    ));
    let (query, text) = fixture(2, TextLanguage::TaiwanTraditional, "原生字幕");
    assert!(matches!(
        owner
            .normalize_translations(
                query,
                text,
                vec!["替換".into()],
                ResolverCancellation::default()
            )
            .await,
        Err(TextLocalizationError::Invalid(ResolverError::InvalidData))
    ));
    let (query, text) = fixture(3, language, "synthetic source");
    let result = owner
        .normalize_translations(
            query,
            text,
            vec!["软件".into()],
            ResolverCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.translated()[0].as_ref(), "軟體");
    assert!(matches!(
        result.original().language,
        TextLanguage::Source(SpeechLanguage::English)
    ));
    assert_eq!(result.original().cues[0].text.as_ref(), "synthetic source");
}
