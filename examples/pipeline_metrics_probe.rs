//! Explicit mock-only pipeline: synthetic PCM, no cloud, media file or native UI.
use std::{
    error::Error,
    time::{Duration, Instant},
};
use subtitlenow::{
    ai::{speech::*, translation::*},
    audio::{
        capture::{AudioStamp, BLOCK_FRAMES, SessionId},
        preprocess::NormalizedAudio,
    },
    domain::{SourceLanguage, TranslationQuality},
    performance::{
        live::{Activity, Count, Latency, LiveDiagnostics},
        resources::ProcessSampler,
    },
    session::live_translation::LiveTranslation,
    subtitle::timeline::Stability,
    ui::bridge::{UiSource, channel},
};
static NEXT_SYNTHETIC_SESSION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(98_101);
fn next_identity() -> Result<StreamIdentity, Box<dyn Error>> {
    Ok(StreamIdentity {
        session: SessionId(
            NEXT_SYNTHETIC_SESSION
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |id| id.checked_add(1),
                )
                .map_err(|_| "synthetic Session ID exhausted")?,
        ),
        generation: 1,
    })
}
struct MockRecognizer;
impl SpeechRecognizer for MockRecognizer {
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
        _cancel: Cancellation,
    ) -> SpeechFuture<'a> {
        Box::pin(async move {
            loop {
                let block = audio.recv().await?;
                let utterance = block.stamp.sequence;
                for (revision, kind) in [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)] {
                    output.emit(TranscriptEvent {
                        identity: output.identity(),
                        utterance,
                        sequence: utterance,
                        revision,
                        start_us: block.start_us(),
                        duration_us: Some(block.duration_us()),
                        language: Some(SpeechLanguage::English),
                        kind,
                        text: format!("explicit mock speech number {utterance}").into(),
                    })?;
                }
            }
        })
    }
}
struct MockTranslator {
    pending: bool,
}
impl Translator for MockTranslator {
    fn capabilities(&self) -> TranslationCapabilities {
        TranslationCapabilities {
            streaming: true,
            cancellation: true,
            supported_languages: [true; 5],
            max_concurrent: 4,
        }
    }
    fn translate<'a>(
        &'a self,
        _: &'a TranslationRequest,
        output: &'a mut TranslationEmitter,
        cancel: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            if self.pending {
                return Err(cancel.cancelled().await);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            output.emit("合成")?;
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok("合成測試的臺灣譯文。".into())
        })
    }
}
fn input(identity: StreamIdentity, sequence: u64) -> NormalizedAudio {
    NormalizedAudio {
        stamp: AudioStamp {
            session: identity.session,
            sequence,
            frame_position: Some((sequence - 1) * BLOCK_FRAMES as u64),
            qpc_100ns: None,
            received: Instant::now(),
            discontinuity: false,
        },
        frames: BLOCK_FRAMES,
        samples: [0.0; BLOCK_FRAMES],
    }
}
async fn run_case(quality: TranslationQuality, parallel: bool) -> Result<(), Box<dyn Error>> {
    let identity = next_identity()?;
    let metrics = LiveDiagnostics::new(identity);
    let _scope = metrics.scope();
    #[cfg(windows)]
    let mut resources =
        ProcessSampler::new(subtitlenow::win32::performance::NativeProcessProbe::current());
    #[cfg(windows)]
    let _baseline = resources.sample()?;
    let recognizer = MockRecognizer;
    let translator = MockTranslator { pending: false };
    let (mut speech, task) = SpeechSession::diagnosed(
        identity,
        SourceLanguage::English,
        recognizer.capabilities(),
        Some(metrics.clone()),
    )?;
    let mut worker = Box::pin(task.run(&recognizer));
    let mut flow =
        LiveTranslation::diagnosed(identity, quality, &translator, Some(metrics.clone()))?;
    let (mut publisher, mut inbox) = channel();
    publisher.begin(
        identity,
        UiSource {
            pid: 1,
            provider: subtitlenow::ai::provider::SpeechProvider::OpenAi,
        },
    )?;
    publisher.attach_diagnostics(Some(metrics.clone()))?;
    let mut offered = if parallel { 8 } else { 1 };
    for sequence in 1..=offered {
        speech.input.try_push(input(identity, sequence))?;
    }
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            tokio::select! {
                result=&mut worker=>return Err::<(),Box<dyn Error>>(format!("mock STT ended: {result:?}").into()),
                event=speech.next()=>{let event=event.ok_or("mock STT stopped")?;flow.ingest(event)?;},
                result=flow.changed()=>{result?;publisher.publish(flow.timeline(),true)?;if let Some(frame)=inbox.poll(){inbox.render_submitted(frame.version());}},
            }
            let finals=flow.timeline().items().iter().filter(|item|item.stability==Stability::Final).count() as u64;
            if finals==8 {break;}
            if !parallel && finals==offered {offered+=1;speech.input.try_push(input(identity,offered))?;}
        }
        Ok(())
    }).await??;
    speech.cancellation().cancel();
    drop(worker);
    drop(flow);
    drop(speech);
    assert_eq!(metrics.gauge(Activity::SpeechOperation).current, 0);
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 0);
    let durations:Vec<_>=Latency::ALL.into_iter().map(|key|{let d=metrics.distribution(key);serde_json::json!({"stage":format!("{key:?}"),"samples":d.samples,"p50_upper_ns":d.p50_upper_ns,"p95_upper_ns":d.p95_upper_ns,"max_ns":d.max_ns})}).collect();
    let activities:Vec<_>=Activity::ALL.into_iter().map(|key|{let g=metrics.gauge(key);serde_json::json!({"stage":format!("{key:?}"),"current":g.current,"observed_high_water":g.observed_high_water})}).collect();
    let counts: Vec<_> = Count::ALL
        .into_iter()
        .map(|key| serde_json::json!({"reason":format!("{key:?}"),"count":metrics.count(key)}))
        .collect();
    #[cfg(windows)]
    let resource = {
        let r = resources.sample()?;
        serde_json::json!({"cpu_core_ppm":r.cpu_core_ppm,"cpu_machine_ppm":r.cpu_machine_ppm,"private_commit_bytes":r.sample.private_commit_bytes,"working_set_bytes":r.sample.working_set_bytes,"handles":r.sample.handles})
    };
    #[cfg(not(windows))]
    let resource = serde_json::Value::Null;
    println!(
        "{}",
        serde_json::json!({"scenario":"explicit_mock_pipeline","session":identity.session.0,"quality":format!("{quality:?}"),"parallel":parallel,"mock_render_receipt":true,"native_ui":false,"durations":durations,"activities":activities,"counts":counts,"resources":resource})
    );
    Ok(())
}
async fn cancel_case() -> Result<(), Box<dyn Error>> {
    let identity = next_identity()?;
    let metrics = LiveDiagnostics::new(identity);
    let translator = MockTranslator { pending: true };
    let mut flow = LiveTranslation::diagnosed(
        identity,
        TranslationQuality::Fast,
        &translator,
        Some(metrics.clone()),
    )?;
    for sequence in 1..=4 {
        flow.ingest(TranscriptEvent {
            identity,
            utterance: sequence,
            sequence,
            revision: 1,
            start_us: Some(0),
            duration_us: Some(20_000),
            language: Some(SpeechLanguage::English),
            kind: TranscriptKind::Final,
            text: "explicit pending mock".into(),
        })?;
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), flow.changed())
            .await
            .is_err()
    );
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 4);
    drop(flow);
    assert_eq!(metrics.gauge(Activity::TranslationOperation).current, 0);
    assert_eq!(metrics.gauge(Activity::TranslationPermits).current, 0);
    assert_eq!(
        metrics
            .distribution(Latency::TranslationRequestFinal)
            .samples,
        0
    );
    println!(
        "{}",
        serde_json::json!({"scenario":"mock_cancel_four_pending_requests","peak":metrics.gauge(Activity::TranslationOperation).observed_high_water,"remaining":metrics.gauge(Activity::TranslationOperation).current,"false_completions":metrics.distribution(Latency::TranslationRequestFinal).samples,"unfinished_finals_at_stop":metrics.count(Count::FinalSegmentsUnfinishedAtStop)})
    );
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        for round in 1..=3 {
        for quality in [
            TranslationQuality::Fast,
            TranslationQuality::Standard,
            TranslationQuality::High,
        ] {
            run_case(quality, false).await?;
        }
        run_case(TranslationQuality::Standard, true).await?;
        cancel_case().await?;
        #[cfg(windows)] {let sample=subtitlenow::win32::performance::NativeProcessProbe::current().sample()?;println!("{}",serde_json::json!({"scenario":"post_round_resources","round":round,"private_commit_bytes":sample.private_commit_bytes,"working_set_bytes":sample.working_set_bytes,"handles":sample.handles}));}
        }
        Ok(())
    })
}
