//! Explicit development probe: RAM-only synthetic PCM and mock providers.
//! No native capture, credentials, external network, media files or native UI.
use std::{
    error::Error,
    time::{Duration, Instant},
};
use subtitlenow::{
    ai::{provider::SpeechProvider, speech::*, translation::*},
    audio::capture::{BLOCK_FRAMES, CaptureSession, RING_BLOCKS, SAMPLE_RATE},
    domain::{SourceLanguage, TranslationQuality},
    performance::live::{Activity, Count, LiveDiagnostics},
    session::live_translation::LiveTranslation,
    subtitle::timeline::{MAX_TIMELINE_ITEMS, Stability},
    ui::bridge::{UiSource, UiStatus, channel},
};

const LIVE_DURATION: Duration = Duration::from_secs(300);
const WARMUP: Duration = Duration::from_secs(10);
const RESOURCE_INTERVAL: Duration = Duration::from_secs(15);
const PRIVATE_GROWTH_ALLOWANCE: u64 = 8 * 1024 * 1024;
const HANDLE_GROWTH_ALLOWANCE: u32 = 2;
const SOURCE_SWITCHES: usize = 100;
const AUDIO_INTERVAL: Duration = Duration::from_millis(20);
const SYNTHETIC_CLOCK_TICKS_PER_SECOND: u64 = 10_000_000;

struct MockSpeech {
    every: u64,
}
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
        _: Cancellation,
    ) -> SpeechFuture<'a> {
        Box::pin(async move {
            loop {
                let block = audio.recv().await?;
                if (block.stamp.sequence - 1).is_multiple_of(self.every) {
                    for (revision, kind) in
                        [(1, TranscriptKind::Partial), (2, TranscriptKind::Final)]
                    {
                        output.emit(TranscriptEvent {
                            identity: output.identity(),
                            utterance: block.stamp.sequence,
                            sequence: block.stamp.sequence,
                            revision,
                            start_us: block.start_us(),
                            duration_us: Some(block.duration_us()),
                            language: Some(SpeechLanguage::English),
                            kind,
                            text: "explicit synthetic hardening speech".into(),
                        })?;
                    }
                }
            }
        })
    }
}
struct MockTranslator;
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
        _: TranslationCancellation,
    ) -> TranslationFuture<'a> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            output.emit("明確合成")?;
            tokio::time::sleep(Duration::from_millis(5)).await;
            Ok("明確合成的臺灣測試譯文。".into())
        })
    }
}
fn source() -> Result<CaptureSession, Box<dyn Error>> {
    Ok(CaptureSession::start(|mut sink| {
        sink.started();
        let mut position = 0;
        while !sink.cancelled() {
            // Explicit mock sample clock. Unknown timing deliberately marks
            // every block discontinuous in the production input gate. This
            // synthetic clock is never native/media alignment evidence.
            let synthetic_qpc =
                position * SYNTHETIC_CLOCK_TICKS_PER_SECOND / u64::from(SAMPLE_RATE);
            sink.push_pcm(None, BLOCK_FRAMES, position, Some(synthetic_qpc), false)?;
            position += BLOCK_FRAMES as u64;
            std::thread::park_timeout(AUDIO_INTERVAL);
        }
        Ok(())
    })?)
}
async fn run(long: bool, quality: TranslationQuality) -> Result<(u64, u64), Box<dyn Error>> {
    let mut capture = source()?;
    let identity = StreamIdentity {
        session: capture.id(),
        generation: 1,
    };
    let metrics = LiveDiagnostics::new(identity);
    let _scope = metrics.scope();
    let speech_provider = MockSpeech {
        every: if long {
            u64::from(SAMPLE_RATE) / BLOCK_FRAMES as u64
        } else {
            1
        },
    };
    let (mut speech, task) = SpeechSession::diagnosed(
        identity,
        SourceLanguage::English,
        speech_provider.capabilities(),
        Some(metrics.clone()),
    )?;
    let mut speech_run = Box::pin(task.run(&speech_provider));
    let translator = MockTranslator;
    let mut flow =
        LiveTranslation::diagnosed(identity, quality, &translator, Some(metrics.clone()))?;
    let (mut publisher, mut inbox) = channel();
    publisher.begin(
        identity,
        UiSource {
            pid: std::process::id(),
            provider: SpeechProvider::OpenAi,
        },
    )?;
    publisher.attach_diagnostics(Some(metrics.clone()))?;
    let mut poll = tokio::time::interval(Duration::from_millis(5));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let began = Instant::now();
    let mut next_resource = WARMUP;
    let mut finals = 0u64;
    let mut timeline_peak = 0usize;
    #[cfg(windows)]
    let mut resources = subtitlenow::performance::resources::ProcessSampler::new(
        subtitlenow::win32::performance::NativeProcessProbe::current(),
    );
    #[cfg(windows)]
    let mut baseline: Option<subtitlenow::performance::resources::ProcessSample> = None;
    let mut resource_samples = 0u32;
    loop {
        tokio::select! {
            result = &mut speech_run => return Err(format!("explicit mock speech ended: {result:?}").into()),
            event = speech.next() => { let event = event.ok_or("explicit mock speech stopped")?;
                finals += u64::from(event.kind == TranscriptKind::Final); flow.ingest(event)?; },
            changed = flow.changed() => { changed?; publisher.publish(flow.timeline(), true)?;
                if let Some(frame) = inbox.poll() { inbox.render_submitted(frame.version()); } },
            _ = poll.tick() => {
                while let Some(block) = capture.try_next() { speech.input.try_push(block)?; }
                let state = capture.state();
                if matches!(state, subtitlenow::audio::capture::CaptureState::Stopped(_)) { return Err(format!("synthetic source stopped: {state:?}").into()); }
            },
        }
        timeline_peak = timeline_peak.max(flow.timeline().items().len());
        assert!(timeline_peak <= MAX_TIMELINE_ITEMS);
        let counts = capture.metrics();
        assert!(
            counts.raw_queue_depth <= RING_BLOCKS && counts.normalized_queue_depth <= RING_BLOCKS
        );
        if long && began.elapsed() >= next_resource {
            #[cfg(windows)]
            {
                let reading = resources.sample()?;
                let sample = reading.sample;
                if let Some(old) = baseline {
                    assert!(
                        sample.private_commit_bytes
                            <= old.private_commit_bytes + PRIVATE_GROWTH_ALLOWANCE
                    );
                    assert!(sample.handles <= old.handles + HANDLE_GROWTH_ALLOWANCE);
                } else {
                    baseline = Some(sample);
                }
                println!(
                    "{}",
                    serde_json::json!({"scenario":"explicit_mock_live_soak_sample","elapsed_ms":began.elapsed().as_millis(),"finals":finals,"timeline_items":flow.timeline().items().len(),"private_commit_bytes":sample.private_commit_bytes,"working_set_bytes":sample.working_set_bytes,"handles":sample.handles,"cpu_core_ppm":reading.cpu_core_ppm})
                );
            }
            resource_samples += 1;
            next_resource += RESOURCE_INTERVAL;
        }
        if long && began.elapsed() >= LIVE_DURATION {
            break;
        }
        if !long
            && flow
                .timeline()
                .items()
                .iter()
                .any(|item| item.stability == Stability::Final)
        {
            break;
        }
        if !long && began.elapsed() > Duration::from_secs(3) {
            return Err("synthetic switch failed to finish".into());
        }
    }
    if long {
        assert!(finals >= 128 && timeline_peak == MAX_TIMELINE_ITEMS && resource_samples >= 19);
    }
    speech.cancellation().cancel();
    drop(speech_run);
    drop(flow);
    drop(speech);
    capture.stop()?;
    publisher.finish(UiStatus::Stopped)?;
    assert!(
        inbox
            .poll()
            .is_some_and(|frame| frame.identity.is_none() && frame.snapshot.is_none())
    );
    for stage in Activity::ALL {
        assert_eq!(
            metrics.gauge(stage).current,
            0,
            "activity {stage:?} retained after stop"
        );
    }
    assert_eq!(capture.metrics().workers_running, 0);
    if long {
        println!(
            "{}",
            serde_json::json!({"scenario":"explicit_mock_live_soak_complete","elapsed_ms":began.elapsed().as_millis(),"finals":finals,"timeline_peak":timeline_peak,"samples":resource_samples,"workers_remaining":capture.metrics().workers_running,"unfinished_finals_at_stop":metrics.count(Count::FinalSegmentsUnfinishedAtStop),"captured_frames":capture.metrics().captured_frames,"dropped_frames":capture.metrics().dropped_frames,"stale_frames":capture.metrics().stale_frames,"mock_render_receipt":true,"native_ui":false,"native_capture":false,"cloud":false})
        );
    }
    Ok((identity.session.0, finals))
}
fn main() -> Result<(), Box<dyn Error>> {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::sink)
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    println!(
        "{}",
        serde_json::json!({"scenario":"explicit_mock_live_soak_start","duration_s":LIVE_DURATION.as_secs(),"synthetic_sample_clock":true,"native_capture":false,"native_ui":false,"cloud":false})
    );
    runtime.block_on(async {
        run(true, TranslationQuality::Standard).await?;
        #[cfg(windows)]
        let switch_baseline = subtitlenow::win32::performance::NativeProcessProbe::current().sample()?;
        let mut previous = None;
        let mut count = 0u64;
        for index in 0..SOURCE_SWITCHES {
            let quality = [TranslationQuality::Fast, TranslationQuality::Standard, TranslationQuality::High][index % 3];
            let (session, finals) = run(false, quality).await?;
            assert_ne!(previous, Some(session));
            previous = Some(session);
            count += finals;
            #[cfg(windows)]
            if (index + 1).is_multiple_of(20) {
                let sample = subtitlenow::win32::performance::NativeProcessProbe::current().sample()?;
                assert!(sample.private_commit_bytes <= switch_baseline.private_commit_bytes + PRIVATE_GROWTH_ALLOWANCE);
                assert!(sample.handles <= switch_baseline.handles + HANDLE_GROWTH_ALLOWANCE);
                println!("{}", serde_json::json!({"scenario":"synthetic_switch_resources","completed":index+1,"baseline_handles":switch_baseline.handles,"handles":sample.handles,"baseline_private_commit_bytes":switch_baseline.private_commit_bytes,"private_commit_bytes":sample.private_commit_bytes}));
            }
        }
        println!("{}", serde_json::json!({"scenario":"explicit_synthetic_source_switches","switches":SOURCE_SWITCHES,"finals":count,"cloud":false,"native_capture":false}));
        Ok(())
    })
}
