//! Explicit live cloud orchestration. Stops capture before reconnecting; never replays old audio.
use crate::{
    ai::{
        credentials::{ApiKey, CredentialStore, WindowsCredentialStore},
        provider::SpeechProvider,
        speech::{SpeechError, SpeechRecognizer, SpeechSession, StreamIdentity, TranscriptLedger},
        streaming::NetworkRecognizer,
        translation::TranslationError,
        translation_http::HttpTranslator,
    },
    app::content::{ContentConfiguration, LiveContent},
    audio::capture::{CaptureSession, CaptureState, RING_BLOCKS},
    domain::{SourceLanguage, TranslationQuality},
    error::{AppError, Result},
    session::{
        live_text::LiveTextPipeline, live_translation::LiveTranslation,
        playback_clock::LivePlaybackClock,
    },
    ui::bridge::{UI_REFRESH_INTERVAL, UiBridgeError, UiPublisher, UiSource, UiStatus},
    win32::capture::{ProcessTarget, capture},
};
use std::{
    io::{IsTerminal, Write},
    time::{Duration, Instant},
};

use super::recovery::{Recovery, RecoveryAction};

pub(super) fn publish_content_status(
    publisher: &mut UiPublisher,
    identity: StreamIdentity,
    content: &LiveContent,
    text: Option<&LiveTextPipeline>,
    flow: Option<&LiveTranslation<'_>>,
) -> std::result::Result<(), UiBridgeError> {
    publisher.publish_content(identity, content.status())?;
    publisher.publish_lyrics(identity, lyrics_status(content, text, flow))?;
    publisher.publish_subtitles(identity, subtitles_status(content, text, flow))?;
    Ok(())
}
fn write_content_status(
    output: &mut impl Write,
    content: &LiveContent,
    text: Option<&LiveTextPipeline>,
    flow: Option<&LiveTranslation<'_>>,
) -> std::io::Result<()> {
    writeln!(output, "{}", content.status().label())?;
    if content.lyrics_status().disclosure.is_some() {
        writeln!(output, "{}", lyrics_status(content, text, flow).label())?;
    }
    if content.subtitles_status().disclosure.is_some() {
        writeln!(output, "{}", subtitles_status(content, text, flow).label())?;
    }
    output.flush()
}
fn lyrics_status(
    content: &LiveContent,
    text: Option<&LiveTextPipeline>,
    flow: Option<&LiveTranslation<'_>>,
) -> crate::ui::lyrics::UiLyricsStatus {
    let status = content.lyrics_status();
    if pipeline_source(content, text)
        != Some(crate::resolver::contracts::SubtitleSource::ExistingLyrics)
    {
        return status;
    }
    text.map_or(status, |text| {
        let adopted = flow
            .and_then(|flow| flow.timeline().items().last())
            .is_some_and(|item| {
                item.display_origin() == crate::resolver::contracts::SubtitleSource::ExistingLyrics
            });
        status.with_pipeline(text.status(), adopted)
    })
}
fn pipeline_source(
    content: &LiveContent,
    text: Option<&LiveTextPipeline>,
) -> Option<crate::resolver::contracts::SubtitleSource> {
    text.and_then(LiveTextPipeline::source)
        .or_else(|| content.ready_text().map(|input| input.text().source))
}
fn subtitles_status(
    content: &LiveContent,
    text: Option<&LiveTextPipeline>,
    flow: Option<&LiveTranslation<'_>>,
) -> crate::ui::subtitles::UiSubtitleStatus {
    let status = content.subtitles_status();
    if pipeline_source(content, text)
        != Some(crate::resolver::contracts::SubtitleSource::ExistingSubtitle)
    {
        return status;
    }
    text.map_or(status, |text| {
        let adopted = flow
            .and_then(|flow| flow.timeline().items().last())
            .is_some_and(|item| {
                item.display_origin()
                    == crate::resolver::contracts::SubtitleSource::ExistingSubtitle
            });
        status.with_pipeline(text.status(), adopted)
    })
}
pub(super) fn schedule_text(
    content: &LiveContent,
    text: Option<&mut LiveTextPipeline>,
    flow: Option<&mut LiveTranslation<'_>>,
) {
    if let (Some(text), Some(flow)) = (text, flow) {
        let version = flow.sources_mut().version();
        let event = flow
            .timeline()
            .items()
            .iter()
            .rev()
            .find(|item| item.source.kind == crate::ai::speech::TranscriptKind::Final)
            .map(|item| &item.source);
        if let Err(error) = text.offer(content.ready_text(), event, version) {
            text.reject(error);
        }
    }
}

#[derive(Clone)]
pub(super) struct LiveOptions {
    pub pid: u32,
    pub provider: SpeechProvider,
    pub live_model: Option<crate::ai::provider::LiveSpeechModel>,
    pub language: SourceLanguage,
    pub quality: Option<TranslationQuality>,
    pub content: Option<ContentConfiguration>,
}
pub(super) fn prompt_interactive_key(provider: SpeechProvider) -> Result<ApiKey> {
    let provider = match provider {
        SpeechProvider::OpenAi => "OpenAI",
        SpeechProvider::Gemini => "Gemini",
    };
    let prompt =
        format!("結案測試：請手動輸入 {provider} API 金鑰（隱藏輸入、僅供本次使用，不儲存）：");
    let text = zeroize::Zeroizing::new(
        rpassword::prompt_password(prompt)
            .map_err(|_| crate::ai::credentials::CredentialError::Unavailable)?,
    );
    Ok(ApiKey::from_text(&text)?)
}

pub async fn run(
    pid: u32,
    provider: SpeechProvider,
    language: SourceLanguage,
    interactive: bool,
    quality: Option<TranslationQuality>,
) -> Result<()> {
    // A live transcript goes to the console only. Refuse redirection to an archive.
    if !std::io::stdout().is_terminal() {
        return Err(AppError::ConsoleOutput);
    }
    let pinned_source = ProcessTarget::open(pid)?;
    let key = if interactive {
        Some(prompt_interactive_key(provider)?)
    } else {
        None
    };
    run_inner(
        LiveOptions {
            pid,
            provider,
            live_model: None,
            language,
            quality,
            content: None,
        },
        pinned_source,
        key,
        None,
        None,
    )
    .await
}
pub(super) async fn run_to_window(
    options: LiveOptions,
    pinned_source: ProcessTarget,
    key: Option<ApiKey>,
    publisher: &mut UiPublisher,
) -> Result<()> {
    if options.quality.is_none() {
        return Err(UiBridgeError::InvalidState.into());
    }
    run_inner(options, pinned_source, key, Some(publisher), None).await
}
pub(super) async fn run_to_window_prepared(
    options: LiveOptions,
    pinned_source: ProcessTarget,
    key: Option<ApiKey>,
    publisher: &mut UiPublisher,
    source: crate::subtitle::fuzzy_worker::AlignmentSource,
) -> Result<()> {
    if options.quality.is_none() || options.content.is_some() {
        return Err(UiBridgeError::InvalidState.into());
    }
    run_inner(options, pinned_source, key, Some(publisher), Some(source)).await
}
async fn run_inner(
    options: LiveOptions,
    pinned_source: ProcessTarget,
    interactive_key: Option<ApiKey>,
    mut ui: Option<&mut UiPublisher>,
    prepared_source: Option<crate::subtitle::fuzzy_worker::AlignmentSource>,
) -> Result<()> {
    let LiveOptions {
        pid,
        provider,
        live_model,
        language,
        quality,
        content,
    } = options;
    let live_model =
        live_model.unwrap_or_else(|| crate::ai::provider::LiveSpeechModel::default_for(provider));
    if live_model.provider() != provider {
        return Err(SpeechError::InvalidData.into());
    }
    live_model.validate_live(language, quality.unwrap_or_default())?;
    if live_model.native_translation()
        && (prepared_source.is_some() || content.is_some() || quality.is_none())
    {
        return Err(SpeechError::UnsupportedCapability.into());
    }
    let content = content.map(|config| {
        config.for_selected_source(crate::app::content::SelectedProcess {
            pid,
            created: pinned_source.creation_time(),
        })
    });
    if ui
        .as_ref()
        .is_some_and(|publisher| publisher.cancellation().is_cancelled())
    {
        return Ok(());
    }
    let ui_cancel = ui.as_ref().map(|publisher| publisher.cancellation());
    let stop_signal = async move {
        if let Some(cancel) = ui_cancel {
            tokio::select! {biased;_=cancel.cancelled()=>Ok(()),result=tokio::signal::ctrl_c()=>result}
        } else {
            tokio::signal::ctrl_c().await
        }
    };
    tokio::pin!(stop_signal);
    let mut output = std::io::stdout();
    if ui.is_none() {
        writeln!(output,"準備辨識 PID {pid} 與其子程序的音訊；所選服務：{provider:?}，來源語言：{language:?}。音訊將送至該服務，僅在本程式 RAM 處理、不寫檔。按 Ctrl+C 停止。")
        .and_then(|_| output.flush()).map_err(|_| AppError::ConsoleOutput)?;
        if let Some(quality) = quality {
            writeln!(output,"已啟用臺灣繁中翻譯，品質：{quality:?}；逐字稿、前文與詞彙將送至同一服務，僅保留當次 RAM，不匯出。")
        .and_then(|_|output.flush()).map_err(|_|AppError::ConsoleOutput)?;
        }
        if let Some(config) = &content {
            writeln!(
                output,
                "{}",
                crate::ui::content::UiContentStatus {
                    disclosure: Some(config.disclosure()),
                    state: crate::ui::content::UiContentState::Disabled,
                    movie_dialogue: config.movie_dialogue_enabled(),
                }
                .label()
            )
            .and_then(|_| output.flush())
            .map_err(|_| AppError::ConsoleOutput)?;
            if let Some(disclosure) = config.lyrics_disclosure() {
                writeln!(
                    output,
                    "{}",
                    crate::ui::lyrics::UiLyricsStatus {
                        disclosure: Some(disclosure),
                        state: crate::ui::lyrics::UiLyricsState::Disabled,
                    }
                    .label()
                )
                .and_then(|_| output.flush())
                .map_err(|_| AppError::ConsoleOutput)?;
            }
            if let Some(disclosure) = config.subtitles_disclosure() {
                writeln!(
                    output,
                    "{}",
                    crate::ui::subtitles::UiSubtitleStatus {
                        disclosure: Some(disclosure),
                        state: crate::ui::subtitles::UiSubtitleState::Disabled,
                    }
                    .label()
                )
                .and_then(|_| output.flush())
                .map_err(|_| AppError::ConsoleOutput)?;
            }
        }
    }
    let mut recovery = Recovery::new(pid);
    loop {
        if ui
            .as_ref()
            .is_some_and(|publisher| publisher.cancellation().is_cancelled())
        {
            return Ok(());
        }
        pinned_source.ensure_alive()?;
        let stored_key = if interactive_key.is_none() {
            Some(WindowsCredentialStore.read(provider)?)
        } else {
            None
        };
        let key = interactive_key
            .as_ref()
            .or(stored_key.as_ref())
            .ok_or(SpeechError::Credential)?;
        if live_model.native_translation() {
            let _ = crate::subtitle::localization::taiwan_phrase_conversion("")?;
        }
        let translator = quality
            .filter(|_| !live_model.native_translation())
            .map(|_| HttpTranslator::new(provider, key))
            .transpose()?;
        tracing::info!(state = ?super::state::AppState::StartingLive, cloud_processing = true, speech_provider = ?provider, "正在連接所選辨識服務");
        let provider_result = tokio::select! { biased;
            signal = &mut stop_signal => { signal.map_err(|_| AppError::StopSignal)?; return Ok(()); },
            result = NetworkRecognizer::connect_model(live_model, key, language, quality.unwrap_or_default()) => result,
        };
        drop(stored_key);
        let started = Instant::now();
        let result = match provider_result {
            Ok(recognizer) => {
                pinned_source.ensure_alive()?;
                let target = ProcessTarget::open(pid)?;
                pinned_source.ensure_alive()?;
                let mut capture_session =
                    CaptureSession::start_normalized(move |sink| capture(target, sink))?;
                let identity = StreamIdentity {
                    session: capture_session.id(),
                    generation: 1,
                };
                let diagnostics = tracing::enabled!(tracing::Level::DEBUG)
                    .then(|| crate::performance::live::LiveDiagnostics::new(identity));
                let _diagnostic_scope = diagnostics.as_ref().map(|d| d.scope());
                let mut process_sampler = diagnostics.as_ref().map(|_| {
                    crate::performance::resources::ProcessSampler::new(
                        crate::win32::performance::NativeProcessProbe::current(),
                    )
                });
                let mut helper_sampler = None;
                if recovery.attempts() > 0
                    && let Some(metrics) = &diagnostics
                {
                    for _ in 0..recovery.attempts() {
                        metrics.increment(crate::performance::live::Count::Reconnects);
                    }
                }
                let (mut session, task) = SpeechSession::diagnosed(
                    identity,
                    language,
                    recognizer.capabilities(),
                    diagnostics.clone(),
                )?;
                let cancel = session.cancellation();
                let mut native_captions = if live_model.native_translation() {
                    Some(session.take_captions().ok_or(SpeechError::InvalidData)?)
                } else {
                    None
                };
                let mut worker = Box::pin(task.run(&recognizer));
                let mut ledger = quality.is_none().then(|| TranscriptLedger::new(identity));
                let mut translations = if live_model.native_translation() {
                    Some(LiveTranslation::native(
                        identity,
                        quality.unwrap_or_default(),
                        diagnostics.clone(),
                    )?)
                } else {
                    translator
                        .as_ref()
                        .map(|translator| {
                            LiveTranslation::diagnosed(
                                identity,
                                quality.unwrap_or_default(),
                                translator,
                                diagnostics.clone(),
                            )
                        })
                        .transpose()?
                };
                if let Some(publisher) = ui.as_deref_mut() {
                    publisher.begin(identity, UiSource { pid, provider })?;
                    publisher.attach_diagnostics(diagnostics.clone())?;
                }
                let mut alignment_commands =
                    ui.as_ref().map(|publisher| publisher.alignment_commands());
                let alignment_command = alignment_commands
                    .as_mut()
                    .map(|commands| *commands.borrow_and_update())
                    .unwrap_or(crate::ui::alignment::AlignmentCommand {
                        running: true,
                        reset: 0,
                    });
                let mut aligned = match &prepared_source {
                    Some(source) => Some(
                        super::live_alignment::PreparedLiveAlignment::new(
                            source.clone(),
                            identity,
                            alignment_command,
                        )
                        .await?,
                    ),
                    None => None,
                };
                if let (Some(aligned), Some(publisher)) = (&aligned, ui.as_deref_mut()) {
                    publisher.publish_alignment(identity, aligned.state())?;
                }
                let mut contents = LiveContent::new(identity, content.as_ref());
                let (mut texts, text_task) = if translator.is_some()
                    && content.as_ref().is_some_and(|config| {
                        config.lyrics_disclosure().is_some()
                            || config.subtitles_disclosure().is_some()
                    }) {
                    let (owner, task) =
                        LiveTextPipeline::new(identity, quality.unwrap_or_default());
                    (Some(owner), Some(task))
                } else {
                    (None, None)
                };
                let mut text_worker = Box::pin(async {
                    match (text_task, translator.as_ref()) {
                        (Some(task), Some(provider)) => task.run(provider).await,
                        _ => std::future::pending().await,
                    }
                });
                let mut text_worker_running = texts.is_some();
                let mut playback = LivePlaybackClock::new(identity);
                if let Some(publisher) = ui.as_deref_mut() {
                    publish_content_status(
                        publisher,
                        identity,
                        &contents,
                        texts.as_ref(),
                        translations.as_ref(),
                    )?;
                }
                let mut ui_dirty = false;
                let mut ui_tick = tokio::time::interval(UI_REFRESH_INTERVAL);
                ui_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut drain = tokio::time::interval(Duration::from_millis(10));
                drain.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut report = tokio::time::interval(Duration::from_secs(1));
                report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let result: Result<()> = 'live_run: loop {
                    tokio::select! { biased;
                        signal = &mut stop_signal => { cancel.cancel(); break signal.map_err(|_| AppError::StopSignal); },
                        command = async { match &mut alignment_commands {
                            Some(commands) => commands.changed().await.map(|_| *commands.borrow_and_update()).ok(),
                            None => std::future::pending().await,
                        } }, if aligned.is_some() => {
                            let outcome: Result<()> = async {
                                if let Some(command) = command {
                                    if let Some(aligned) = &mut aligned {
                                        aligned.command(command).await?;
                                        if let Some(flow) = &mut translations { flow.sources_mut().invalidate().map_err(|_| TranslationError::InvalidData)?; flow.reconcile_sources(); }
                                        if let Some(publisher) = ui.as_deref_mut() { publisher.publish_alignment(identity, aligned.state())?; }
                                        ui_dirty = true;
                                    }
                                } else { alignment_commands = None; }
                                Ok(())
                            }.await;
                            if let Err(error) = outcome { break Err(error); }
                        },
                        result = async { match &mut aligned {Some(aligned) => aligned.changed().await, None => std::future::pending().await} }, if aligned.as_ref().is_some_and(|aligned| aligned.running()) => {
                            let outcome: Result<()> = async {
                                result?;
                                if let Some(aligned) = &mut aligned {
                                    ui_dirty |= aligned.reconcile(&playback, translations.as_mut()).await?;
                                    if let Some(publisher) = ui.as_deref_mut() { publisher.publish_alignment(identity, aligned.state())?; }
                                }
                                Ok(())
                            }.await;
                            if let Err(error) = outcome { break Err(error); }
                        },
                        result = &mut worker => break result.map_err(AppError::from),
                        _ = contents.changed() => {
                            schedule_text(&contents, texts.as_mut(), translations.as_mut());
                            if let Some(publisher) = ui.as_deref_mut() {
                                if let Err(error) = publish_content_status(publisher, identity, &contents, texts.as_ref(), translations.as_ref()) { break Err(error.into()); }
                            } else if content.is_some() && write_content_status(&mut output, &contents, texts.as_ref(), translations.as_ref()).is_err() { break Err(AppError::ConsoleOutput); }
                        },
                        result = &mut text_worker, if text_worker_running => {
                            text_worker_running = false;
                            if let Some(text) = &mut texts { match result { Ok(()) => text.stop(), Err(error) => text.fail(error) } }
                            if let Some(publisher) = ui.as_deref_mut()
                                && let Err(error) = publish_content_status(publisher, identity, &contents, texts.as_ref(), translations.as_ref()) { break Err(error.into()); }
                        },
                        completion = async { match &mut texts { Some(text) => text.changed().await, None => std::future::pending().await } } => {
                            if let (Some(completion), Some(text), Some(flow)) = (completion, texts.as_mut(), translations.as_mut()) {
                                let observation = playback.observation(flow.sources_mut().version());
                                let changed = text.apply(completion, flow, observation).unwrap_or(false);
                                ui_dirty |= changed;
                                if changed && ui.is_none()
                                    && let Some(binding) = text.binding()
                                    && let Some(item) = flow.timeline().items().iter().find(|item| item.source.utterance == binding.utterance)
                                    && write_translation(&mut output, item).is_err() { break Err(AppError::ConsoleOutput); }
                            }
                            if let Some(publisher) = ui.as_deref_mut() {
                                if let Err(error) = publish_content_status(publisher, identity, &contents, texts.as_ref(), translations.as_ref()) { break Err(error.into()); }
                            } else if content.is_some() && write_content_status(&mut output, &contents, texts.as_ref(), translations.as_ref()).is_err() { break Err(AppError::ConsoleOutput); }
                        },
                        update=async{match &mut translations{Some(coordinator)=>coordinator.changed().await,None=>std::future::pending().await}}=>{
                            if let Some(flow) = &translations
                                && let Some(publisher) = ui.as_deref_mut()
                                && let Err(error) = publisher.publish_translation_notice(identity, flow.notice()) {
                                break Err(error.into());
                            }
                            match update{
                                Ok(Some(utterance))=>{if let Some(coordinator)=&translations
                                    && let Some(item)=coordinator.timeline().items().iter().find(|item|item.source.utterance==utterance) {
                                        if let Some(publisher)=ui.as_deref_mut(){
                                            let force=matches!(item.display_stability(),crate::subtitle::timeline::Stability::Stable|crate::subtitle::timeline::Stability::Final);
                                            match publisher.publish(coordinator.timeline(),force){Ok(published)=>ui_dirty = !published,Err(error)=>break Err(error.into())}
                                        }else if write_translation(&mut output,item).is_err(){break Err(AppError::ConsoleOutput);}
                                    }},
                                Ok(None)=>{},Err(error)=>break Err(error.into()),
                            }
                            schedule_text(&contents, texts.as_mut(), translations.as_mut());
                        },
                        state = capture_session.changed() => match state {
                            Ok(CaptureState::Running) => { tracing::info!(session=identity.session.0,state=?super::state::AppState::LiveRunning,"正在聆聽與辨識指定來源"); },
                            Ok(CaptureState::Stopped(result)) => { cancel.cancel(); break result.map_err(AppError::from); },
                            Ok(CaptureState::Starting) => {},
                            Err(error) => { cancel.cancel(); break Err(error.into()); },
                        },
                        caption = async {match &mut native_captions {Some(receiver)=>receiver.recv().await,None=>std::future::pending().await}} => {
                            let outcome:Result<()>=(|| {
                                let caption=caption.ok_or(SpeechError::Network)?;
                                let final_caption=caption.kind==crate::ai::speech::TranscriptKind::Final;
                                let flow=translations.as_mut().ok_or(TranslationError::UnsupportedCapability)?;
                                if flow.ingest_native(caption)? {
                                    if let Some(publisher)=ui.as_deref_mut() {ui_dirty = !publisher.publish(flow.timeline(),final_caption)?;}
                                    else if let Some(item)=flow.timeline().items().last() {write_translation(&mut output,item).map_err(|_|AppError::ConsoleOutput)?;}
                                }
                                Ok(())
                            })();
                            if let Err(error)=outcome {break Err(error);}
                        },
                        event = session.next() => {
                            if let Some(event) = event {
                                let alignment_event = (aligned.as_ref().is_some_and(|aligned| aligned.running()) && event.kind == crate::ai::speech::TranscriptKind::Final).then(|| event.clone());
                                let sequence = event.sequence;
                                let movie_event = (contents.movie_dialogue_enabled()
                                    && event.kind == crate::ai::speech::TranscriptKind::Final)
                                    .then(|| event.clone());
                                let accepted = if let Some(coordinator)=&mut translations {
                                    match coordinator.ingest(event) {
                                        Ok(accepted) => { ui_dirty |= accepted; accepted },
                                        Err(error) => break Err(error.into()),
                                    }
                                } else if let Some(ledger)=&mut ledger { ledger.apply(event) }
                                else { false };
                                if accepted && let Some(event) = &movie_event { contents.observe_transcript(event); }
                                if accepted && let (Some(event), Some(aligned)) = (alignment_event, aligned.as_mut())
                                    && let Err(error) = aligned.offer(event, &playback).await {
                                    break Err(error.into());
                                }
                                if translations.is_none() && accepted
                                    && let Some(ledger) = &ledger
                                    && let Some(item) = ledger.entries().iter().find(|item| item.sequence == sequence) {
                                    let text:String = item.text.chars().map(|character| if character.is_control() { ' ' } else { character }).collect();
                                    if writeln!(output,"[工作階段 {}／來源 {}／段落 {}／修訂 {}／{:?}／時間 {:?} μs] {}",
                                        identity.session.0,item.sequence,item.utterance,item.revision,item.kind,item.start_us,text)
                                        .and_then(|_| output.flush()).is_err() { cancel.cancel(); break Err(AppError::ConsoleOutput); }
                                }
                            } else {
                                // Session closure accompanies task completion; avoid a ready-None busy loop.
                                break worker.as_mut().await.map_err(AppError::from);
                            }
                        },
                        _=ui_tick.tick(),if ui.is_some()&&(ui_dirty||aligned.is_some())=>{
                            if let Some(aligned) = &mut aligned {
                                match aligned.reconcile(&playback, translations.as_mut()).await { Ok(changed) => ui_dirty |= changed, Err(error) => break Err(error.into()) }
                                if let Some(publisher) = ui.as_deref_mut() && let Err(error) = publisher.publish_alignment(identity, aligned.state()) { break Err(error.into()); }
                            }
                            if let Some(publisher)=ui.as_deref_mut() && let Some(coordinator)=&translations {
                                match publisher.publish(coordinator.timeline(),false){Ok(published)=>ui_dirty = !published,Err(error)=>break Err(error.into())}
                            }
                        },
                        _ = drain.tick() => {
                            let mut failure = None;
                            let mut content_dirty = false;
                            for _ in 0..RING_BLOCKS {
                                let Some(block) = capture_session.try_next() else { break; };
                                if playback.observe(&block) {
                                    if let Some(aligned) = &mut aligned && aligned.running()
                                        && let Err(error) = aligned.audio_gap().await { break 'live_run Err(error.into()); }
                                    content_dirty |= contents.reset_text_for_audio_gap();
                                    if let Some(text) = &mut texts { text.clear(); }
                                    if let Some(flow) = &mut translations {
                                        if flow.sources_mut().invalidate().is_err() { break 'live_run Err(AppError::from(TranslationError::InvalidData)); }
                                        ui_dirty |= flow.reconcile_sources().is_some();
                                    }
                                }
                                match contents.submit_audio(&session.input, block) {
                                    Ok(changed) => content_dirty |= changed,
                                    Err(error) => { failure = Some(error); break; }
                                }
                            }
                            if let Some(error) = failure { cancel.cancel(); break Err(error.into()); }
                            schedule_text(&contents, texts.as_mut(), translations.as_mut());
                            if content_dirty {
                                if let Some(publisher) = ui.as_deref_mut() {
                                    if let Err(error) = publish_content_status(publisher, identity, &contents, texts.as_ref(), translations.as_ref()) { break Err(error.into()); }
                                } else if content.is_some() && write_content_status(&mut output, &contents, texts.as_ref(), translations.as_ref()).is_err() { break Err(AppError::ConsoleOutput); }
                            }
                        },
                        _ = report.tick() => {
                            if !live_model.native_translation() && let Some(publisher) = ui.as_deref_mut()
                                && let Err(error) = publisher.publish_speech_progress(session.progress_snapshot()) {
                                    break Err(error.into());
                                }
                            if let Some(sampler)=&mut process_sampler {
                                super::performance::report_resources("parent",sampler);
                                if helper_sampler.is_none() {helper_sampler=capture_session.resource_probe().map(crate::performance::resources::ProcessSampler::new);}
                                if let Some(helper)=&mut helper_sampler {super::performance::report_resources("capture_helper",helper);}
                            }
                            if let Some(metrics)=&diagnostics {if let Some(resolver)=contents.metrics(){metrics.observe(crate::performance::live::Activity::ResolverWorkers,resolver.worker_count);} super::performance::report_live(metrics);}
                            let metrics = capture_session.metrics();
                            super::performance::report_capture(identity.session, metrics);
                            tracing::debug!(session=identity.session.0,raw_queue_depth=metrics.raw_queue_depth,
                                normalized_queue_depth=metrics.normalized_queue_depth,dropped_frames=metrics.dropped_frames,
                                stt_audio_queue_depth=session.input.queue_depth(),stt_event_queue_depth=session.event_queue_depth(),
                                stale_frames=metrics.stale_frames,stt_sessions=1,reconnect_count=recovery.attempts(),"辨識管線統計");
                            if let Some(coordinator)=&translations{tracing::debug!(translation_active_requests=coordinator.active_requests(),translation_pending=coordinator.pending_requests(),translation_queue_depth=coordinator.update_queue_depth(),"翻譯管線統計");}
                            if let Some(metrics)=contents.metrics(){tracing::debug!(resolver_audio_queue_depth=metrics.queue_depth,resolver_processed_blocks=metrics.processed_blocks,resolver_dropped_blocks=metrics.dropped_blocks,resolver_resets=metrics.resets,resolver_worker_count=metrics.worker_count,"內容辨識管線統計");}
                        },
                    }
                };
                // Last numeric sample remains available with an error; never skips cleanup.
                if language == SourceLanguage::Auto
                    && let Some(publisher) = ui.as_deref_mut()
                {
                    let _ = publisher.publish_speech_progress(session.progress_snapshot());
                }
                cancel.cancel();
                drop(worker);
                drop(session);
                let alignment_result = if let Some(aligned) = &mut aligned {
                    aligned.stop().await.map_err(AppError::from)
                } else {
                    Ok(())
                };
                if let Some(text) = &mut texts {
                    text.stop();
                }
                drop(text_worker);
                contents.stop();
                if let Some(coordinator) = &translations {
                    coordinator.cancellation().cancel();
                }
                drop(translations);
                if let Some(metrics) = &diagnostics {
                    metrics.observe(crate::performance::live::Activity::TranslationPending, 0);
                    metrics.observe(crate::performance::live::Activity::TranslationUpdates, 0);
                    metrics.observe(crate::performance::live::Activity::ResolverWorkers, 0);
                    super::performance::report_live(metrics);
                }
                // Dropping the future closes its socket and joins its bounded CPU worker.
                // Stop/join capture regardless of the recognition outcome.
                let capture_result = capture_session.stop();
                if let Some(sampler) = &mut process_sampler {
                    super::performance::report_resources("parent_stopped", sampler);
                }
                super::performance::report_capture(identity.session, capture_session.metrics());
                result
                    .and(alignment_result)
                    .and(capture_result.map_err(AppError::from))
            }
            Err(error) => Err(error.into()),
        };
        match recovery.decide(&result, started.elapsed()) {
            RecoveryAction::Stop => {
                if ui.is_none() {
                    writeln!(output, "PID {pid} 辨識已停止；工作階段記憶體已釋放。")
                        .map_err(|_| AppError::ConsoleOutput)?;
                }
                return Ok(());
            }
            RecoveryAction::Rotate => {
                if let Some(publisher) = ui.as_deref_mut() {
                    publisher.clear(UiStatus::Reconnecting)?;
                }
                tracing::info!(state=?super::state::AppState::Recovering,"辨識連線期限已到，重新建立工作階段；未完成段落已捨棄");
            }
            RecoveryAction::Retry(delay) => {
                if let Some(publisher) = ui.as_deref_mut() {
                    publisher.clear(UiStatus::Reconnecting)?;
                }
                if let Err(error) = &result {
                    tracing::warn!(state=?super::state::AppState::Recovering,retry=recovery.attempts(),error=%error,"即時處理中斷，停止舊擷取與要求後重連同一來源及服務；未完成內容已捨棄");
                }
                tokio::select! { biased;
                    signal = &mut stop_signal => { signal.map_err(|_| AppError::StopSignal)?; return Ok(()); },
                    _ = tokio::time::sleep(delay) => {},
                }
            }
            RecoveryAction::Fail => return result,
        }
    }
}
fn write_translation(
    output: &mut impl Write,
    item: &crate::subtitle::timeline::TimelineItem,
) -> std::io::Result<()> {
    let clean = |text: &str| {
        text.chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>()
    };
    let display = item.display_at(Instant::now());
    writeln!(
        output,
        "[工作階段 {}／段落 {}／{:?}／時間 {:?} μs] 原文：{}\n  臺灣繁中：{}",
        item.source.identity.session.0,
        item.source.utterance,
        display.stability,
        display.timestamp_us,
        clean(display.original),
        clean(display.translated)
    )?;
    output.flush()
}
