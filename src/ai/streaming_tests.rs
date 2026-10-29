//! Local protocol fixtures only; never real credentials or cloud requests.
use super::*;
use crate::{
    ai::speech::{SpeechSession, TranscriptLedger},
    audio::{
        capture::{AudioStamp, BLOCK_FRAMES, SessionId},
        resample::ResampledAudio,
    },
};
fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(900),
        generation: 1,
    }
}
fn caps() -> SpeechCapabilities {
    SpeechCapabilities {
        streaming: true,
        partials: true,
        cancellation: true,
        supported_languages: [true; 5],
        constrained_auto_detection: true,
    }
}
const LOCAL_ENGLISH: &str = "Today we are discussing how computers process information and display translated subtitles on the screen.";

#[tokio::test]
async fn phase11_pause_boundary_socket_commits_at_observed_gap_and_keeps_following_audio() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut blocks = 0;
        let mut commits = Vec::new();
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let message: Value = serde_json::from_str(&text).unwrap();
            if message["type"] == "input_audio_buffer.append" {
                let bytes = STANDARD.decode(message["audio"].as_str().unwrap()).unwrap();
                assert_eq!(bytes.len(), 480 * 2);
                blocks += 1;
            } else if message["type"] == "input_audio_buffer.commit" {
                commits.push(blocks);
                let key = format!("pause-{}", commits.len());
                send_json(&mut socket, serde_json::from_slice(&ack(&key)).unwrap())
                    .await
                    .unwrap();
                send_json(&mut socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":key,"content_index":0,"transcript":"synthetic final"})).await.unwrap();
            }
        }
        assert_eq!(blocks, 220);
        assert_eq!(commits, vec![185, 220]);
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::OpenAi,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            for sequence in 0..220 {
                let level = if (180..185).contains(&sequence) || sequence >= 200 {
                    0.0
                } else {
                    0.1
                };
                session
                    .input
                    .try_push(crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp {
                            session: identity().session,
                            sequence,
                            frame_position: Some(sequence * BLOCK_FRAMES as u64),
                            qpc_100ns: None,
                            received: Instant::now(),
                            discontinuity: false,
                        },
                        frames: BLOCK_FRAMES,
                        samples: [level; BLOCK_FRAMES],
                    })
                    .unwrap();
                if sequence < 219 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            for (sequence, start, duration) in [
                (0, None, 3_700_000),
                (
                    185,
                    Some(
                        (185 * BLOCK_FRAMES as u64
                            - crate::audio::resample::FILTER_DELAY_FRAMES as u64)
                            * 1_000_000
                            / 48000,
                    ),
                    700_000,
                ),
            ] {
                let final_event = session.next().await.unwrap();
                assert_eq!(
                    (
                        final_event.kind,
                        final_event.sequence,
                        final_event.start_us,
                        final_event.duration_us
                    ),
                    (TranscriptKind::Final, sequence, start, Some(duration))
                );
            }
            let progress = session.progress_snapshot();
            assert_eq!(
                (
                    progress.audio_received,
                    progress.audio_sent,
                    progress.prefix_blocks
                ),
                (220, 220, 0)
            );
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[test]
fn phase11_openai_prefix_is_bounded_by_time_and_item_count() {
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    for sequence in 0..8 {
        let mut quiet = block(sequence, false, false);
        quiet.active = false;
        quiet.audio.samples.fill(sequence as i16 + 1);
        protocol.audio(quiet).unwrap();
    }
    assert_eq!(
        (protocol.openai_prefix.len(), protocol.openai_prefix_frames),
        (4, 1920)
    );
    let messages = protocol.audio(block(8, true, false)).unwrap();
    let bytes = STANDARD
        .decode(messages[0]["audio"].as_str().unwrap())
        .unwrap();
    assert_eq!(bytes.len(), (1920 + 480) * 2);
    assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 5);
    assert_eq!(protocol.current.as_ref().unwrap().context.sequence, 4);
    assert_eq!(protocol.prefix_merged, 4);
    let mut tiny = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    for sequence in 0..100 {
        let mut quiet = block(sequence, false, false);
        quiet.active = false;
        quiet.audio.frames = 1;
        quiet.audio.stamp.frame_position = Some(sequence * 2);
        tiny.audio(quiet).unwrap();
    }
    assert_eq!(
        tiny.openai_prefix.len(),
        super::super::speech::AUDIO_QUEUE_BLOCKS
    );
    assert_eq!(
        tiny.openai_prefix_frames,
        super::super::speech::AUDIO_QUEUE_BLOCKS as u64
    );
}

#[test]
fn phase11_openai_prefix_expiry_and_unknown_time_do_not_invent_a_capture_start() {
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    let mut quiet = block(100, false, false);
    quiet.active = false;
    quiet.audio.stamp.frame_position = None;
    protocol.audio(quiet).unwrap();
    protocol.audio(block(101, true, false)).unwrap();
    assert_eq!(protocol.current.as_ref().unwrap().context.start_us, None);
    let mut expired = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    let mut quiet = block(1, false, false);
    quiet.active = false;
    expired.audio(quiet).unwrap();
    expired.prune_openai_prefix(Instant::now() + OPENAI_PREFIX_MAX_AGE + Duration::from_millis(1));
    assert!(expired.openai_prefix.is_empty());
    assert_eq!(expired.openai_prefix_frames, 0);
    let messages = expired.audio(block(2, true, false)).unwrap();
    assert_eq!(
        STANDARD
            .decode(messages[0]["audio"].as_str().unwrap())
            .unwrap()
            .len(),
        480 * 2
    );
    assert_eq!(expired.current.as_ref().unwrap().context.sequence, 2);
}

#[test]
fn phase11_openai_prefix_rejects_mixed_sessions_or_formats_and_leaves_gemini_unchanged() {
    for session_mismatch in [true, false] {
        let mut protocol =
            Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
        let mut quiet = block(0, false, false);
        quiet.active = false;
        protocol.audio(quiet).unwrap();
        let mut voice = block(1, true, false);
        if session_mismatch {
            voice.audio.stamp.session = SessionId(999);
        } else {
            voice.audio.sample_rate = 16000;
        }
        assert_eq!(protocol.audio(voice), Err(SpeechError::InvalidData));
        assert!(protocol.current.is_none());
    }
    let mut gemini = Protocol::new(SpeechProvider::Gemini, SourceLanguage::English, identity());
    let mut quiet = block(0, false, false);
    quiet.active = false;
    assert_eq!(gemini.audio(quiet).unwrap().len(), 1);
    assert!(gemini.openai_prefix.is_empty());
}

#[test]
fn phase11_openai_prefix_preserves_weak_audio_after_a_previous_turn_boundary() {
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    protocol.audio(block(0, true, true)).unwrap();
    let mut quiet = block(1, false, false);
    quiet.active = false;
    quiet.audio.samples.fill(16);
    protocol.audio(quiet).unwrap();
    let messages = protocol.audio(block(2, true, false)).unwrap();
    assert_eq!(
        STANDARD
            .decode(messages[0]["audio"].as_str().unwrap())
            .unwrap()
            .len(),
        960 * 2
    );
    assert_eq!(protocol.current.as_ref().unwrap().context.sequence, 1);
    assert_eq!(protocol.pending.front().unwrap().0.sequence, 0);
}

#[tokio::test]
async fn phase11_openai_prefix_real_socket_preserves_pcm_span_and_reports_merged_blocks() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut blocks = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "input_audio_buffer.append" {
                let bytes = STANDARD.decode(value["audio"].as_str().unwrap()).unwrap();
                if blocks == 0 {
                    assert_eq!(bytes.len(), 3 * 480 * 2);
                    let prefix = &bytes.as_chunks::<2>().0[..960];
                    assert!(prefix.iter().any(|sample| i16::from_le_bytes(*sample) != 0));
                    assert!(
                        prefix
                            .iter()
                            .all(|sample| i16::from_le_bytes(*sample).unsigned_abs() < 32)
                    );
                }
                blocks += 1;
            } else if value["type"] == "input_audio_buffer.commit" {
                send_json(
                    &mut socket,
                    serde_json::from_slice(&ack("prefix-real")).unwrap(),
                )
                .await
                .unwrap();
                send_json(&mut socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"prefix-real","content_index":0,"transcript":"synthetic final"})).await.unwrap();
            }
        }
        assert_eq!(blocks, 26);
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::OpenAi,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            for sequence in 0..28 {
                session
                    .input
                    .try_push(crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp {
                            session: identity().session,
                            sequence,
                            frame_position: Some(sequence * BLOCK_FRAMES as u64),
                            qpc_100ns: None,
                            received: Instant::now(),
                            discontinuity: false,
                        },
                        frames: BLOCK_FRAMES,
                        samples: [if sequence < 2 {
                            0.0005
                        } else if sequence < 8 {
                            0.1
                        } else {
                            0.0
                        }; BLOCK_FRAMES],
                    })
                    .unwrap();
                if sequence < 27 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            let final_event = session.next().await.unwrap();
            assert_eq!(
                (
                    final_event.kind,
                    final_event.sequence,
                    final_event.start_us,
                    final_event.duration_us
                ),
                (TranscriptKind::Final, 0, None, Some(560_000))
            );
            let progress = session.progress_snapshot();
            assert_eq!(
                (
                    progress.audio_received,
                    progress.audio_sent,
                    progress.prefix_blocks
                ),
                (28, 26, 2)
            );
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phase11_openai_burst_yields_to_inline_transcript_consumer() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let updates = super::super::speech::EVENT_QUEUE_ITEMS + 16;
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "input_audio_buffer.commit" {
                socket
                    .feed(Message::Text(
                        String::from_utf8(ack("burst")).unwrap().into(),
                    ))
                    .await
                    .unwrap();
                for _ in 0..updates {
                    socket
                        .feed(Message::Text(
                            String::from_utf8(delta("burst", "x")).unwrap().into(),
                        ))
                        .await
                        .unwrap();
                }
                socket.feed(Message::Text(json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"burst","content_index":0,"transcript":"x".repeat(updates)}).to_string().into())).await.unwrap();
                socket.flush().await.unwrap();
            }
        }
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::OpenAi,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            for sequence in 0..26 {
                session
                    .input
                    .try_push(crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp {
                            session: identity().session,
                            sequence,
                            frame_position: Some(sequence * BLOCK_FRAMES as u64),
                            qpc_100ns: None,
                            received: Instant::now(),
                            discontinuity: false,
                        },
                        frames: BLOCK_FRAMES,
                        samples: [if sequence < 6 { 0.1 } else { 0.0 }; BLOCK_FRAMES],
                    })
                    .unwrap();
                if sequence < 25 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
            let mut partials = 0;
            let mut final_seen = false;
            while let Some(event) = session.next().await {
                if event.kind == TranscriptKind::Final {
                    assert_eq!(event.text.len(), updates);
                    final_seen = true;
                    cancel.cancel();
                    break;
                }
                partials += 1;
            }
            (partials, final_seen)
        };
        let (result, seen) = tokio::join!(runner, driver);
        assert_eq!(
            result,
            Err(SpeechError::Cancelled),
            "a ready socket must give its inline consumer a polling opportunity"
        );
        assert_eq!(seen, (updates, true));
    };
    tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[test]
fn phase11_openai_quiet_prefix_is_sent_once_with_its_real_capture_context() {
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    let mut quiet = block(0, false, false);
    quiet.active = false;
    quiet.audio.samples.fill(16);
    assert!(protocol.audio(quiet).unwrap().is_empty());
    let mut voice = block(1, true, false);
    voice.audio.samples.fill(2000);
    let messages = protocol.audio(voice).unwrap();
    let bytes = STANDARD
        .decode(messages[0]["audio"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        bytes.len(),
        960 * 2,
        "the quiet prefix must not disappear at the activity threshold"
    );
    assert!(
        bytes.as_chunks::<2>().0[..480]
            .iter()
            .all(|sample| i16::from_le_bytes(*sample) == 16)
    );
    assert!(
        bytes.as_chunks::<2>().0[480..]
            .iter()
            .all(|sample| i16::from_le_bytes(*sample) == 2000)
    );
    let turn = protocol.current.as_ref().unwrap();
    assert_eq!(
        (turn.context.sequence, turn.context.start_us, turn.frames),
        (0, None, 960)
    );
    let next = protocol.audio(block(2, false, true)).unwrap();
    assert_eq!(
        STANDARD
            .decode(next[0]["audio"].as_str().unwrap())
            .unwrap()
            .len(),
        480 * 2
    );
    assert_eq!(
        protocol.pending.front().unwrap().0.duration_us,
        Some(60_000)
    );
}
#[tokio::test]
async fn phase11_openai_early_real_socket_streams_pcm_until_ack_then_delivers_and_cancels() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut blocks = 0;
        let mut commits = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["type"] == "input_audio_buffer.append" {
                blocks += 1;
                if blocks == 1 {
                    for text in ["synthetic first ", "prefix"] {
                        send_json(
                            &mut socket,
                            serde_json::from_slice(&delta("early-real", text)).unwrap(),
                        )
                        .await
                        .unwrap();
                    }
                }
            } else if value["type"] == "input_audio_buffer.commit" {
                commits += 1;
                assert_eq!(blocks, 26, "early replies cannot stop fresh PCM");
                send_json(
                    &mut socket,
                    serde_json::from_slice(&ack("early-real")).unwrap(),
                )
                .await
                .unwrap();
                send_json(&mut socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"early-real","content_index":0,"transcript":"synthetic final"})).await.unwrap();
            }
        }
        assert_eq!((blocks, commits), (26, 1));
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::OpenAi,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            for sequence in 0..26 {
                session
                    .input
                    .try_push(crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp {
                            session: identity().session,
                            sequence,
                            frame_position: Some(sequence * BLOCK_FRAMES as u64),
                            qpc_100ns: None,
                            received: Instant::now(),
                            discontinuity: false,
                        },
                        frames: BLOCK_FRAMES,
                        samples: [if sequence < 6 { 0.1 } else { 0.0 }; BLOCK_FRAMES],
                    })
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                if sequence < 24 {
                    assert_eq!(session.event_queue_depth(), 0);
                }
            }
            let partial = session.next().await.unwrap();
            let final_event = session.next().await.unwrap();
            assert_eq!(
                (partial.kind, partial.text.as_ref()),
                (TranscriptKind::Partial, "synthetic first prefix")
            );
            assert_eq!(
                (final_event.kind, final_event.text.as_ref()),
                (TranscriptKind::Final, "synthetic final")
            );
            assert_eq!(partial.duration_us, Some(520_000));
            assert_eq!(
                (partial.identity, final_event.identity),
                (identity(), identity())
            );
            assert!(partial.start_us.is_none() && final_event.start_us.is_none());
            let progress = session.progress_snapshot();
            assert_eq!(
                (progress.early_held, progress.emitted, progress.audio_sent),
                (2, 2, 26)
            );
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(server, client);
    })
    .await
    .expect("early transcript frames must preserve the live audio/cancellation path");
}

#[tokio::test]
async fn phase11_openai_real_socket_error_before_audio_keeps_stage_code_and_no_private_message() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        send_json(&mut socket, json!({"type":"error","error":{"code":"invalid_value","message":"private-fixture","param":"private-fixture"}})).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                assert_ne!(
                    serde_json::from_str::<Value>(&text).unwrap()["type"],
                    "input_audio_buffer.append"
                );
            }
        }
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let error = run_socket(
            socket,
            SpeechProvider::OpenAi,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            session.cancellation(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.root_cause(), &SpeechError::InvalidData);
        assert!(matches!(
            error,
            SpeechError::Connection {
                stage: ConnectionStage::StreamEvent,
                ..
            }
        ));
        assert!(error.to_string().contains("invalid_value"));
        assert!(!format!("{error:?}").contains("private-fixture"));
        assert_eq!(
            (
                session.progress_snapshot().audio_received,
                session.progress_snapshot().audio_sent
            ),
            (0, 0)
        );
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[test]
fn phase11_openai_early_unmatched_reply_has_a_finite_correlation_deadline() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    protocol.audio(block(1, true, false)).unwrap();
    protocol
        .receive(&delta("unmatched", "fixture"), task.output_ref_for_test())
        .unwrap();
    assert_eq!(
        protocol.expiration_error(Instant::now() + FINAL_TIMEOUT + Duration::from_millis(1)),
        Some(SpeechError::Connection {
            stage: ConnectionStage::CorrelateTranscript,
            cause: Box::new(SpeechError::Timeout)
        })
    );
    assert_eq!(session.event_queue_depth(), 0);
}

#[tokio::test]
async fn phase11_openai_early_replies_follow_ack_identity_instead_of_arrival_order() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    protocol.audio(block(1, true, true)).unwrap();
    protocol.audio(block(2, true, true)).unwrap();
    protocol
        .receive(
            &delta("second", "synthetic second"),
            task.output_ref_for_test(),
        )
        .unwrap();
    protocol
        .receive(
            &delta("first", "synthetic first"),
            task.output_ref_for_test(),
        )
        .unwrap();
    assert_eq!(session.event_queue_depth(), 0);
    protocol
        .receive(&ack("first"), task.output_ref_for_test())
        .unwrap();
    let first = session.next().await.unwrap();
    assert_eq!(
        (first.utterance, first.sequence, first.text.as_ref()),
        (1, 1, "synthetic first")
    );
    protocol
        .receive(&ack("second"), task.output_ref_for_test())
        .unwrap();
    let second = session.next().await.unwrap();
    assert_eq!(
        (second.utterance, second.sequence, second.text.as_ref()),
        (2, 2, "synthetic second")
    );
}

#[tokio::test]
async fn phase11_openai_early_final_is_not_delivered_before_ack_and_unsolicited_text_is_rejected() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    assert_eq!(
        protocol.receive(&delta("unsolicited", "fixture"), task.output_ref_for_test()),
        Err(SpeechError::InvalidData)
    );
    protocol.audio(block(1, true, true)).unwrap();
    protocol
        .receive(
            &delta("item", "synthetic partial"),
            task.output_ref_for_test(),
        )
        .unwrap();
    protocol
        .receive(&final_text("item", "en"), task.output_ref_for_test())
        .unwrap();
    assert_eq!(session.event_queue_depth(), 0);
    protocol
        .receive(&ack("item"), task.output_ref_for_test())
        .unwrap();
    assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Partial);
    assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Final);
    assert!(protocol.awaiting_final.is_empty());
    assert_eq!(session.progress_snapshot().early_held, 2);
}

#[test]
fn phase11_openai_data_errors_keep_only_safe_stage_and_allowlisted_codes() {
    use crate::ai::speech::RequestErrorCode;
    let cases = [
        ("insufficient_quota", RequestErrorCode::InsufficientQuota),
        ("invalid_request_error", RequestErrorCode::InvalidRequest),
        ("invalid_value", RequestErrorCode::InvalidValue),
        ("unknown_parameter", RequestErrorCode::UnknownParameter),
        (
            "unsupported_parameter",
            RequestErrorCode::UnsupportedParameter,
        ),
        ("model_not_found", RequestErrorCode::ModelNotFound),
        (
            "input_audio_buffer_commit_empty",
            RequestErrorCode::EmptyCommit,
        ),
    ];
    for (code, expected) in cases {
        let error = check_service_error(
            &json!({"error":{"code":code,"message":"private-fixture","param":"private-fixture"}}),
        )
        .unwrap_err();
        assert_eq!(
            error,
            SpeechError::RequestRejected {
                code: expected,
                cause: Box::new(SpeechError::InvalidData)
            }
        );
        let error = runtime_data_error(SpeechProvider::OpenAi, ConnectionStage::StreamEvent, error);
        assert_eq!(error.root_cause(), &SpeechError::InvalidData);
        assert!(!error.retryable());
        assert!(error.to_string().contains(code));
        assert!(error.to_string().contains("處理辨識事件"));
        assert!(!format!("{error:?}").contains("private-fixture"));
    }
    assert_eq!(
        check_service_error(
            &json!({"error":{"code":"private-fixture","message":"private-fixture"}})
        ),
        Err(SpeechError::InvalidData)
    );
    assert_eq!(
        runtime_data_error(
            SpeechProvider::Gemini,
            ConnectionStage::StreamEvent,
            SpeechError::InvalidData
        ),
        SpeechError::InvalidData
    );
}

#[tokio::test]
async fn phase11_openai_early_reply_waits_for_actual_commit_correlation() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    protocol.audio(block(1, true, false)).unwrap();
    assert_eq!(
        protocol.receive(
            &delta("early-item", "synthetic first "),
            task.output_ref_for_test()
        ),
        Ok(())
    );
    assert_eq!(
        session.event_queue_depth(),
        0,
        "unmatched text must not acquire capture identity"
    );
    protocol.audio(block(2, false, false)).unwrap();
    protocol
        .receive(&delta("early-item", "prefix"), task.output_ref_for_test())
        .unwrap();
    protocol.audio(block(3, false, true)).unwrap();
    protocol
        .receive(&ack("early-item"), task.output_ref_for_test())
        .unwrap();
    let event = session.next().await.unwrap();
    assert_eq!(
        (event.kind, event.utterance, event.sequence),
        (TranscriptKind::Partial, 1, 1)
    );
    assert_eq!(event.text.as_ref(), "synthetic first prefix");
    assert_eq!(event.duration_us, Some(60_000));
    protocol
        .receive(&final_text("early-item", "en"), task.output_ref_for_test())
        .unwrap();
    assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Final);
}
#[test]
fn phase11_auto_hybrid_endpoints_request_final_before_any_transcript() {
    let mut protocol = Protocol::for_model(
        LiveSpeechModel::GeminiTranscribeLive,
        SourceLanguage::Auto,
        identity(),
    );
    assert_eq!(protocol.audio(block(1, true, false)).unwrap().len(), 1);
    let messages = protocol.audio(block(2, false, true)).unwrap();
    assert_eq!(
        messages.len(),
        2,
        "endpoint must reach the service even before its first partial"
    );
    assert!(messages[0]["realtimeInput"].get("audio").is_some());
    assert_eq!(messages[1]["realtimeInput"]["audioStreamEnd"], true);
    assert!(protocol.current.is_none());
    assert!(protocol.last_gemini_audio.is_none());
    assert!(protocol.held_since.is_none());
}
const LOCAL_JAPANESE: &str =
    "今日は図書館で新しい本を借りました。家に帰ってからゆっくり読もうと思っています。";
#[test]
fn phase11_auto_hybrid_endpoint_before_partial_deduplicates_later_language_wait() {
    let mut protocol = Protocol::for_model(
        LiveSpeechModel::GeminiTranscribeLive,
        SourceLanguage::Auto,
        identity(),
    );
    assert_eq!(protocol.audio(block(1, true, true)).unwrap().len(), 2);
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    protocol
        .receive(
            br#"{"serverContent":{"interimInputTranscription":{"text":"12345"}}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    assert_eq!(session.event_queue_depth(), 0);
    protocol.held_since = Some(Instant::now() - AUTO_LANGUAGE_FINALIZE_AFTER);
    assert!(protocol.language_input_end(Instant::now()).is_none());
    // Ongoing capture is still accepted while the actual server final is pending.
    assert_eq!(protocol.audio(block(2, true, false)).unwrap().len(), 1);
    assert_eq!(protocol.audio(block(3, false, true)).unwrap().len(), 1);
    assert!(protocol.current.is_some());
    assert!(protocol.held_since.is_some());
}

#[test]
fn phase11_auto_hybrid_quiet_does_not_repeat_and_new_audio_reopens() {
    let mut protocol = Protocol::for_model(
        LiveSpeechModel::GeminiTranscribeLive,
        SourceLanguage::Auto,
        identity(),
    );
    protocol.audio(block(1, true, true)).unwrap();
    for sequence in 2..100 {
        let mut quiet = block(sequence, false, false);
        quiet.active = false;
        assert_eq!(protocol.audio(quiet).unwrap().len(), 1);
    }
    assert!(
        protocol
            .idle_input(Instant::now() + GEMINI_INPUT_IDLE, true)
            .is_none()
    );
    assert_eq!(protocol.audio(block(100, true, false)).unwrap().len(), 1);
    assert!(protocol.last_gemini_audio.is_some());
    assert_eq!(protocol.audio(block(101, false, true)).unwrap().len(), 2);
    assert!(protocol.current.is_none()); // Client endpoints cannot invent server utterances.
}

#[test]
fn phase11_auto_hybrid_deduplicates_language_end_and_keeps_response_deadline() {
    let mut protocol = Protocol::for_model(
        LiveSpeechModel::GeminiTranscribeLive,
        SourceLanguage::Auto,
        identity(),
    );
    protocol.audio(block(1, true, false)).unwrap();
    protocol.held_since = Some(Instant::now() - AUTO_LANGUAGE_FINALIZE_AFTER);
    assert!(protocol.language_input_end(Instant::now()).is_some());
    assert_eq!(protocol.audio(block(2, false, true)).unwrap().len(), 1);
    assert!(protocol.last_gemini_audio.is_none());
    assert!(protocol.language_input_end(Instant::now()).is_none());

    protocol.held_since = None;
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    protocol.receive(br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic English","languageCode":"en-US"}}}"#, task.output_ref_for_test()).unwrap();
    assert_eq!(session.event_queue_depth(), 1);
    let expired = Instant::now() - FINAL_TIMEOUT - Duration::from_millis(1);
    protocol.current.as_mut().unwrap().last_response = Some(expired);
    protocol.audio(block(3, true, true)).unwrap();
    assert_eq!(
        protocol.expiration_error(Instant::now()),
        Some(SpeechError::Timeout)
    );
    assert_eq!(
        protocol.current.as_ref().unwrap().last_response,
        Some(expired)
    );
}

#[test]
fn phase11_auto_hybrid_is_explicitly_scoped_to_transcribe_auto() {
    for model in [
        LiveSpeechModel::GeminiTranscribeLive,
        LiveSpeechModel::GeminiLiveTranslate,
        LiveSpeechModel::GptLiveTranscribe,
    ] {
        for language in [
            SourceLanguage::Auto,
            SourceLanguage::English,
            SourceLanguage::Japanese,
            SourceLanguage::French,
            SourceLanguage::German,
            SourceLanguage::Korean,
        ] {
            let mut protocol = Protocol::for_model(model, language, identity());
            assert_eq!(
                protocol.client_endpointing,
                model == LiveSpeechModel::GeminiTranscribeLive && language == SourceLanguage::Auto
            );
            let messages = protocol.audio(block(1, true, true)).unwrap();
            if model == LiveSpeechModel::GeminiLiveTranslate
                || (model == LiveSpeechModel::GeminiTranscribeLive
                    && language != SourceLanguage::Auto)
            {
                assert_eq!(messages.len(), 1);
                assert!(messages[0]["realtimeInput"].get("audio").is_some());
            }
        }
    }
}

#[tokio::test]
async fn phase11_auto_hybrid_socket_finalizes_before_first_text_and_after_quiet_without_language_leak()
 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut endings = 0;
        let mut blocks = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["realtimeInput"].get("audio").is_some() {
                blocks += 1; // Deliberately no transcript until the client requests a final.
            } else if value["realtimeInput"]["audioStreamEnd"] == true {
                assert!(endings < 2, "quiet PCM must not send another endpoint");
                let text = if endings == 0 {
                    LOCAL_ENGLISH
                } else {
                    LOCAL_JAPANESE
                };
                send_json(
                    &mut socket,
                    serde_json::from_slice(&local_payload("inputTranscription", text)).unwrap(),
                )
                .await
                .unwrap();
                endings += 1;
            }
        }
        assert_eq!((blocks, endings), (112, 2));
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::Gemini,
            SourceLanguage::Auto,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            let mut sequence = 0;
            for (index, language) in [SpeechLanguage::English, SpeechLanguage::Japanese]
                .into_iter()
                .enumerate()
            {
                // Six active packets followed by exactly the existing 400 ms quiet endpoint.
                for packet in 0..26 {
                    session
                        .input
                        .try_push(crate::audio::preprocess::NormalizedAudio {
                            stamp: AudioStamp {
                                session: identity().session,
                                sequence,
                                frame_position: Some(sequence * BLOCK_FRAMES as u64),
                                qpc_100ns: None,
                                received: Instant::now(),
                                discontinuity: false,
                            },
                            frames: BLOCK_FRAMES,
                            samples: [if packet < 6 { 0.1 } else { 0.0 }; BLOCK_FRAMES],
                        })
                        .unwrap();
                    sequence += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let event = session.next().await.unwrap();
                assert_eq!(
                    (event.kind, event.language, event.utterance),
                    (TranscriptKind::Final, Some(language), index as u64 + 1)
                );
                assert!(event.start_us.is_none() && event.duration_us.is_none());
                assert_eq!(session.progress_snapshot().audio_ends, index as u64 + 1);
                if index == 0 {
                    // Keep sending silent packets beyond the transport-idle threshold.
                    for _ in 0..60 {
                        session
                            .input
                            .try_push(crate::audio::preprocess::NormalizedAudio {
                                stamp: AudioStamp {
                                    session: identity().session,
                                    sequence,
                                    frame_position: Some(sequence * BLOCK_FRAMES as u64),
                                    qpc_100ns: None,
                                    received: Instant::now(),
                                    discontinuity: false,
                                },
                                frames: BLOCK_FRAMES,
                                samples: [0.0; BLOCK_FRAMES],
                            })
                            .unwrap();
                        sequence += 1;
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    assert_eq!(session.progress_snapshot().audio_ends, 1);
                }
            }
            let progress = session.progress_snapshot();
            assert_eq!(
                (progress.finals, progress.emitted, progress.partials),
                (2, 2, 0)
            );
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(8), async {
        tokio::join!(server, client);
    })
    .await
    .expect("client endpoints must work without waiting for the server's first transcript");
}

fn local_payload(field: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"serverContent":{field:{"text":text}}})).unwrap()
}
#[tokio::test]
async fn phase11_soft_input_socket_sends_boosted_pcm_in_auto_and_explicit_modes() {
    for language in [SourceLanguage::Auto, SourceLanguage::English] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut blocks = 0;
            while let Some(Ok(message)) = socket.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let value: Value = serde_json::from_str(&text).unwrap();
                let Some(encoded) = value["realtimeInput"]["audio"]["data"].as_str() else {
                    continue;
                };
                let bytes = STANDARD.decode(encoded).unwrap();
                assert!(bytes.as_chunks::<2>().1.is_empty());
                let peak = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|sample| i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs())
                    .max()
                    .unwrap();
                assert!(
                    peak > 160 && peak < 400,
                    "wire PCM must contain the bounded gain, not merely new activity flags"
                );
                blocks += 1;
                assert_eq!(blocks, 1);
                send_json(
                    &mut socket,
                    serde_json::from_slice(&local_payload("inputTranscription", LOCAL_ENGLISH))
                        .unwrap(),
                )
                .await
                .unwrap();
            }
            assert_eq!(blocks, 1);
        };
        let client = async {
            let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap();
            let (mut session, mut task) = SpeechSession::new(identity(), language, caps()).unwrap();
            let cancel = session.cancellation();
            let runner = run_socket(
                socket,
                SpeechProvider::Gemini,
                language,
                task.take_audio_for_test(),
                task.take_output_for_test(),
                cancel.clone(),
            );
            let driver = async {
                let mut input = crate::audio::preprocess::NormalizedAudio {
                    stamp: AudioStamp {
                        session: identity().session,
                        sequence: 0,
                        frame_position: Some(0),
                        qpc_100ns: None,
                        received: Instant::now(),
                        discontinuity: false,
                    },
                    frames: BLOCK_FRAMES,
                    samples: [0.0; BLOCK_FRAMES],
                };
                for (index, sample) in input.samples.iter_mut().enumerate() {
                    *sample = 0.0005
                        * (index as f32 * std::f32::consts::TAU * 500.0
                            / crate::audio::capture::SAMPLE_RATE as f32)
                            .sin();
                }
                session.input.try_push(input).unwrap();
                let event = session.next().await.unwrap();
                assert_eq!(
                    (event.kind, event.language),
                    (
                        TranscriptKind::Final,
                        if language == SourceLanguage::Auto {
                            Some(SpeechLanguage::English)
                        } else {
                            None
                        }
                    ),
                    "gain must not fabricate service language metadata in {language:?}"
                );
                assert!(event.start_us.is_none() && event.duration_us.is_none());
                let progress = session.progress_snapshot();
                assert_eq!(
                    (progress.boosted, progress.audio_sent, progress.emitted),
                    (1, 1, 1)
                );
                cancel.cancel();
            };
            let (result, ()) = tokio::join!(runner, driver);
            assert_eq!(result, Err(SpeechError::Cancelled));
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, client);
        })
        .await
        .expect("bounded gain must reach both explicit and Auto wire paths");
    }
}

async fn receive_local(
    protocol: &mut Protocol,
    session: &SpeechSession,
    task: &crate::ai::speech::SpeechTask,
    payload: &[u8],
) -> Result<(), SpeechError> {
    let local = protocol
        .assess_missing_language(payload, identity(), &session.cancellation())
        .await?;
    protocol.receive_assessed(payload, task.output_ref_for_test(), &local)
}
async fn next_local(session: &mut SpeechSession) -> crate::ai::speech::TranscriptEvent {
    tokio::time::timeout(Duration::from_secs(2), session.next())
        .await
        .expect("local language output deadline")
        .expect("local language output closed")
}

#[tokio::test]
async fn phase11_local_language_auto_handles_missing_tags_skips_uncertain_and_reassesses_each_utterance()
 {
    let (mut session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("interimInputTranscription", LOCAL_ENGLISH),
    )
    .await
    .unwrap();
    let partial = next_local(&mut session).await;
    assert_eq!(partial.language, Some(SpeechLanguage::English));
    assert!(partial.start_us.is_none() && partial.duration_us.is_none());
    assert!(
        protocol
            .current
            .as_ref()
            .unwrap()
            .context
            .language
            .is_none()
    ); // Not a provider tag.
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("inputTranscription", LOCAL_ENGLISH),
    )
    .await
    .unwrap();
    assert_eq!(next_local(&mut session).await.kind, TranscriptKind::Final);
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("interimInputTranscription", "ok"),
    )
    .await
    .unwrap();
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("inputTranscription", "ok"),
    )
    .await
    .unwrap();
    assert_eq!(session.event_queue_depth(), 0);
    assert!(
        protocol.current.is_none() && protocol.held.is_empty() && protocol.held_since.is_none()
    );
    assert_eq!(session.progress_snapshot().language_skipped, 1);
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("inputTranscription", LOCAL_JAPANESE),
    )
    .await
    .unwrap();
    let next = next_local(&mut session).await;
    assert_eq!(
        (next.utterance, next.language),
        (3, Some(SpeechLanguage::Japanese))
    );
    assert_eq!(session.progress_snapshot().local_language_used, 3);
}

#[tokio::test]
async fn phase11_local_language_rechecks_revisions_and_does_not_backfill_unassessed_partials() {
    let (mut session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("interimInputTranscription", "unknown"),
    )
    .await
    .unwrap();
    assert_eq!(session.event_queue_depth(), 0);
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("interimInputTranscription", LOCAL_ENGLISH),
    )
    .await
    .unwrap();
    assert_eq!(session.event_queue_depth(), 1); // The earlier unassessed text is dropped, not labelled English.
    assert_eq!(next_local(&mut session).await.text.as_ref(), LOCAL_ENGLISH);
    receive_local(
        &mut protocol,
        &session,
        &task,
        &local_payload("inputTranscription", LOCAL_JAPANESE),
    )
    .await
    .unwrap();
    assert_eq!(
        next_local(&mut session).await.language,
        Some(SpeechLanguage::Japanese)
    );
}

#[tokio::test]
async fn phase11_local_language_never_overrides_present_provider_metadata_or_old_deadlines() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    let payload = local_payload("inputTranscription", LOCAL_ENGLISH);
    let local = protocol
        .assess_missing_language(&payload, identity(), &session.cancellation())
        .await
        .unwrap();
    for code in [json!("es-ES"), json!("unknown"), json!(13)] {
        let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
        protocol.audio(block(1, false, false)).unwrap();
        let tagged=serde_json::to_vec(&json!({"serverContent":{"inputTranscription":{"text":LOCAL_ENGLISH,"languageCode":code}}})).unwrap();
        assert!(
            protocol
                .receive_assessed(&tagged, task.output_ref_for_test(), &local)
                .is_err()
        );
        assert_eq!(session.event_queue_depth(), 0);
    }
    protocol
        .receive(
            br#"{"serverContent":{"interimInputTranscription":{"text":"unknown"}}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    protocol.held_since = Some(Instant::now() - AUTO_LANGUAGE_TIMEOUT - Duration::from_millis(1));
    assert_eq!(
        protocol.receive_assessed(&payload, task.output_ref_for_test(), &local),
        Err(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 0);
}

#[tokio::test]
async fn phase11_local_language_does_not_force_foreign_or_mismatched_evidence_into_the_five() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    for text in [
        "Esta mañana vamos a visitar la biblioteca para leer algunos libros interesantes y hablar con nuestros amigos.",
        "這是一段中文測試文字，用來確認不會把中文強制當成英文或其他支援語言。",
    ] {
        let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
        protocol.audio(block(1, false, false)).unwrap();
        let result = receive_local(
            &mut protocol,
            &session,
            &task,
            &local_payload("inputTranscription", text),
        )
        .await;
        assert!(result.is_ok() || result == Err(SpeechError::UnsupportedLanguage));
        assert_eq!(session.event_queue_depth(), 0);
    }
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    let good = local_payload("inputTranscription", LOCAL_ENGLISH);
    let old = StreamIdentity {
        generation: 2,
        ..identity()
    };
    let local = protocol
        .assess_missing_language(&good, old, &session.cancellation())
        .await
        .unwrap();
    assert_eq!(
        protocol.receive_assessed(&good, task.output_ref_for_test(), &local),
        Err(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 0);
    // Even identical text in the next utterance must get a new message-bound assessment.
    let mut repeat = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    repeat.audio(block(1, false, false)).unwrap();
    let evidence = repeat
        .assess_missing_language(&good, identity(), &session.cancellation())
        .await
        .unwrap();
    repeat
        .receive_assessed(&good, task.output_ref_for_test(), &evidence)
        .unwrap();
    assert_eq!(session.event_queue_depth(), 1);
    assert_eq!(
        repeat.receive_assessed(&good, task.output_ref_for_test(), &evidence),
        Err(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 1);
}

#[tokio::test]
async fn phase11_local_language_real_socket_delivers_two_languages_without_service_tags_then_cancels()
 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut endings = 0;
        let mut blocks = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            let source = if endings == 0 {
                LOCAL_ENGLISH
            } else {
                LOCAL_JAPANESE
            };
            if value["realtimeInput"].get("audio").is_some() {
                blocks += 1;
                socket
                    .send(Message::Binary(
                        local_payload("interimInputTranscription", source).into(),
                    ))
                    .await
                    .unwrap();
            } else if value["realtimeInput"]["audioStreamEnd"] == true {
                send_json(
                    &mut socket,
                    serde_json::from_slice(&local_payload("inputTranscription", source)).unwrap(),
                )
                .await
                .unwrap();
                endings += 1;
            }
        }
        assert_eq!((blocks, endings), (2, 2));
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::Gemini,
            SourceLanguage::Auto,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            for (sequence, language) in
                [(0, SpeechLanguage::English), (1, SpeechLanguage::Japanese)]
            {
                session
                    .input
                    .try_push(crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp {
                            session: identity().session,
                            sequence,
                            frame_position: Some(sequence * BLOCK_FRAMES as u64),
                            qpc_100ns: None,
                            received: Instant::now(),
                            discontinuity: false,
                        },
                        frames: BLOCK_FRAMES,
                        samples: [0.1; BLOCK_FRAMES],
                    })
                    .unwrap();
                for kind in [TranscriptKind::Partial, TranscriptKind::Final] {
                    let event = session.next().await.unwrap();
                    assert_eq!((event.kind, event.language), (kind, Some(language)));
                    assert!(event.start_us.is_none() && event.duration_us.is_none());
                }
            }
            assert_eq!(session.progress_snapshot().local_language_used, 4);
            assert_eq!(session.progress_snapshot().language_skipped, 0);
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn phase11_live_translate_real_local_socket_uses_native_model_and_caption_lane_then_cancels()
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let Message::Text(setup) = socket.next().await.unwrap().unwrap() else {
            panic!("setup must be JSON");
        };
        let setup: Value = serde_json::from_str(&setup).unwrap();
        assert_eq!(
            setup["setup"]["model"],
            "models/gemini-3.5-live-translate-preview"
        );
        socket
            .send(Message::Binary(
                serde_json::to_vec(&json!({"setupComplete":{}}))
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        let mut sent = false;
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["realtimeInput"].get("audio").is_some() && !sent {
                    sent = true;
                    send_json(&mut socket,json!({"serverContent":{"inputTranscription":{"text":"synthetic English","languageCode":"en"},"outputTranscription":{"text":"合成中文","languageCode":"zh-Hant"},"modelTurn":{"parts":[{"inlineData":{"data":"not-decoded","mimeType":"audio/pcm;rate=24000"}}]},"generationComplete":true}})).await.unwrap();
                }
            }
        }
        assert!(sent);
    };
    let client = async {
        let request = format!("ws://{address}").into_client_request().unwrap();
        let recognizer = NetworkRecognizer::connect_request_model(
            LiveSpeechModel::GeminiLiveTranslate,
            request,
            SourceLanguage::English,
            TranslationQuality::Standard,
        )
        .await
        .unwrap();
        let (mut session, task) = SpeechSession::new(
            identity(),
            SourceLanguage::English,
            recognizer.capabilities(),
        )
        .unwrap();
        let mut captions = session.take_captions().unwrap();
        assert!(session.take_captions().is_none());
        let cancel = session.cancellation();
        let runner = task.run(&recognizer);
        tokio::pin!(runner);
        let mut feed = tokio::time::interval(Duration::from_millis(10));
        let mut sequence = 0;
        loop {
            tokio::select! {biased;
                result=&mut runner=>panic!("unexpected early completion: {result:?}"),
                caption=captions.recv()=>{let caption=caption.unwrap();assert_eq!(caption.text.as_ref(),"合成中文");assert_eq!(caption.kind,TranscriptKind::Final);assert_eq!(caption.identity,identity());assert_eq!(session.event_queue_depth(),0);cancel.cancel();break;},
                _=feed.tick()=>{session.input.try_push(crate::audio::preprocess::NormalizedAudio{stamp:AudioStamp{session:identity().session,sequence,frame_position:Some(sequence*480),qpc_100ns:None,received:Instant::now(),discontinuity:false},frames:480,samples:[0.1;BLOCK_FRAMES]}).unwrap();sequence+=1;},
            }
        }
        assert_eq!(runner.await, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}
fn block(sequence: u64, begin: bool, end: bool) -> PreparedBlock {
    PreparedBlock {
        audio: ResampledAudio {
            stamp: AudioStamp {
                session: identity().session,
                sequence,
                frame_position: Some(sequence * BLOCK_FRAMES as u64),
                qpc_100ns: None,
                received: Instant::now(),
                discontinuity: false,
            },
            sample_rate: 24000,
            frames: 480,
            samples: [0; 480],
            first_input_offset: 0,
        },
        active: true,
        begin,
        end,
        boosted: false,
    }
}
fn delta(key: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"type":"conversation.item.input_audio_transcription.delta","item_id":key,"content_index":0,"delta":text})).unwrap()
}
fn final_text(key: &str, language: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"type":"conversation.item.input_audio_transcription.completed","item_id":key,"content_index":0,"transcript":"fixture final","languages":[{"code":language}]})).unwrap()
}
fn ack(key: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"type":"input_audio_buffer.committed","item_id":key})).unwrap()
}
#[tokio::test]
async fn openai_commit_correlates_audio_and_auto_waits_for_supported_language() {
    let (mut session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let output = task.output_ref_for_test();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::Auto, identity());
    let messages = protocol.audio(block(1, true, true)).unwrap();
    assert_eq!(messages[0]["type"], "input_audio_buffer.append");
    assert_eq!(messages[1]["type"], "input_audio_buffer.commit");
    protocol.receive(&ack("item-A"), output).unwrap();
    protocol
        .receive(&delta("item-A", "fixture partial"), output)
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), session.next())
            .await
            .is_err()
    );
    protocol
        .receive(&final_text("item-A", "en"), output)
        .unwrap();
    let partial = session.next().await.unwrap();
    let final_event = session.next().await.unwrap();
    assert_eq!(partial.kind, TranscriptKind::Partial);
    assert_eq!(final_event.kind, TranscriptKind::Final);
    assert_eq!(final_event.sequence, 1);
    assert_eq!(final_event.duration_us, Some(20000));
    assert_eq!(final_event.language, Some(SpeechLanguage::English));
    assert!(protocol.awaiting_final.is_empty());
}
#[tokio::test]
async fn phase11_gemini_auto_accepts_supported_regional_language_tags() {
    for (code, language) in [
        ("en-GB", SpeechLanguage::English),
        ("EN-us", SpeechLanguage::English),
        ("fr-CA", SpeechLanguage::French),
        ("de-AT", SpeechLanguage::German),
        ("ja-Jpan-JP", SpeechLanguage::Japanese),
        ("ko-Kore-KR", SpeechLanguage::Korean),
    ] {
        let (mut session, task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
        protocol.audio(block(1, false, false)).unwrap();
        protocol.receive(&serde_json::to_vec(&json!({"serverContent":{"inputTranscription":{"text":"synthetic source","languageCode":code}}})).unwrap(), task.output_ref_for_test()).unwrap();
        let event = session.next().await.unwrap();
        assert_eq!(event.language, Some(language));
        assert_eq!(event.kind, TranscriptKind::Final);
        assert!(event.start_us.is_none() && event.duration_us.is_none());
    }
}

#[tokio::test]
async fn phase11_auto_unknown_language_partials_have_a_total_wait_budget() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    let payload = br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic unverified partial"}}}"#;
    protocol
        .receive(payload, task.output_ref_for_test())
        .unwrap();
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        protocol
            .receive(payload, task.output_ref_for_test())
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        protocol.expired(),
        "repeated unknown-language partials renewed the wait forever"
    );
    assert_eq!(
        protocol.receive(payload, task.output_ref_for_test()),
        Err(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 0);
}

#[test]
fn phase11_auto_language_wait_is_retired_with_the_utterance_and_rejects_late_detection() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    let unknown =
        br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic unknown"}}}"#;
    let known = br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic known","languageCode":"en-GB"}}}"#;
    protocol
        .receive(unknown, task.output_ref_for_test())
        .unwrap();
    assert!(session.progress_snapshot().waiting_language);
    protocol.held_since = Some(Instant::now() - Duration::from_secs(7));
    protocol.receive(known, task.output_ref_for_test()).unwrap();
    assert!(protocol.held_since.is_none());
    assert!(!session.progress_snapshot().waiting_language);
    protocol
        .receive(
            br#"{"serverContent":{"inputTranscription":{"text":"synthetic final"}}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    assert_eq!(session.event_queue_depth(), 3);
    protocol
        .receive(unknown, task.output_ref_for_test())
        .unwrap();
    assert!(protocol.held_since.is_some());
    assert!(session.progress_snapshot().waiting_language);
    protocol.held_since = Some(Instant::now() - AUTO_LANGUAGE_TIMEOUT - Duration::from_millis(1));
    assert_eq!(
        protocol.expiration_error(Instant::now()),
        Some(SpeechError::LanguageNotReported)
    );
    assert_eq!(
        protocol.receive(known, task.output_ref_for_test()),
        Err(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 3);
}

#[tokio::test]
async fn phase11_auto_real_local_socket_reports_audio_service_and_language_wait_without_text() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (allow_final, final_allowed) = tokio::sync::oneshot::channel();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut first = true;
        let mut final_allowed = Box::pin(final_allowed);
        let mut final_sent = false;
        loop {
            tokio::select! {
                result=&mut final_allowed, if !final_sent => {
                    result.unwrap();final_sent=true;
                    socket.send(Message::Binary(br#"{"serverContent":{"inputTranscription":{"text":"synthetic final","languageCode":"en-GB"}}}"#.to_vec().into())).await.unwrap();
                },
                message=socket.next()=>{
                    let Some(Ok(message))=message else {break;};
                    if let Message::Text(text)=message {
                        let value:Value=serde_json::from_str(&text).unwrap();
                        if value["realtimeInput"].get("audio").is_some() && first {
                            first=false;
                            send_json(&mut socket,json!({"serverContent":{"interimInputTranscription":{"text":"12345"}}})).await.unwrap();
                        }
                    }
                }
            }
        }
        assert!(!first && final_sent);
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::Gemini,
            SourceLanguage::Auto,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            session
                .input
                .try_push(crate::audio::preprocess::NormalizedAudio {
                    stamp: AudioStamp {
                        session: identity().session,
                        sequence: 0,
                        frame_position: Some(0),
                        qpc_100ns: None,
                        received: Instant::now(),
                        discontinuity: false,
                    },
                    frames: BLOCK_FRAMES,
                    samples: [0.1; BLOCK_FRAMES],
                })
                .unwrap();
            loop {
                if session.progress_snapshot().partials > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            let held = session.progress_snapshot();
            assert_eq!(
                (
                    held.audio_received,
                    held.audio_sent,
                    held.messages,
                    held.partials,
                    held.finals,
                    held.emitted
                ),
                (1, 1, 1, 1, 0, 0)
            );
            assert!(held.waiting_language);
            assert_eq!(session.event_queue_depth(), 0);
            allow_final.send(()).unwrap();
            assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Partial);
            assert_eq!(session.next().await.unwrap().kind, TranscriptKind::Final);
            let released = session.progress_snapshot();
            assert_eq!(
                (
                    released.messages,
                    released.partials,
                    released.finals,
                    released.emitted
                ),
                (2, 1, 1, 2)
            );
            assert!(!released.waiting_language);
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phase11_auto_continuous_audio_requests_real_finals_before_language_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // Synthetic service: only its real final carries language. Never infer it from PCM/text.
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut blocks = 0u64;
        let mut endings = 0;
        let mut waiting = false;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["realtimeInput"].get("audio").is_some() {
                blocks += 1;
                if !waiting || blocks.is_multiple_of(25) {
                    waiting = true;
                    send_json(
                        &mut socket,
                        json!({"serverContent":{"interimInputTranscription":{"text":"12345"}}}),
                    )
                    .await
                    .unwrap();
                }
            } else if value["realtimeInput"]["audioStreamEnd"] == true {
                assert!(waiting);
                waiting = false;
                endings += 1;
                let language = if endings == 1 { "en-GB" } else { "ja-JP" };
                send_json(&mut socket, json!({"serverContent":{"inputTranscription":{"text":"synthetic final","languageCode":language}}})).await.unwrap();
            }
        }
        assert_eq!(endings, 2);
        assert!(blocks > 2);
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::Gemini,
            SourceLanguage::Auto,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        tokio::pin!(runner);
        let mut feed = tokio::time::interval(Duration::from_millis(10));
        let mut sequence = 0;
        let mut finals = 0;
        loop {
            tokio::select! {biased;
                result = &mut runner => panic!("recognizer ended before two real finals: {result:?}"),
                event = session.next() => {
                    let event = event.unwrap();
                    assert!(event.start_us.is_none() && event.duration_us.is_none());
                    assert_eq!(event.language, Some(if finals == 0 {SpeechLanguage::English} else {SpeechLanguage::Japanese}));
                    if event.kind == TranscriptKind::Final {
                        finals += 1;
                        if finals == 2 {cancel.cancel();break;}
                    }
                },
                _ = feed.tick() => {
                    session.input.try_push(crate::audio::preprocess::NormalizedAudio {stamp:AudioStamp {session:identity().session,sequence,frame_position:Some(sequence * BLOCK_FRAMES as u64),qpc_100ns:None,received:Instant::now(),discontinuity:false},frames:BLOCK_FRAMES,samples:[0.1;BLOCK_FRAMES]}).unwrap();
                    sequence += 1;
                }
            }
        }
        assert_eq!(runner.await, Err(SpeechError::Cancelled));
        assert_eq!(session.progress_snapshot().finals, 2);
        assert_eq!(session.progress_snapshot().language_ends, 2);
    };
    tokio::time::timeout(FINAL_TIMEOUT + Duration::from_secs(2), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[test]
fn phase11_auto_language_end_is_once_and_never_renews_or_fakes_a_final() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    protocol
        .receive(
            br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic unknown"}}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    let since = protocol.held_since.unwrap();
    assert!(
        protocol
            .language_input_end(since + AUTO_LANGUAGE_FINALIZE_AFTER - Duration::from_millis(1))
            .is_none()
    );
    assert_eq!(
        protocol.language_input_end(since + AUTO_LANGUAGE_FINALIZE_AFTER),
        Some(json!({"realtimeInput":{"audioStreamEnd":true}}))
    );
    assert_eq!(protocol.held_since, Some(since));
    assert!(
        protocol
            .language_input_end(since + AUTO_LANGUAGE_FINALIZE_AFTER + Duration::from_secs(1))
            .is_none()
    );
    assert_eq!(protocol.audio(block(2, false, false)).unwrap().len(), 1);
    assert!(
        protocol
            .idle_input(
                protocol.last_gemini_audio.unwrap() + GEMINI_INPUT_IDLE,
                true
            )
            .is_none()
    );
    assert_eq!(
        protocol.expiration_error(since + AUTO_LANGUAGE_TIMEOUT),
        Some(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 0);
    assert_eq!(session.progress_snapshot().finals, 0);
}

#[test]
fn phase11_auto_idle_end_deduplicates_language_end_until_real_final() {
    let (session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    let unknown =
        br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic unknown"}}}"#;
    protocol
        .receive(unknown, task.output_ref_for_test())
        .unwrap();
    let since = protocol.held_since.unwrap();
    assert!(
        protocol
            .idle_input(
                protocol.last_gemini_audio.unwrap() + GEMINI_INPUT_IDLE,
                true
            )
            .is_some()
    );
    assert!(
        protocol
            .language_input_end(since + AUTO_LANGUAGE_FINALIZE_AFTER)
            .is_none()
    );
    assert!(protocol.current.is_some());
    protocol.receive(br#"{"serverContent":{"inputTranscription":{"text":"synthetic final","languageCode":"en-GB"}}}"#,task.output_ref_for_test()).unwrap();
    assert!(protocol.current.is_none());
    assert!(!protocol.language_end_sent);
    assert_eq!(session.event_queue_depth(), 2);
    protocol.audio(block(2, false, false)).unwrap();
    protocol
        .receive(unknown, task.output_ref_for_test())
        .unwrap();
    assert_eq!(session.event_queue_depth(), 2); // Previous English must not release this utterance.
    assert!(
        protocol
            .language_input_end(protocol.held_since.unwrap() + AUTO_LANGUAGE_FINALIZE_AFTER)
            .is_some()
    );
}

#[test]
fn phase11_auto_language_end_keeps_known_and_explicit_languages_on_the_existing_path() {
    for language in [SourceLanguage::Auto, SourceLanguage::English] {
        let (session, task) = SpeechSession::new(identity(), language, caps()).unwrap();
        let mut protocol = Protocol::new(SpeechProvider::Gemini, language, identity());
        protocol.audio(block(1, false, false)).unwrap();
        protocol.receive(br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic known","languageCode":"en-US"}}}"#,task.output_ref_for_test()).unwrap();
        assert_eq!(session.event_queue_depth(), 1);
        assert!(
            protocol
                .language_input_end(Instant::now() + AUTO_LANGUAGE_FINALIZE_AFTER)
                .is_none()
        );
    }
    let mut openai = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::Auto, identity());
    assert!(
        openai
            .language_input_end(Instant::now() + AUTO_LANGUAGE_FINALIZE_AFTER)
            .is_none()
    );
}
#[tokio::test]
async fn phase11_gemini_auto_keeps_reported_language_only_within_the_same_utterance() {
    let (mut session, task) = SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
    protocol.audio(block(1, false, false)).unwrap();
    protocol.receive(br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic partial","languageCode":"en-US"}}}"#, task.output_ref_for_test()).unwrap();
    assert_eq!(
        session.next().await.unwrap().language,
        Some(SpeechLanguage::English)
    );
    protocol
        .receive(
            br#"{"serverContent":{"inputTranscription":{"text":"synthetic final"}}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    let final_event = session.next().await.unwrap();
    assert_eq!(final_event.language, Some(SpeechLanguage::English));
    assert!(protocol.current.is_none());
    assert_eq!(
        protocol.receive(
            br#"{"serverContent":{"inputTranscription":{"text":"synthetic next unknown"}}}"#,
            task.output_ref_for_test()
        ),
        Err(SpeechError::LanguageNotReported)
    );
    assert_eq!(session.event_queue_depth(), 0);
}
#[test]
fn phase11_gemini_auto_plan_requests_detection_instead_of_language_hints() {
    let plan = SpeechProvider::Gemini.plan(SourceLanguage::Auto);
    assert_eq!(
        plan.setup["setup"]["inputAudioTranscription"]["languageCodes"],
        json!([])
    );
    for language in SpeechLanguage::ALL {
        let source = match language {
            SpeechLanguage::English => SourceLanguage::English,
            SpeechLanguage::Japanese => SourceLanguage::Japanese,
            SpeechLanguage::French => SourceLanguage::French,
            SpeechLanguage::German => SourceLanguage::German,
            SpeechLanguage::Korean => SourceLanguage::Korean,
        };
        assert_eq!(
            SpeechProvider::Gemini.plan(source).setup["setup"]["inputAudioTranscription"]["languageCodes"],
            json!([language.locale()])
        );
    }
}

#[tokio::test]
async fn phase11_gemini_auto_rejects_foreign_or_malformed_language_after_valid_partial() {
    for code in ["es-ES", "zh-Hant", "en--US", "en_US", "", "unknown"] {
        let (mut session, task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Auto, identity());
        protocol.audio(block(1, false, false)).unwrap();
        protocol.receive(br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic partial","languageCode":"en-GB"}}}"#, task.output_ref_for_test()).unwrap();
        assert_eq!(
            session.next().await.unwrap().language,
            Some(SpeechLanguage::English)
        );
        let payload = serde_json::to_vec(&json!({"serverContent":{"inputTranscription":{"text":"synthetic rejected","languageCode":code}}})).unwrap();
        assert_eq!(
            protocol.receive(&payload, task.output_ref_for_test()),
            Err(SpeechError::UnsupportedLanguage)
        );
        assert_eq!(session.event_queue_depth(), 0);
    }
}
#[tokio::test]
async fn unsupported_detection_never_releases_held_private_text() {
    for code in ["es", "zh-CN", "unknown"] {
        let (mut session, task) =
            SpeechSession::new(identity(), SourceLanguage::Auto, caps()).unwrap();
        let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::Auto, identity());
        protocol.audio(block(1, true, true)).unwrap();
        protocol
            .receive(&ack("a"), task.output_ref_for_test())
            .unwrap();
        protocol
            .receive(&delta("a", "private"), task.output_ref_for_test())
            .unwrap();
        assert_eq!(
            protocol.receive(&final_text("a", code), task.output_ref_for_test()),
            Err(SpeechError::UnsupportedLanguage)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(5), session.next())
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn out_of_order_final_results_keep_explicit_capture_order() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    for (sequence, key) in [(1, "a"), (2, "b")] {
        protocol.audio(block(sequence, true, true)).unwrap();
        protocol
            .receive(&ack(key), task.output_ref_for_test())
            .unwrap();
    }
    protocol
        .receive(&final_text("b", "en"), task.output_ref_for_test())
        .unwrap();
    protocol
        .receive(&final_text("a", "en"), task.output_ref_for_test())
        .unwrap();
    let mut ledger = TranscriptLedger::new(identity());
    assert!(ledger.apply(session.next().await.unwrap()));
    assert!(ledger.apply(session.next().await.unwrap()));
    assert_eq!(ledger.entries()[0].sequence, 1);
    assert_eq!(ledger.entries()[1].sequence, 2);
}
#[tokio::test]
async fn gemini_continuous_audio_has_server_utterance_ids_and_unknown_capture_times() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::Japanese, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::Japanese, identity());
    for sequence in 1..200 {
        let messages = protocol.audio(block(sequence, true, true)).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0]["realtimeInput"].get("audio").is_some());
        assert!(messages[0]["realtimeInput"].get("activityStart").is_none());
    }
    protocol.receive(br#"{"serverContent":{"interimInputTranscription":{"text":"fixture","languageCode":"ja-JP"}}}"#,task.output_ref_for_test()).unwrap();
    let partial = session.next().await.unwrap();
    assert_eq!(partial.utterance, 1);
    assert_eq!(partial.sequence, 1);
    assert_eq!(partial.start_us, None);
    assert_eq!(partial.duration_us, None);
    protocol.audio(block(200, true, true)).unwrap();
    protocol.receive(br#"{"serverContent":{"inputTranscription":{"text":"fixture final","languageCode":"ja-JP"}}}"#,task.output_ref_for_test()).unwrap();
    let final_event = session.next().await.unwrap();
    assert_eq!(final_event.utterance, partial.utterance);
    assert_eq!(final_event.kind, TranscriptKind::Final);
    assert_eq!(final_event.duration_us, None);
    assert!(protocol.current.is_none());
}
#[test]
fn malformed_ack_expired_final_and_secret_error_are_bounded() {
    let (_session, task) = SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    assert!(
        protocol
            .receive(&ack("unsolicited"), task.output_ref_for_test())
            .is_err()
    );
    protocol.audio(block(1, true, true)).unwrap();
    protocol.pending.front_mut().unwrap().1 -= FINAL_TIMEOUT + Duration::from_secs(1);
    assert!(protocol.expired());
    assert_eq!(
        check_service_error(&json!({"error":{"code":429,"message":"SECRET"}})),
        Err(SpeechError::RateLimited)
    );
    assert!(!SpeechError::RateLimited.to_string().contains("SECRET"));
}

#[tokio::test]
async fn phase11_gemini_unsolicited_text_and_control_frames_cannot_create_capture_evidence() {
    let (mut session, task) =
        SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::English, identity());
    let partial=br#"{"serverContent":{"interimInputTranscription":{"text":"fixture","languageCode":"en-US"}}}"#;
    assert_eq!(
        protocol.receive(partial, task.output_ref_for_test()),
        Err(SpeechError::InvalidData)
    );
    protocol.audio(block(999, true, true)).unwrap();
    protocol
        .receive(
            br#"{"serverContent":{"turnComplete":true}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    assert!(protocol.current.is_none());
    protocol
        .receive(partial, task.output_ref_for_test())
        .unwrap();
    let event = session.next().await.unwrap();
    assert_eq!(event.sequence, 1);
    assert_eq!(event.start_us, None);
    protocol
        .receive(
            br#"{"serverContent":{"turnComplete":true}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    assert!(protocol.current.is_some());
    protocol
        .receive(
            br#"{"serverContent":{"inputTranscription":{"text":"final","languageCode":"en-US"}}}"#,
            task.output_ref_for_test(),
        )
        .unwrap();
    assert_eq!(session.next().await.unwrap().duration_us, None);
    assert!(protocol.current.is_none());
}
#[tokio::test]
async fn phase11_gemini_delayed_final_keeps_fresh_audio_flowing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut chunks = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            assert!(value["realtimeInput"].get("activityEnd").is_none());
            if value["realtimeInput"].get("audio").is_some() {
                chunks += 1;
                if chunks == 1 {
                    send_json(&mut socket,json!({"serverContent":{"interimInputTranscription":{"text":"synthetic partial","languageCode":"en-US"}}})).await.unwrap();
                }
                if chunks == 100 {
                    send_json(&mut socket,json!({"serverContent":{"inputTranscription":{"text":"synthetic final","languageCode":"en-US"}}})).await.unwrap();
                }
            }
        }
        assert!(chunks >= 100);
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let recognizer = NetworkRecognizer {
            provider: SpeechProvider::Gemini,
            model: LiveSpeechModel::default_for(SpeechProvider::Gemini),
            quality: TranslationQuality::Standard,
            socket: Mutex::new(Some(socket)),
        };
        let (mut session, task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = task.run(&recognizer);
        tokio::pin!(runner);
        let mut feed = tokio::time::interval(Duration::from_millis(10));
        feed.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut sequence = 0;
        let mut final_seen = false;
        loop {
            tokio::select! { biased;
             result=&mut runner=>{assert_eq!(result,Err(SpeechError::Cancelled));break;},
             event=session.next()=>{let event=event.unwrap();assert_eq!(event.start_us,None);assert_eq!(event.duration_us,None);if event.kind==TranscriptKind::Final {final_seen=true;cancel.cancel();}},
             _=feed.tick()=>{
                    let input=crate::audio::preprocess::NormalizedAudio{stamp:AudioStamp{session:identity().session,sequence,frame_position:Some(sequence*480),qpc_100ns:None,received:Instant::now(),discontinuity:false},frames:480,samples:[if !(8..48).contains(&sequence) {0.1} else {0.0};BLOCK_FRAMES]};
              session.input.try_push(input).unwrap();sequence+=1;
             }
            }
        }
        assert!(final_seen && sequence >= 100);
        let mut protocol =
            Protocol::new(SpeechProvider::Gemini, SourceLanguage::English, identity());
        let mut stale = block(1, true, false);
        stale.audio.stamp.received -= MAX_AUDIO_AGE + Duration::from_millis(1);
        assert_eq!(
            protocol.audio(stale),
            Err(SpeechError::audio_failure(
                AudioStage::Protocol,
                AudioIssue::Age
            ))
        );
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn local_websocket_stream_handles_cancellation_without_orphan_worker() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(message) = socket.next().await {
            if message.is_err() {
                break;
            }
        }
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::OpenAi,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let stopper = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, stopper);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn phase11_gemini_paused_input_flushes_final_and_resumes_on_the_same_connection() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let mut audio_blocks = 0;
        let mut endings = 0;
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["realtimeInput"].get("audio").is_some() {
                audio_blocks += 1;
                assert_eq!(audio_blocks, endings + 1);
                send_json(&mut socket, json!({"serverContent":{"interimInputTranscription":{"text":"synthetic paused partial","languageCode":"en-US"}}})).await.unwrap();
            } else {
                assert_eq!(value, json!({"realtimeInput":{"audioStreamEnd":true}}));
                endings += 1;
                assert_eq!(endings, audio_blocks);
                send_json(&mut socket, json!({"serverContent":{"inputTranscription":{"text":"synthetic paused final","languageCode":"en-US"}}})).await.unwrap();
            }
        }
        assert_eq!((audio_blocks, endings), (2, 2));
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (mut session, mut task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = run_socket(
            socket,
            SpeechProvider::Gemini,
            SourceLanguage::English,
            task.take_audio_for_test(),
            task.take_output_for_test(),
            cancel.clone(),
        );
        let driver = async {
            let mut finals = 0;
            for sequence in 0..2 {
                session
                    .input
                    .try_push(crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp {
                            session: identity().session,
                            sequence,
                            frame_position: Some(sequence * 480),
                            qpc_100ns: None,
                            received: Instant::now(),
                            discontinuity: false,
                        },
                        frames: 480,
                        samples: [0.1; BLOCK_FRAMES],
                    })
                    .unwrap();
                loop {
                    let event = session.next().await.unwrap();
                    assert_eq!(event.identity, identity());
                    assert_eq!(event.utterance, sequence + 1);
                    assert_eq!(event.start_us, None);
                    if event.kind == TranscriptKind::Final {
                        finals += 1;
                        break;
                    }
                }
            }
            assert_eq!(finals, 2);
            cancel.cancel();
        };
        let (result, ()) = tokio::join!(runner, driver);
        assert_eq!(result, Err(SpeechError::Cancelled));
    };
    tokio::time::timeout(Duration::from_secs(4), async {
        tokio::join!(server, client);
    })
    .await
    .expect("paused input must flush without new PCM or a new Session");
}

#[test]
fn phase11_idle_flush_waits_for_real_queues_and_never_repeats_or_changes_openai() {
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::English, identity());
    let now = Instant::now();
    assert!(protocol.idle_input(now, true).is_none());
    protocol.audio(block(1, true, false)).unwrap();
    let last = protocol.last_gemini_audio.unwrap();
    assert!(
        protocol
            .idle_input(last + GEMINI_INPUT_IDLE - Duration::from_millis(1), true)
            .is_none()
    );
    assert!(
        protocol
            .idle_input(last + GEMINI_INPUT_IDLE, false)
            .is_none()
    );
    assert_eq!(
        protocol.idle_input(last + GEMINI_INPUT_IDLE, true).unwrap(),
        json!({"realtimeInput":{"audioStreamEnd":true}})
    );
    assert!(
        protocol
            .idle_input(last + GEMINI_INPUT_IDLE + FINAL_TIMEOUT, true)
            .is_none()
    );
    protocol.audio(block(2, false, false)).unwrap();
    assert!(
        protocol
            .idle_input(
                protocol.last_gemini_audio.unwrap() + GEMINI_INPUT_IDLE,
                true
            )
            .is_some()
    );
    let mut openai = Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
    openai.audio(block(1, true, false)).unwrap();
    assert!(openai.idle_input(now + FINAL_TIMEOUT, true).is_none());
}

#[tokio::test]
async fn phase11_idle_final_keeps_original_timeout_and_completed_pause_does_not_expire() {
    let (_session, task) = SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::English, identity());
    protocol.audio(block(1, true, false)).unwrap();
    protocol.receive(br#"{"serverContent":{"interimInputTranscription":{"text":"synthetic partial","languageCode":"en-US"}}}"#,task.output_ref_for_test()).unwrap();
    let last = Instant::now() - GEMINI_INPUT_IDLE;
    protocol.last_gemini_audio = Some(last);
    assert!(protocol.idle_input(Instant::now(), true).is_some());
    protocol.current.as_mut().unwrap().last_response =
        Some(Instant::now() - FINAL_TIMEOUT - Duration::from_secs(1));
    assert!(protocol.expired()); // No unbounded grace period or fake final.
    protocol.receive(br#"{"serverContent":{"inputTranscription":{"text":"synthetic final","languageCode":"en-US"}}}"#,task.output_ref_for_test()).unwrap();
    assert!(protocol.current.is_none());
    assert!(!protocol.expired());
    assert!(
        protocol
            .idle_input(Instant::now() + FINAL_TIMEOUT, true)
            .is_none()
    );
}

#[tokio::test]
async fn phase11_gemini_ten_ms_packets_keep_streaming_across_four_final_only_turns() {
    const TURN_BLOCKS: u64 = 48; // 80 ms voiced + the existing 400 ms endpoint silence.
    const TURNS: u64 = 4;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let (mut starts, mut ends, mut chunks) = (0_u64, 0_u64, 0_u64);
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            let input = &value["realtimeInput"];
            if input.get("activityStart").is_some() {
                assert_eq!(starts, ends);
                starts += 1;
            }
            if let Some(data) = input["audio"]["data"].as_str() {
                assert_eq!(STANDARD.decode(data).unwrap().len(), 320);
                chunks += 1;
            }
            if input.get("audio").is_some() && chunks % TURN_BLOCKS == 0 {
                ends += 1;
                assert_eq!(chunks, ends * TURN_BLOCKS);
                tokio::time::sleep(Duration::from_millis(30)).await;
                socket.send(Message::Binary(serde_json::to_vec(&json!({"serverContent":{
                    "inputTranscription":{"text":"synthetic final fixture","languageCode":"en-US"}
                }})).unwrap().into())).await.unwrap();
            }
        }
        assert_eq!(starts, 0);
        assert_eq!(ends, TURNS);
    };
    let client = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let recognizer = NetworkRecognizer {
            provider: SpeechProvider::Gemini,
            model: LiveSpeechModel::default_for(SpeechProvider::Gemini),
            quality: TranslationQuality::Standard,
            socket: Mutex::new(Some(socket)),
        };
        let (mut session, task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let cancel = session.cancellation();
        let runner = task.run(&recognizer);
        tokio::pin!(runner);
        let mut feed = tokio::time::interval(Duration::from_millis(10));
        feed.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (mut sequence, mut finals) = (0_u64, 0_u64);
        loop {
            tokio::select! { biased;
                result = &mut runner => { assert_eq!(result, Err(SpeechError::Cancelled)); break; },
                event = session.next() => {
                    let event = event.expect("runner must not discard a live session");
                    assert_eq!(event.kind, TranscriptKind::Final);
                    assert_eq!(event.utterance, finals + 1);
                    assert_eq!(event.sequence, finals + 1);
                    assert_eq!(event.duration_us, None);
                    finals += 1;
                    if finals == TURNS { cancel.cancel(); }
                },
                _ = feed.tick(), if sequence < TURNS * TURN_BLOCKS => {
                    let mut input = crate::audio::preprocess::NormalizedAudio {
                        stamp: AudioStamp { session:identity().session, sequence,
                            frame_position:Some(sequence * 480), qpc_100ns:None,
                            received:Instant::now(), discontinuity:false },
                        frames:480, samples:[0.0;BLOCK_FRAMES],
                    };
                    if sequence % TURN_BLOCKS < 8 { input.samples[..480].fill(0.1); }
                    session.input.try_push(input).unwrap();
                    sequence += 1;
                }
            }
        }
        assert_eq!(finals, TURNS);
    };
    tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn synthetic_pcm_stream_reaches_partial_and_final_for_both_providers_and_five_languages() {
    use crate::audio::preprocess::NormalizedAudio;
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for language in [
            SourceLanguage::English,
            SourceLanguage::Japanese,
            SourceLanguage::French,
            SourceLanguage::German,
            SourceLanguage::Korean,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let selected = SpeechLanguage::selected(language).unwrap();
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut chunks = 0;
                let mut nonzero = false;
                while let Some(Ok(message)) = socket.next().await {
                    let Message::Text(text) = message else {
                        continue;
                    };
                    let message: Value = serde_json::from_str(&text).unwrap();
                    let data = match provider {
                        SpeechProvider::OpenAi => message["audio"].as_str(),
                        SpeechProvider::Gemini => {
                            message["realtimeInput"]["audio"]["data"].as_str()
                        }
                    };
                    if let Some(data) = data {
                        let pcm = STANDARD.decode(data).unwrap();
                        let expected = if provider == SpeechProvider::OpenAi {
                            960
                        } else {
                            640
                        };
                        assert_eq!(pcm.len(), expected);
                        chunks += 1;
                        nonzero |= pcm.iter().any(|byte| *byte != 0);
                    }
                    let ended = match provider {
                        SpeechProvider::OpenAi => message["type"] == "input_audio_buffer.commit",
                        SpeechProvider::Gemini => chunks == 30,
                    };
                    if ended {
                        assert!(chunks >= 20 && nonzero);
                        match provider {
                            SpeechProvider::OpenAi => {
                                socket
                                    .send(Message::Text(
                                        String::from_utf8(ack("fixture-item")).unwrap().into(),
                                    ))
                                    .await
                                    .unwrap();
                                socket
                                    .send(Message::Text(
                                        String::from_utf8(delta(
                                            "fixture-item",
                                            "synthetic partial",
                                        ))
                                        .unwrap()
                                        .into(),
                                    ))
                                    .await
                                    .unwrap();
                                socket
                                    .send(Message::Text(
                                        String::from_utf8(final_text(
                                            "fixture-item",
                                            selected.iso_code(),
                                        ))
                                        .unwrap()
                                        .into(),
                                    ))
                                    .await
                                    .unwrap();
                            }
                            SpeechProvider::Gemini => {
                                // Gemini may carry the same JSON in Binary frames.
                                // Mix Binary partial and Text final through the real
                                // socket runner for every supported source language.
                                socket.send(Message::Binary(serde_json::to_vec(&json!({"serverContent":{"interimInputTranscription":{"text":"synthetic partial","languageCode":selected.locale()}}})).unwrap().into())).await.unwrap();
                                send_json(&mut socket,json!({"serverContent":{"inputTranscription":{"text":"fixture final","languageCode":selected.locale()}}})).await.unwrap();
                            }
                        }
                    }
                }
            };
            let client = async {
                let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
                    .await
                    .unwrap();
                let (mut session, mut task) =
                    SpeechSession::new(identity(), language, caps()).unwrap();
                let cancel = session.cancellation();
                let runner = run_socket(
                    socket,
                    provider,
                    language,
                    task.take_audio_for_test(),
                    task.take_output_for_test(),
                    cancel.clone(),
                );
                tokio::pin!(runner);
                let mut feed = tokio::time::interval(Duration::from_millis(20));
                feed.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let mut sequence = 0;
                let mut partial = false;
                loop {
                    tokio::select! {
                        result = &mut runner => panic!("fixture ended prematurely: {result:?}"),
                        _ = feed.tick(), if sequence < 30 => {
                            let mut input = NormalizedAudio { stamp:AudioStamp { session:identity().session,sequence,
                                frame_position:Some(sequence * BLOCK_FRAMES as u64),qpc_100ns:None,received:Instant::now(),discontinuity:false },
                                frames:BLOCK_FRAMES,samples:[0.0;BLOCK_FRAMES] };
                            if sequence < 10 {
                                for (index,sample) in input.samples.iter_mut().enumerate() {
                                    *sample = (std::f64::consts::TAU * 1000.0 * (sequence * BLOCK_FRAMES as u64 + index as u64) as f64 / 48000.0).sin() as f32 * 0.1;
                                }
                            }
                            session.input.try_push(input).unwrap(); sequence += 1;
                        },
                        event = session.next() => {
                            let event = event.unwrap();
                            assert_eq!(event.identity,identity());
                            if event.kind == TranscriptKind::Final || provider == SpeechProvider::Gemini { assert_eq!(event.language,Some(selected)); }
                            else { assert!(event.language.is_none()); }
                            if event.kind == TranscriptKind::Partial { partial = true; }
                            else { assert!(partial); assert_eq!(event.duration_us, if provider == SpeechProvider::OpenAi {Some(600000)} else {None}); break; }
                        },
                    }
                }
                cancel.cancel();
                assert_eq!(runner.await, Err(SpeechError::Cancelled));
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(server, client);
            })
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn oversized_websocket_message_is_rejected_before_json_processing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        socket
            .send(Message::Text("x".repeat(MAX_WIRE_BYTES + 1).into()))
            .await
            .unwrap();
    };
    let client = async {
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(MAX_WIRE_BYTES);
        config.max_frame_size = Some(MAX_WIRE_BYTES);
        let (mut socket, _) = tokio_tungstenite::connect_async_with_config(
            format!("ws://{address}"),
            Some(config),
            false,
        )
        .await
        .unwrap();
        assert!(matches!(
            read_message(&mut socket, SpeechProvider::OpenAi).await,
            Err(SpeechError::InvalidData)
        ));
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_failure_is_not_replaced_by_worker_cleanup_cancellation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        send_json(
            &mut socket,
            json!({"error":{"code":"rate_limit_exceeded","message":"PRIVATE_SENTINEL"}}),
        )
        .await
        .unwrap();
    };
    let client = async {
        let stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let (socket, _) = tokio_tungstenite::client_async(
            format!("ws://{address}"),
            MaybeTlsStream::Plain(stream),
        )
        .await
        .unwrap();
        let recognizer = NetworkRecognizer {
            provider: SpeechProvider::OpenAi,
            model: LiveSpeechModel::default_for(SpeechProvider::OpenAi),
            quality: TranslationQuality::Standard,
            socket: Mutex::new(Some(socket)),
        };
        let (session, task) = SpeechSession::new(
            identity(),
            SourceLanguage::English,
            recognizer.capabilities(),
        )
        .unwrap();
        assert_eq!(task.run(&recognizer).await, Err(SpeechError::RateLimited));
        assert_eq!(
            session.cancellation().reason(),
            Some(SpeechError::RateLimited)
        );
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(server, client);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn slow_transport_write_times_out_without_growing_audio_backlog() {
    let (writer, _unread_peer) = tokio::io::duplex(16);
    let mut socket =
        WebSocketStream::from_raw_socket(writer, tungstenite::protocol::Role::Client, None).await;
    assert_eq!(
        send_json(
            &mut socket,
            json!({"type":"input_audio_buffer.append","audio":"SYNTHETIC".repeat(100)})
        )
        .await,
        Err(SpeechError::Timeout)
    );
}

#[test]
fn malformed_language_fields_and_rotation_messages_are_not_accepted() {
    assert_eq!(
        encode_key(&ApiKey::from_text("A&B=/?%+").unwrap()).as_str(),
        "A%26B%3D%2F%3F%25%2B"
    );
    for value in [
        json!("en"),
        json!([{"code":12}]),
        json!([{"code":"en"},null]),
    ] {
        let (_session, task) =
            SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
        let mut protocol =
            Protocol::new(SpeechProvider::OpenAi, SourceLanguage::English, identity());
        protocol.audio(block(1, true, true)).unwrap();
        protocol
            .receive(&ack("a"), task.output_ref_for_test())
            .unwrap();
        let mut message: Value = serde_json::from_slice(&final_text("a", "en")).unwrap();
        message["languages"] = value;
        assert_eq!(
            protocol.receive(
                &serde_json::to_vec(&message).unwrap(),
                task.output_ref_for_test()
            ),
            Err(SpeechError::InvalidData)
        );
    }
    let (_session, task) = SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
    let mut protocol = Protocol::new(SpeechProvider::Gemini, SourceLanguage::English, identity());
    assert_eq!(
        protocol.receive(br#"{"goAway":null}"#, task.output_ref_for_test()),
        Err(SpeechError::InvalidData)
    );
    assert_eq!(
        protocol.receive(
            br#"{"goAway":{"timeLeft":"1s"}}"#,
            task.output_ref_for_test()
        ),
        Err(SpeechError::SessionExpired)
    );
}

#[tokio::test]
#[ignore = "live TLS/auth boundary only; sends a synthetic invalid key, never audio or real credentials"]
async fn live_secure_endpoints_reject_synthetic_invalid_keys_without_audio() {
    let key = ApiKey::from_text("SYNTHETIC_INVALID_TEST_KEY").unwrap();
    let mut verified = true;
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let result = NetworkRecognizer::connect(provider, &key, SourceLanguage::English).await;
        let error = result.err();
        eprintln!("provider={provider:?}; classified result={error:?}");
        verified &= matches!(
            error.as_ref().map(SpeechError::root_cause),
            Some(SpeechError::Credential | SpeechError::InvalidData)
        );
    }
    assert!(
        verified,
        "secure endpoints did not both return a classified auth/protocol rejection"
    );
}

#[tokio::test]
async fn phase11_connection_setup_close_preserves_stage_code_and_never_private_reason() {
    use tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for code in [1000u16, 1007, 1008, 1011, 1013] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let setup = socket.next().await.unwrap().unwrap();
                assert!(setup.is_text());
                socket
                    .close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: "PRIVATE_KEY_SIGNED_URL_AND_TRANSCRIPT".into(),
                    }))
                    .await
                    .unwrap();
            };
            let client = async {
                let request = format!("ws://{address}/?key=PRIVATE_FAKE_KEY")
                    .into_client_request()
                    .unwrap();
                let error =
                    NetworkRecognizer::connect_request(provider, request, SourceLanguage::English)
                        .await
                        .err()
                        .unwrap();
                assert!(matches!(
                    error,
                    SpeechError::Connection {
                        stage: ConnectionStage::SetupReceive,
                        ..
                    }
                ));
                let SpeechError::Connection { cause, .. } = &error else {
                    unreachable!()
                };
                assert!(matches!(cause.as_ref(), SpeechError::RemoteStatus {
                    kind: RemoteStatusKind::WebSocket, code: actual, ..
                } if *actual == code));
                assert_eq!(error.retryable(), matches!(code, 1000 | 1011 | 1013));
                assert!(!format!("{error:?} {error}").contains("PRIVATE"));
                assert!(!error.to_string().contains(&address.to_string()));
                assert!(error.to_string().contains(&code.to_string()));
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(server, client);
            })
            .await
            .unwrap();
        }
    }
}

#[test]
fn phase11_rustls_wrapped_io_errors_are_terminal_tls_failures_without_leaking_details() {
    for cause in [
        rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        rustls::Error::General("PRIVATE_CERTIFICATE_OR_NETWORK_DETAIL".into()),
    ] {
        let error = map_socket_error(tungstenite::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            cause,
        )));
        assert_eq!(error, SpeechError::SecureTransport);
        assert!(!error.retryable());
        assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    }
    for kind in [
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::UnexpectedEof,
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::InvalidData,
    ] {
        let error = map_socket_error(tungstenite::Error::Io(std::io::Error::new(
            kind,
            "PRIVATE_NON_TLS_IO_DETAIL",
        )));
        assert_eq!(error, SpeechError::Network);
        assert!(error.retryable());
        assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    }
}

#[tokio::test]
async fn phase11_connection_http_rejection_keeps_status_but_discards_private_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for status in [400u16, 401, 403, 404, 429, 500, 503] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut headers = [0u8; 4096];
            let mut used = 0;
            while !headers[..used].windows(4).any(|end| end == b"\r\n\r\n") {
                assert!(used < headers.len(), "bounded fixture request headers");
                let read = stream.read(&mut headers[used..]).await.unwrap();
                assert_ne!(read, 0);
                used += read;
            }
            let body = "PRIVATE_BODY_KEY_AND_TRANSCRIPT";
            stream.write_all(format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await.unwrap();
        };
        let client = async {
            let request = format!("ws://{address}/?key=PRIVATE_FAKE_KEY")
                .into_client_request()
                .unwrap();
            let error = NetworkRecognizer::connect_request(
                SpeechProvider::Gemini,
                request,
                SourceLanguage::English,
            )
            .await
            .err()
            .unwrap();
            let SpeechError::Connection { stage, cause } = &error else {
                panic!("missing stage")
            };
            assert_eq!(*stage, ConnectionStage::Handshake);
            assert!(matches!(cause.as_ref(), SpeechError::RemoteStatus {
                kind: RemoteStatusKind::Http, code, ..
            } if *code == status));
            assert_eq!(error.retryable(), status == 429 || status >= 500);
            assert!(!format!("{error:?} {error}").contains("PRIVATE"));
            assert!(error.to_string().contains(&status.to_string()));
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, client);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn phase11_connection_ready_and_one_shared_deadline_preserve_the_original_contract() {
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let setup = socket.next().await.unwrap().unwrap();
            assert_eq!(
                parse(setup.into_text().unwrap().as_bytes()).unwrap(),
                LiveSpeechModel::default_for(provider)
                    .plan(SourceLanguage::English, TranslationQuality::Standard)
                    .setup
            );
            send_json(
                &mut socket,
                match provider {
                    SpeechProvider::OpenAi => json!({"type":"session.updated"}),
                    SpeechProvider::Gemini => json!({"setupComplete":{}}),
                },
            )
            .await
            .unwrap();
        };
        let client = async {
            let request = format!("ws://{address}").into_client_request().unwrap();
            let recognizer =
                NetworkRecognizer::connect_request(provider, request, SourceLanguage::English)
                    .await
                    .unwrap();
            assert!(recognizer.socket.lock().unwrap().is_some());
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, client);
        })
        .await
        .unwrap();
    }
    // A completed earlier step cannot give setup a fresh 20-second budget.
    let expired = tokio::time::Instant::now() - Duration::from_millis(1);
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        connection_step::<()>(
            ConnectionStage::SetupReceive,
            expired,
            std::future::pending(),
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(
        error,
        SpeechError::Connection {
            stage: ConnectionStage::SetupReceive,
            cause: Box::new(SpeechError::Timeout),
        }
    );
}

#[tokio::test]
async fn phase11_gemini_binary_json_setup_accepts_all_language_choices_without_audio() {
    for language in [
        SourceLanguage::Auto,
        SourceLanguage::English,
        SourceLanguage::Japanese,
        SourceLanguage::French,
        SourceLanguage::German,
        SourceLanguage::Korean,
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let setup = socket.next().await.unwrap().unwrap();
            assert_eq!(
                parse(setup.into_text().unwrap().as_bytes()).unwrap(),
                SpeechProvider::Gemini.plan(language).setup
            );
            socket
                .send(Message::Binary(br#"{"setupComplete":{}}"#.to_vec().into()))
                .await
                .unwrap();
            // connect-only diagnostics must close without audio/client content.
            if let Some(Ok(message)) = socket.next().await {
                assert!(matches!(message, Message::Close(_)));
            }
        };
        let client = async {
            let request = format!("ws://{address}/?key=SYNTHETIC_ONLY")
                .into_client_request()
                .unwrap();
            let recognizer =
                NetworkRecognizer::connect_request(SpeechProvider::Gemini, request, language)
                    .await
                    .unwrap();
            assert!(recognizer.socket.lock().unwrap().is_some());
            drop(recognizer);
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, client);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn phase11_binary_setup_keeps_json_size_provider_and_safe_error_boundaries() {
    let mut at_limit = br#"{"setupComplete":{},"padding":""}"#.to_vec();
    at_limit.splice(
        at_limit.len() - 2..at_limit.len() - 2,
        std::iter::repeat_n(b'x', MAX_WIRE_BYTES - at_limit.len()),
    );
    assert_eq!(at_limit.len(), MAX_WIRE_BYTES);
    let cases = [
        (SpeechProvider::Gemini, at_limit, Ok(())),
        (
            SpeechProvider::Gemini,
            vec![0xff, 0xfe],
            Err(SpeechError::InvalidData),
        ),
        (
            SpeechProvider::Gemini,
            b"PRIVATE_MALFORMED_JSON".to_vec(),
            Err(SpeechError::InvalidData),
        ),
        (
            SpeechProvider::Gemini,
            b"[]".to_vec(),
            Err(SpeechError::InvalidData),
        ),
        (
            SpeechProvider::Gemini,
            br#"{"setupComplete":null}"#.to_vec(),
            Err(SpeechError::InvalidData),
        ),
        (
            SpeechProvider::Gemini,
            vec![b'x'; MAX_WIRE_BYTES + 1],
            Err(SpeechError::InvalidData),
        ),
        (
            SpeechProvider::Gemini,
            br#"{"error":{"code":429,"message":"PRIVATE_FAKE_KEY_AND_TEXT"}}"#.to_vec(),
            Err(SpeechError::RateLimited),
        ),
        (
            SpeechProvider::OpenAi,
            br#"{"type":"session.updated"}"#.to_vec(),
            Err(SpeechError::InvalidData),
        ),
    ];
    for (provider, payload, expected) in cases {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert!(socket.next().await.unwrap().unwrap().is_text());
            socket.send(Message::Binary(payload.into())).await.unwrap();
        };
        let client = async {
            let request = format!("ws://{address}/?key=SYNTHETIC_ONLY")
                .into_client_request()
                .unwrap();
            let result =
                NetworkRecognizer::connect_request(provider, request, SourceLanguage::English)
                    .await;
            match (result, expected) {
                (Ok(_), Ok(())) => (),
                (Err(error), Err(expected)) => {
                    assert_eq!(*error.root_cause(), expected);
                    assert!(matches!(
                        error,
                        SpeechError::Connection {
                            stage: ConnectionStage::SetupReceive,
                            ..
                        }
                    ));
                    assert!(!format!("{error:?} {error}").contains("PRIVATE"));
                }
                _ => panic!("unexpected connection outcome"),
            }
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(server, client);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn phase11_local_close_frames_keep_typed_causes_without_private_reason_text() {
    use tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        for (code, expected) in [
            (1000u16, SpeechError::Network),
            (1008, SpeechError::Credential),
            (1013, SpeechError::RateLimited),
            (1011, SpeechError::ServiceInternal),
            (1002, SpeechError::InvalidData),
        ] {
            let expected = SpeechError::RemoteStatus {
                kind: RemoteStatusKind::WebSocket,
                code,
                cause: Box::new(expected),
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                socket
                    .close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: "PRIVATE_TEST_REASON".into(),
                    }))
                    .await
                    .unwrap();
            };
            let client = async {
                let stream = TcpStream::connect(address).await.unwrap();
                let (socket, _) = tokio_tungstenite::client_async(
                    format!("ws://{address}"),
                    MaybeTlsStream::Plain(stream),
                )
                .await
                .unwrap();
                let recognizer = NetworkRecognizer {
                    provider,
                    model: LiveSpeechModel::default_for(provider),
                    quality: TranslationQuality::Standard,
                    socket: Mutex::new(Some(socket)),
                };
                let (mut session, task) =
                    SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
                let result = task.run(&recognizer).await;
                assert_eq!(result, Err(expected.clone()));
                assert_eq!(session.cancellation().reason(), Some(expected.clone()));
                assert!(!expected.to_string().contains("PRIVATE_TEST_REASON"));
                assert!(session.next().await.is_none());
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(server, client);
            })
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn phase11_repeated_local_transport_disconnects_use_fresh_sessions_and_close_workers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    for (index, provider) in [SpeechProvider::OpenAi, SpeechProvider::Gemini]
        .into_iter()
        .enumerate()
    {
        for round in 0..16 {
            let identity = StreamIdentity {
                session: SessionId(92_000 + index as u64 * 16 + round),
                generation: 1,
            };
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                // Wait until the real client loop is active, then drop without
                // a close handshake. No audio or provider credentials are used.
                assert!(socket.next().await.is_some());
            };
            let client = async {
                let stream = TcpStream::connect(address).await.unwrap();
                let (socket, _) = tokio_tungstenite::client_async(
                    format!("ws://{address}"),
                    MaybeTlsStream::Plain(stream),
                )
                .await
                .unwrap();
                let recognizer = NetworkRecognizer {
                    provider,
                    model: LiveSpeechModel::default_for(provider),
                    quality: TranslationQuality::Standard,
                    socket: Mutex::new(Some(socket)),
                };
                let metrics = crate::performance::live::LiveDiagnostics::new(identity);
                let (mut session, task) = SpeechSession::diagnosed(
                    identity,
                    SourceLanguage::English,
                    caps(),
                    Some(metrics.clone()),
                )
                .unwrap();
                assert_eq!(task.run(&recognizer).await, Err(SpeechError::Network));
                assert_eq!(session.cancellation().reason(), Some(SpeechError::Network));
                assert!(session.next().await.is_none());
                assert_eq!(
                    metrics
                        .gauge(crate::performance::live::Activity::SpeechOperation)
                        .current,
                    0
                );
                assert_eq!(
                    metrics
                        .gauge(crate::performance::live::Activity::SpeechAudioQueue)
                        .current,
                    0
                );
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                tokio::join!(server, client);
            })
            .await
            .unwrap();
        }
    }
}

#[test]
fn phase11_transport_reset_is_retryable_but_malformed_protocol_stays_terminal() {
    use tungstenite::error::ProtocolError;
    let reset = map_socket_error(tungstenite::Error::Protocol(
        ProtocolError::ResetWithoutClosingHandshake,
    ));
    assert_eq!(reset, SpeechError::Network);
    assert!(reset.retryable());
    for protocol in [
        ProtocolError::InvalidOpcode(15),
        ProtocolError::InvalidCloseSequence,
    ] {
        let error = map_socket_error(tungstenite::Error::Protocol(protocol));
        assert_eq!(error, SpeechError::InvalidData);
        assert!(!error.retryable());
    }
}

#[tokio::test]
#[ignore = "real 30-second heartbeat timeout for each local mock provider; no cloud or audio"]
async fn phase11_silent_local_peers_hit_real_heartbeat_deadlines_and_release_workers() {
    for provider in [SpeechProvider::OpenAi, SpeechProvider::Gemini] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (done, finished) = tokio::sync::oneshot::channel();
        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            // Retain the local connection without reading or responding to Ping.
            let _ = finished.await;
            drop(socket);
        };
        let client = async {
            let stream = TcpStream::connect(address).await.unwrap();
            let (socket, _) = tokio_tungstenite::client_async(
                format!("ws://{address}"),
                MaybeTlsStream::Plain(stream),
            )
            .await
            .unwrap();
            let recognizer = NetworkRecognizer {
                provider,
                model: LiveSpeechModel::default_for(provider),
                quality: TranslationQuality::Standard,
                socket: Mutex::new(Some(socket)),
            };
            let (mut session, task) =
                SpeechSession::new(identity(), SourceLanguage::English, caps()).unwrap();
            let started = Instant::now();
            assert_eq!(task.run(&recognizer).await, Err(SpeechError::Timeout));
            assert!(started.elapsed() >= HEARTBEAT_TIMEOUT);
            assert_eq!(session.cancellation().reason(), Some(SpeechError::Timeout));
            assert!(session.next().await.is_none());
            println!(
                "PHASE11_HEARTBEAT provider={provider:?} elapsed_ms={} typed_timeout=true",
                started.elapsed().as_millis()
            );
            let _ = done.send(());
        };
        tokio::time::timeout(
            HEARTBEAT_TIMEOUT + HEARTBEAT_INTERVAL + Duration::from_secs(5),
            async {
                tokio::join!(server, client);
            },
        )
        .await
        .unwrap();
    }
}
