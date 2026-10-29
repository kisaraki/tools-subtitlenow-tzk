//! Provider-neutral translated captions. No inferred source/capture correlation.
use super::speech::{StreamIdentity, TranscriptKind};
use std::time::Instant;

pub struct NativeCaption {
    pub identity: StreamIdentity,
    pub sequence: u64,
    pub revision: u64,
    pub kind: TranscriptKind,
    pub text: Box<str>,
    /// Transport receipt freshness only, never capture or media time.
    pub received: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::speech::{
            EVENT_QUEUE_ITEMS, SpeechCapabilities, SpeechError, SpeechQueue, SpeechSession,
        },
        audio::capture::SessionId,
        domain::SourceLanguage,
    };
    #[test]
    fn phase11_native_caption_lane_is_bounded_and_preserves_first_cancellation_reason() {
        let identity = StreamIdentity {
            session: SessionId(4321),
            generation: 1,
        };
        let (session, task) = SpeechSession::new(
            identity,
            SourceLanguage::English,
            SpeechCapabilities {
                streaming: true,
                partials: true,
                cancellation: true,
                supported_languages: [true; 5],
                constrained_auto_detection: false,
            },
        )
        .unwrap();
        for sequence in 1..=EVENT_QUEUE_ITEMS as u64 {
            task.output_ref_for_test()
                .emit_caption(NativeCaption {
                    identity,
                    sequence,
                    revision: 1,
                    kind: TranscriptKind::Final,
                    text: "合成文字".into(),
                    received: Instant::now(),
                })
                .unwrap();
        }
        let error = task
            .output_ref_for_test()
            .emit_caption(NativeCaption {
                identity,
                sequence: 100,
                revision: 1,
                kind: TranscriptKind::Final,
                text: "合成文字".into(),
                received: Instant::now(),
            })
            .unwrap_err();
        assert_eq!(
            error,
            SpeechError::queue_full(SpeechQueue::NativeCaptionOutput, EVENT_QUEUE_ITEMS)
        );
        assert_eq!(session.cancellation().reason(), Some(error));
    }
}
