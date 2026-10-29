//! Conservative local text-language estimates, never provider/audio/alignment evidence.
use super::speech::{
    Cancellation, MAX_TRANSCRIPT_BYTES, SpeechError, SpeechLanguage, StreamIdentity, TranscriptKind,
};
use std::{sync::Arc, time::Duration};

/// Avoid declaring the language of a greeting, name, number or punctuation alone.
const MIN_LANGUAGE_LETTERS: usize = 12;
/// 40 ms against the existing 240 ms audio queue; do not stall on a busy CPU lane.
const LOCAL_LANGUAGE_BUDGET: Duration = Duration::from_millis(40);

#[derive(Clone)]
struct AssessedText {
    text: Box<str>,
    decision: LanguageDecision,
}
#[derive(Clone, Default)]
pub(crate) struct LanguageAssessment {
    identity: Option<StreamIdentity>,
    message_sequence: u64,
    expires: Option<tokio::time::Instant>,
    texts: [Option<AssessedText>; 2],
}
impl LanguageAssessment {
    pub(crate) fn decision(
        &self,
        identity: StreamIdentity,
        message_sequence: u64,
        kind: TranscriptKind,
        text: &str,
    ) -> Option<LanguageDecision> {
        if self.identity != Some(identity) || self.message_sequence != message_sequence {
            return None;
        }
        let index = match kind {
            TranscriptKind::Partial => 0,
            TranscriptKind::Final => 1,
        };
        self.texts[index]
            .as_ref()
            .filter(|item| item.text.as_ref() == text)
            .map(|item| {
                if self
                    .expires
                    .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                {
                    LanguageDecision::Busy
                } else {
                    item.decision
                }
            })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LanguageInput<'a> {
    pub message_sequence: u64,
    pub partial: Option<&'a str>,
    pub final_text: Option<&'a str>,
}
pub(crate) async fn assess_texts(
    identity: StreamIdentity,
    input: LanguageInput<'_>,
    cancel: &Cancellation,
) -> Result<LanguageAssessment, SpeechError> {
    assess_with_lane(identity, input, cancel, crate::media::work::workers()).await
}
async fn assess_with_lane(
    identity: StreamIdentity,
    input: LanguageInput<'_>,
    cancel: &Cancellation,
    lane: Arc<tokio::sync::Semaphore>,
) -> Result<LanguageAssessment, SpeechError> {
    assess_with_operation(
        identity,
        input,
        cancel,
        lane,
        LOCAL_LANGUAGE_BUDGET,
        classify,
    )
    .await
}
async fn assess_with_operation(
    identity: StreamIdentity,
    input: LanguageInput<'_>,
    cancel: &Cancellation,
    lane: Arc<tokio::sync::Semaphore>,
    budget: Duration,
    operation: impl Fn(&str) -> LanguageDecision + Send + 'static,
) -> Result<LanguageAssessment, SpeechError> {
    if let Some(error) = cancel.reason() {
        return Err(error);
    }
    let deadline = tokio::time::Instant::now() + budget;
    if input.message_sequence == 0 {
        return Err(SpeechError::InvalidData);
    }
    let mut assessment = LanguageAssessment {
        identity: Some(identity),
        message_sequence: input.message_sequence,
        expires: Some(deadline),
        ..Default::default()
    };
    for (index, text) in [input.partial, input.final_text].into_iter().enumerate() {
        let Some(text) = text else {
            continue;
        };
        if text.len() > MAX_TRANSCRIPT_BYTES {
            return Err(SpeechError::InvalidData);
        }
        assessment.texts[index] = Some(AssessedText {
            text: text.into(),
            decision: LanguageDecision::Busy,
        });
    }
    if assessment.texts.iter().all(Option::is_none) {
        return Ok(assessment);
    }
    let Ok(permit) = lane.try_acquire_owned() else {
        return Ok(assessment);
    };
    let worker_cancel = cancel.clone();
    let busy = assessment.clone();
    let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
        let _permit = permit; // Held until the real CPU job exits, including owner cancellation.
        if let Some(error) = worker_cancel.reason() {
            return Err(error);
        }
        for item in assessment.texts.iter_mut().flatten() {
            item.decision = operation(&item.text);
        }
        if let Some(error) = worker_cancel.reason() {
            return Err(error);
        }
        Ok(assessment)
    }));
    tokio::select! { biased;
        error=cancel.cancelled()=>Err(error),
        result=tokio::time::timeout_at(deadline,&mut worker.0)=> match result {
            Ok(Ok(result)) if tokio::time::Instant::now() < deadline => result,
            _=>Ok(busy),
        }
    }
}
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LanguageDecision {
    Supported(SpeechLanguage),
    Unsupported,
    Uncertain,
    Busy,
}

fn classify(text: &str) -> LanguageDecision {
    if text.len() > MAX_TRANSCRIPT_BYTES
        || text
            .chars()
            .filter(|character| character.is_alphabetic())
            .take(MIN_LANGUAGE_LETTERS)
            .count()
            < MIN_LANGUAGE_LETTERS
    {
        return LanguageDecision::Uncertain;
    }
    // Use every bundled language model, then filter the result. A five-model
    // allowlist would force unsupported languages toward one of our five.
    let Some(info) = whatlang::detect(text).filter(whatlang::Info::is_reliable) else {
        return LanguageDecision::Uncertain;
    };
    let language = match info.lang() {
        whatlang::Lang::Eng => SpeechLanguage::English,
        whatlang::Lang::Jpn => SpeechLanguage::Japanese,
        whatlang::Lang::Fra => SpeechLanguage::French,
        whatlang::Lang::Deu => SpeechLanguage::German,
        whatlang::Lang::Kor => SpeechLanguage::Korean,
        _ => return LanguageDecision::Unsupported,
    };
    LanguageDecision::Supported(language)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ai::speech::{SpeechCapabilities, SpeechSession, SpeechTask},
        audio::capture::SessionId,
    };
    fn identity() -> StreamIdentity {
        StreamIdentity {
            session: SessionId(940),
            generation: 1,
        }
    }
    fn fixture() -> (SpeechSession, SpeechTask, Cancellation) {
        let (session, task) = SpeechSession::new(
            identity(),
            crate::domain::SourceLanguage::Auto,
            SpeechCapabilities {
                streaming: true,
                partials: true,
                cancellation: true,
                supported_languages: [true; 5],
                constrained_auto_detection: true,
            },
        )
        .unwrap();
        let cancel = session.cancellation();
        (session, task, cancel)
    }
    pub(super) const SAMPLES: [(&str, SpeechLanguage); 5] = [
        (
            "Today we are discussing how computers process information and display translated subtitles on the screen.",
            SpeechLanguage::English,
        ),
        (
            "今日は図書館で新しい本を借りました。家に帰ってからゆっくり読もうと思っています。",
            SpeechLanguage::Japanese,
        ),
        (
            "Nous allons visiter la bibliothèque cet après-midi pour découvrir les nouveaux livres et discuter avec nos amis.",
            SpeechLanguage::French,
        ),
        (
            "Heute besuchen wir die Bibliothek und sprechen über die neuen Bücher, die wir gemeinsam lesen möchten.",
            SpeechLanguage::German,
        ),
        (
            "오늘은 도서관에서 새로운 책을 빌렸습니다. 집에 돌아가서 가족들과 함께 읽어 보려고 합니다.",
            SpeechLanguage::Korean,
        ),
    ];
    #[test]
    fn phase11_local_language_classifies_five_languages_without_forcing_other_languages() {
        for (text, language) in SAMPLES {
            assert_eq!(
                classify(text),
                LanguageDecision::Supported(language),
                "synthetic {language:?}"
            );
        }
        for text in [
            "Esta mañana vamos a visitar la biblioteca para leer algunos libros interesantes y hablar con nuestros amigos.",
            "這是一段中文測試文字，用來確認本機語言判定不會把中文誤認為英文或其他支援的語言。",
            "Сегодня мы посетим библиотеку, чтобы почитать новые книги и поговорить с нашими друзьями.",
            "Questa mattina andremo in biblioteca per leggere alcuni libri interessanti e parlare con i nostri amici.",
        ] {
            assert!(
                !matches!(classify(text), LanguageDecision::Supported(_)),
                "foreign synthetic text must not be forced into the five languages"
            );
        }
        for text in [
            "",
            "hello",
            "12345678901234567890",
            "...",
            "Paris",
            "hello world",
        ] {
            assert_eq!(classify(text), LanguageDecision::Uncertain);
        }
        assert_eq!(
            classify(&"a".repeat(MAX_TRANSCRIPT_BYTES + 1)),
            LanguageDecision::Uncertain
        );
    }
    #[test]
    fn phase11_local_language_bounded_cpu_probe_uses_only_synthetic_text() {
        let text = SAMPLES[0]
            .0
            .repeat(MAX_TRANSCRIPT_BYTES / SAMPLES[0].0.len());
        let mut samples = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = std::time::Instant::now();
            assert_eq!(
                classify(&text),
                LanguageDecision::Supported(SpeechLanguage::English)
            );
            samples.push(start.elapsed());
        }
        samples.sort();
        eprintln!(
            "local language CPU probe: bytes={}, p50={:?}, p95={:?}, max={:?}",
            text.len(),
            samples[49],
            samples[94],
            samples[99]
        );
    }

    #[tokio::test]
    async fn phase11_local_language_evidence_is_bound_to_exact_text_identity_and_kind() {
        let (_session, _task, cancel) = fixture();
        let mut evidence = assess_with_lane(
            identity(),
            LanguageInput {
                message_sequence: 1,
                partial: Some(SAMPLES[0].0),
                final_text: None,
            },
            &cancel,
            Arc::new(tokio::sync::Semaphore::new(1)),
        )
        .await
        .unwrap();
        assert_eq!(
            evidence.decision(identity(), 1, TranscriptKind::Partial, SAMPLES[0].0),
            Some(LanguageDecision::Supported(SpeechLanguage::English))
        );
        assert_eq!(
            evidence.decision(identity(), 1, TranscriptKind::Final, SAMPLES[0].0),
            None
        );
        assert_eq!(
            evidence.decision(identity(), 1, TranscriptKind::Partial, SAMPLES[1].0),
            None
        );
        let old = StreamIdentity {
            generation: 2,
            ..identity()
        };
        assert_eq!(
            evidence.decision(old, 1, TranscriptKind::Partial, SAMPLES[0].0),
            None
        );
        assert_eq!(
            evidence.decision(identity(), 2, TranscriptKind::Partial, SAMPLES[0].0),
            None
        );
        evidence.expires = Some(tokio::time::Instant::now() - Duration::from_millis(1));
        assert_eq!(
            evidence.decision(identity(), 1, TranscriptKind::Partial, SAMPLES[0].0),
            Some(LanguageDecision::Busy)
        );
    }
    #[tokio::test]
    async fn phase11_local_language_rejects_oversize_cancelled_and_saturated_work() {
        let (_session, _task, cancel) = fixture();
        let lane = Arc::new(tokio::sync::Semaphore::new(0));
        let input = LanguageInput {
            message_sequence: 1,
            partial: None,
            final_text: Some(SAMPLES[0].0),
        };
        let busy = assess_with_lane(identity(), input, &cancel, lane.clone())
            .await
            .unwrap();
        assert_eq!(
            busy.decision(identity(), 1, TranscriptKind::Final, SAMPLES[0].0),
            Some(LanguageDecision::Busy)
        );
        let oversized = "a".repeat(MAX_TRANSCRIPT_BYTES + 1);
        assert!(matches!(
            assess_with_lane(
                identity(),
                LanguageInput {
                    message_sequence: 1,
                    partial: Some(&oversized),
                    final_text: None
                },
                &cancel,
                lane.clone()
            )
            .await,
            Err(SpeechError::InvalidData)
        ));
        cancel.cancel();
        assert!(matches!(
            assess_with_lane(identity(), input, &cancel, lane).await,
            Err(SpeechError::Cancelled)
        ));
    }
    #[tokio::test]
    async fn phase11_local_language_started_cpu_job_keeps_its_permit_until_actual_cleanup() {
        let (_session, _task, cancel) = fixture();
        let lane = Arc::new(tokio::sync::Semaphore::new(1));
        let (entered, ready) = tokio::sync::oneshot::channel();
        let entered = std::sync::Mutex::new(Some(entered));
        let (release, wait) = std::sync::mpsc::channel();
        let mut job = Box::pin(assess_with_operation(
            identity(),
            LanguageInput {
                message_sequence: 1,
                partial: Some(SAMPLES[0].0),
                final_text: None,
            },
            &cancel,
            lane.clone(),
            Duration::from_secs(2),
            move |text| {
                entered.lock().unwrap().take().unwrap().send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
                classify(text)
            },
        ));
        tokio::select! {result=&mut job=>{ assert!(result.is_ok());panic!("job completed before test release"); }, result=ready=>result.unwrap()};
        cancel.cancel();
        assert!(matches!(job.await, Err(SpeechError::Cancelled)));
        assert_eq!(lane.available_permits(), 0);
        release.send(()).unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(1), lane.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert_eq!(lane.available_permits(), 1);
    }
    #[tokio::test]
    async fn phase11_local_language_timeout_returns_busy_without_adopting_or_leaking_work() {
        let (_session, _task, cancel) = fixture();
        let lane = Arc::new(tokio::sync::Semaphore::new(1));
        let evidence = assess_with_operation(
            identity(),
            LanguageInput {
                message_sequence: 1,
                partial: None,
                final_text: Some(SAMPLES[0].0),
            },
            &cancel,
            lane.clone(),
            Duration::from_millis(1),
            move |text| {
                std::thread::sleep(Duration::from_millis(20));
                classify(text)
            },
        )
        .await
        .unwrap();
        assert_eq!(
            evidence.decision(identity(), 1, TranscriptKind::Final, SAMPLES[0].0),
            Some(LanguageDecision::Busy)
        );
        let permit = tokio::time::timeout(Duration::from_secs(1), lane.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert_eq!(lane.available_permits(), 1);
    }
}
