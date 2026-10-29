//! Private RAM-only unary speech wire formats. Native provider time evidence is
//! checked before context filtering; no generated timestamp JSON is accepted.
use super::{
    prepared_speech::{
        MAX_ASR_CUES_PER_CHUNK, MAX_ASR_RESPONSE_BYTES, MAX_CHUNK_DURATION_MS, PreparedChunkStamp,
        PreparedSpeechCue, PreparedSpeechError, PreparedSpeechRequest, PreparedSpeechResponse,
        validate_text,
    },
    provider::SpeechProvider,
    speech::SpeechLanguage,
};
use crate::{
    media::work,
    resolver::contracts::{MAX_CUE_BYTES, ResolverCancellation},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, future::Future, sync::Arc};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};

pub(crate) const MAX_ASR_HTTP_RESPONSE_BYTES: usize = 512 * 1024;
const WAVE_HEADER_BYTES: usize = 44;
const PCM_SAMPLE_BYTES: usize = 2;
const MAX_WAVE_BYTES: usize =
    WAVE_HEADER_BYTES + 24_000 * MAX_CHUNK_DURATION_MS as usize / 1000 * PCM_SAMPLE_BYTES;
/// Fixed JSON/multipart fields fit below this allowance, independent of input.
const WIRE_METADATA_BYTES: usize = 4096;
const MAX_WIRE_BODY_BYTES: usize = MAX_WAVE_BYTES.div_ceil(3) * 4 + WIRE_METADATA_BYTES;
const MAX_OPENAI_PROMPT_BYTES: usize = 1024;
const PCM_CHECK_SAMPLES: usize = 2048;
const BASE64_CHECK_BYTES: usize = 3 * 4096;
const BOUNDARY_CHECK_BYTES: usize = 4096;
/// Provider duration may be rounded to hundredths; this never loosens cue time.
const PROVIDER_DURATION_ROUNDING_US: u64 = 10_000;
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
/// Independently bound annotation work, even when words become fewer cues.
const MAX_NATIVE_WORDS: usize = 2048;

pub(crate) struct PreparedHttpPayload {
    pub body: Vec<u8>,
    pub content_type: Box<str>,
}
struct Metadata {
    stamp: PreparedChunkStamp,
    language: SpeechLanguage,
    audio_end_us: u64,
}
impl Metadata {
    fn from_request(request: &PreparedSpeechRequest) -> Result<Self, PreparedSpeechError> {
        let language = SpeechLanguage::selected(request.language())
            .ok_or(PreparedSpeechError::UnsupportedLanguage)?;
        Ok(Self {
            stamp: request.chunk().stamp(),
            language,
            audio_end_us: request.chunk().audio_end_us(),
        })
    }
    fn duration_us(&self) -> u64 {
        self.audio_end_us - self.stamp.audio_start_us
    }
}
struct EncodeInput {
    metadata: Metadata,
    samples: Box<[i16]>,
    context: Box<[Box<str>]>,
}

#[cfg(test)]
pub(crate) async fn encode(
    provider: SpeechProvider,
    request: &PreparedSpeechRequest,
    cancel: &ResolverCancellation,
) -> Result<PreparedHttpPayload, PreparedSpeechError> {
    encode_impl(provider, request, cancel, None).await
}
pub(crate) async fn encode_with_lease(
    provider: SpeechProvider,
    request: &PreparedSpeechRequest,
    cancel: &ResolverCancellation,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<PreparedHttpPayload, PreparedSpeechError> {
    encode_impl(provider, request, cancel, Some(lease)).await
}
async fn encode_impl(
    provider: SpeechProvider,
    request: &PreparedSpeechRequest,
    cancel: &ResolverCancellation,
    lease: Option<Arc<OwnedSemaphorePermit>>,
) -> Result<PreparedHttpPayload, PreparedSpeechError> {
    check(cancel)?;
    let metadata = Metadata::from_request(request)?;
    let input = EncodeInput {
        metadata,
        samples: request.chunk().samples().to_vec().into_boxed_slice(),
        context: request.previous_source().to_vec().into_boxed_slice(),
    };
    run_worker(cancel, work::workers(), lease, move |scope| {
        encode_inner(provider, input, scope)
    })
    .await
}
#[cfg(test)]
pub(crate) async fn parse(
    provider: SpeechProvider,
    body: Vec<u8>,
    request: &PreparedSpeechRequest,
    cancel: &ResolverCancellation,
) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
    parse_impl(provider, body, request, cancel, None).await
}
pub(crate) async fn parse_with_lease(
    provider: SpeechProvider,
    body: Vec<u8>,
    request: &PreparedSpeechRequest,
    cancel: &ResolverCancellation,
    lease: Arc<OwnedSemaphorePermit>,
) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
    parse_impl(provider, body, request, cancel, Some(lease)).await
}
async fn parse_impl(
    provider: SpeechProvider,
    body: Vec<u8>,
    request: &PreparedSpeechRequest,
    cancel: &ResolverCancellation,
    lease: Option<Arc<OwnedSemaphorePermit>>,
) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
    check(cancel)?;
    let metadata = Metadata::from_request(request)?;
    if body.is_empty() || body.len() > MAX_ASR_HTTP_RESPONSE_BYTES {
        return Err(PreparedSpeechError::InvalidData);
    }
    let body = body.into_boxed_slice();
    let response = run_worker(cancel, work::workers(), lease, move |scope| {
        parse_inner(provider, &body, metadata, scope)
    })
    .await?;
    check(cancel)?;
    response.validate_for(request)?;
    check(cancel)?;
    Ok(response)
}
struct CancelOnDrop {
    cancel: ResolverCancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.cancel();
        }
    }
}
struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn run_worker<'a, T, F>(
    cancel: &'a ResolverCancellation,
    cpu: Arc<Semaphore>,
    lease: Option<Arc<OwnedSemaphorePermit>>,
    operation: F,
) -> impl Future<Output = Result<T, PreparedSpeechError>> + Send + 'a
where
    T: Send + 'static,
    F: FnOnce(&ResolverCancellation) -> Result<T, PreparedSpeechError> + Send + 'static,
{
    let guard = CancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    async move {
        let mut guard = guard;
        check(cancel)?;
        let permit = cpu
            .try_acquire_owned()
            .map_err(|_| PreparedSpeechError::Overloaded)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| PreparedSpeechError::InvalidData)?;
        let worker_cancel = cancel.clone();
        let mut worker = AbortOnDrop(runtime.spawn_blocking(move || {
            let _permit = permit;
            // HTTP request capacity survives owner Drop until this real worker
            // has released its input/output and completed native CPU work.
            let _lease = lease;
            if let Err(error) = check(&worker_cancel) {
                // Dispose captured PCM/JSON while both capacity leases are
                // still live, including cancellation before the operation.
                drop(operation);
                return Err(error);
            }
            let result = operation(&worker_cancel);
            check(&worker_cancel)?;
            result
        }));
        let result = tokio::select! { biased;
            _=cancel.cancelled()=>return Err(PreparedSpeechError::Cancelled),
            result=&mut worker.0=>result.map_err(|_|PreparedSpeechError::InvalidData)?,
        };
        check(cancel)?;
        if result.is_ok() {
            guard.armed = false;
        }
        result
    }
}
fn check(cancel: &ResolverCancellation) -> Result<(), PreparedSpeechError> {
    if cancel.is_cancelled() {
        Err(PreparedSpeechError::Cancelled)
    } else {
        Ok(())
    }
}
fn append(body: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PreparedSpeechError> {
    body.len()
        .checked_add(bytes.len())
        .filter(|len| *len <= MAX_WIRE_BODY_BYTES)
        .ok_or(PreparedSpeechError::InvalidData)?;
    body.try_reserve_exact(bytes.len())
        .map_err(|_| PreparedSpeechError::Overloaded)?;
    body.extend_from_slice(bytes);
    Ok(())
}
fn wave(
    samples: &[i16],
    rate: u32,
    cancel: &ResolverCancellation,
) -> Result<Vec<u8>, PreparedSpeechError> {
    let data_bytes = samples
        .len()
        .checked_mul(PCM_SAMPLE_BYTES)
        .filter(|len| {
            len.checked_add(WAVE_HEADER_BYTES)
                .is_some_and(|size| size <= MAX_WAVE_BYTES)
        })
        .ok_or(PreparedSpeechError::InvalidData)?;
    let data_bytes = u32::try_from(data_bytes).map_err(|_| PreparedSpeechError::InvalidData)?;
    let mut body = Vec::new();
    body.try_reserve_exact(data_bytes as usize + WAVE_HEADER_BYTES)
        .map_err(|_| PreparedSpeechError::Overloaded)?;
    body.extend_from_slice(b"RIFF");
    body.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    body.extend_from_slice(b"WAVEfmt ");
    body.extend_from_slice(&16u32.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&rate.to_le_bytes());
    body.extend_from_slice(&(rate * 2).to_le_bytes());
    body.extend_from_slice(&2u16.to_le_bytes());
    body.extend_from_slice(&16u16.to_le_bytes());
    body.extend_from_slice(b"data");
    body.extend_from_slice(&data_bytes.to_le_bytes());
    for part in samples.chunks(PCM_CHECK_SAMPLES) {
        check(cancel)?;
        for sample in part {
            body.extend_from_slice(&sample.to_le_bytes());
        }
    }
    check(cancel)?;
    Ok(body)
}
fn context_tail(
    context: &[Box<str>],
    cancel: &ResolverCancellation,
) -> Result<String, PreparedSpeechError> {
    let mut full = String::new();
    for text in context {
        check(cancel)?;
        if !full.is_empty() {
            full.push('\n');
        }
        full.push_str(text);
    }
    let mut start = full.len().saturating_sub(MAX_OPENAI_PROMPT_BYTES);
    while !full.is_char_boundary(start) {
        start += 1;
    }
    Ok(full[start..].to_owned())
}
fn contains_boundary(
    bytes: &[u8],
    boundary: &[u8],
    cancel: &ResolverCancellation,
) -> Result<bool, PreparedSpeechError> {
    for (index, window) in bytes.windows(boundary.len()).enumerate() {
        if index.is_multiple_of(BOUNDARY_CHECK_BYTES) {
            check(cancel)?;
        }
        if window == boundary {
            return Ok(true);
        }
    }
    Ok(false)
}
fn field(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    value: &str,
) -> Result<(), PreparedSpeechError> {
    append(
        body,
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    )
}
fn encode_inner(
    provider: SpeechProvider,
    input: EncodeInput,
    cancel: &ResolverCancellation,
) -> Result<PreparedHttpPayload, PreparedSpeechError> {
    check(cancel)?;
    let wav = wave(&input.samples, input.metadata.stamp.rate.hz(), cancel)?;
    match provider {
        SpeechProvider::OpenAi => {
            let boundary = format!(
                "subtitlenow-{:016x}-{:016x}",
                input.metadata.stamp.identity.session.0, input.metadata.stamp.sequence
            );
            let prompt = context_tail(&input.context, cancel)?;
            if contains_boundary(&wav, boundary.as_bytes(), cancel)?
                || contains_boundary(prompt.as_bytes(), boundary.as_bytes(), cancel)?
            {
                return Err(PreparedSpeechError::InvalidData);
            }
            let mut body = Vec::new();
            field(&mut body, &boundary, "model", "whisper-1")?;
            field(
                &mut body,
                &boundary,
                "language",
                input.metadata.language.iso_code(),
            )?;
            field(&mut body, &boundary, "response_format", "verbose_json")?;
            field(&mut body, &boundary, "timestamp_granularities[]", "segment")?;
            if !prompt.is_empty() {
                field(&mut body, &boundary, "prompt", &prompt)?;
            }
            append(&mut body, format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"chunk.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes())?;
            append(&mut body, &wav)?;
            append(&mut body, format!("\r\n--{boundary}--\r\n").as_bytes())?;
            check(cancel)?;
            Ok(PreparedHttpPayload {
                body: body.into_boxed_slice().into_vec(),
                content_type: format!("multipart/form-data; boundary={boundary}").into(),
            })
        }
        SpeechProvider::Gemini => {
            let mut encoded = String::new();
            let encoded_len = wav
                .len()
                .checked_add(2)
                .and_then(|len| (len / 3).checked_mul(4))
                .ok_or(PreparedSpeechError::InvalidData)?;
            encoded
                .try_reserve_exact(encoded_len)
                .map_err(|_| PreparedSpeechError::Overloaded)?;
            for part in wav.chunks(BASE64_CHECK_BYTES) {
                check(cancel)?;
                STANDARD.encode_string(part, &mut encoded);
            }
            let payload = GeminiInput {
                model: "gemini-3.5-transcribe",
                store: false,
                background: false,
                stream: false,
                input: [GeminiAudio {
                    kind: "audio",
                    data: &encoded,
                    mime_type: "audio/wav",
                }],
                generation_config: GeminiGeneration {
                    transcription_config: GeminiTranscription {
                        language_codes: [input.metadata.language.locale()],
                        mode: GeminiMode {
                            kind: "verbatim",
                            timestamp_granularities: ["word"],
                        },
                    },
                },
            };
            let body =
                serde_json::to_vec(&payload).map_err(|_| PreparedSpeechError::InvalidData)?;
            if body.len() > MAX_WIRE_BODY_BYTES {
                return Err(PreparedSpeechError::InvalidData);
            }
            check(cancel)?;
            Ok(PreparedHttpPayload {
                body: body.into_boxed_slice().into_vec(),
                content_type: "application/json".into(),
            })
        }
    }
}
#[derive(Serialize)]
struct GeminiInput<'a> {
    model: &'static str,
    store: bool,
    background: bool,
    stream: bool,
    input: [GeminiAudio<'a>; 1],
    generation_config: GeminiGeneration,
}
#[derive(Serialize)]
struct GeminiAudio<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    data: &'a str,
    mime_type: &'static str,
}
#[derive(Serialize)]
struct GeminiGeneration {
    transcription_config: GeminiTranscription,
}
#[derive(Serialize)]
struct GeminiTranscription {
    language_codes: [&'static str; 1],
    mode: GeminiMode,
}
#[derive(Serialize)]
struct GeminiMode {
    #[serde(rename = "type")]
    kind: &'static str,
    timestamp_granularities: [&'static str; 1],
}

#[derive(Deserialize)]
struct OpenAiResult {
    language: Box<str>,
    duration: f64,
    text: Box<str>,
    segments: Vec<OpenAiSegment>,
}
#[derive(Deserialize)]
struct OpenAiSegment {
    id: u64,
    start: f64,
    end: f64,
    text: Box<str>,
}
#[derive(Deserialize)]
struct GeminiResult {
    status: Box<str>,
    steps: Vec<GeminiStep>,
}
#[derive(Deserialize)]
#[serde(tag = "type")]
enum GeminiStep {
    #[serde(rename = "model_output")]
    ModelOutput { content: Vec<GeminiContent> },
}
#[derive(Deserialize)]
#[serde(tag = "type")]
enum GeminiContent {
    #[serde(rename = "text")]
    Text {
        text: Box<str>,
        annotations: Vec<GeminiWord>,
    },
}
#[derive(Deserialize)]
#[serde(tag = "type")]
enum GeminiWord {
    #[serde(rename = "word_info")]
    WordInfo {
        text: Box<str>,
        start_offset: Box<str>,
        end_offset: Box<str>,
        start_index: Option<usize>,
        end_index: Option<usize>,
    },
}

fn parse_inner(
    provider: SpeechProvider,
    body: &[u8],
    metadata: Metadata,
    cancel: &ResolverCancellation,
) -> Result<PreparedSpeechResponse, PreparedSpeechError> {
    check(cancel)?;
    let cues = match provider {
        SpeechProvider::OpenAi => parse_openai(body, &metadata, cancel)?,
        SpeechProvider::Gemini => parse_gemini(body, &metadata, cancel)?,
    };
    check(cancel)?;
    Ok(PreparedSpeechResponse {
        identity: metadata.stamp.identity,
        sequence: metadata.stamp.sequence,
        run_id: metadata.stamp.run_id,
        language: Some(metadata.language),
        cues,
    })
}
fn language_name(name: &str) -> Option<SpeechLanguage> {
    [
        ("english", SpeechLanguage::English),
        ("japanese", SpeechLanguage::Japanese),
        ("french", SpeechLanguage::French),
        ("german", SpeechLanguage::German),
        ("korean", SpeechLanguage::Korean),
    ]
    .into_iter()
    .find_map(|(expected, language)| name.eq_ignore_ascii_case(expected).then_some(language))
}
fn seconds_us(seconds: f64) -> Result<u64, PreparedSpeechError> {
    let max = u64::from(MAX_CHUNK_DURATION_MS) * 1000 + PROVIDER_DURATION_ROUNDING_US;
    if !seconds.is_finite()
        || seconds < 0.0
        || seconds > max as f64 / MICROSECONDS_PER_SECOND as f64
    {
        return Err(PreparedSpeechError::InvalidData);
    }
    Ok((seconds * MICROSECONDS_PER_SECOND as f64).round() as u64)
}
fn timed(start: u64, end: u64, metadata: &Metadata) -> Result<bool, PreparedSpeechError> {
    if end <= start || end > metadata.duration_us() {
        return Err(PreparedSpeechError::InvalidData);
    }
    let absolute_start = metadata
        .stamp
        .audio_start_us
        .checked_add(start)
        .ok_or(PreparedSpeechError::InvalidData)?;
    let absolute_end = metadata
        .stamp
        .audio_start_us
        .checked_add(end)
        .ok_or(PreparedSpeechError::InvalidData)?;
    if absolute_end <= metadata.stamp.body_start_us || absolute_start >= metadata.stamp.body_end_us
    {
        return Ok(false);
    }
    if absolute_start < metadata.stamp.body_start_us || absolute_end > metadata.stamp.body_end_us {
        return Err(PreparedSpeechError::BoundaryMismatch);
    }
    Ok(true)
}
fn aggregate_text(text: &str) -> Result<(), PreparedSpeechError> {
    if text.len() > MAX_ASR_RESPONSE_BYTES
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        Err(PreparedSpeechError::InvalidData)
    } else {
        Ok(())
    }
}
fn parse_openai(
    body: &[u8],
    metadata: &Metadata,
    cancel: &ResolverCancellation,
) -> Result<Vec<PreparedSpeechCue>, PreparedSpeechError> {
    let result: OpenAiResult =
        serde_json::from_slice(body).map_err(|_| PreparedSpeechError::InvalidData)?;
    check(cancel)?;
    if language_name(&result.language) != Some(metadata.language) {
        return Err(PreparedSpeechError::UnsupportedLanguage);
    }
    let duration = seconds_us(result.duration)?;
    if duration == 0
        || duration.abs_diff(metadata.duration_us()) > PROVIDER_DURATION_ROUNDING_US
        || result.segments.len() > MAX_ASR_CUES_PER_CHUNK
    {
        return Err(PreparedSpeechError::InvalidData);
    }
    aggregate_text(&result.text)?;
    if result.text.trim().is_empty() != result.segments.is_empty()
        || !result
            .text
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .eq(result
                .segments
                .iter()
                .flat_map(|segment| segment.text.chars())
                .filter(|ch| !ch.is_whitespace()))
    {
        return Err(PreparedSpeechError::InvalidData);
    }
    let mut cues = Vec::with_capacity(result.segments.len());
    let mut ids = HashSet::with_capacity(result.segments.len());
    let mut previous_end = None;
    let mut bytes = 0usize;
    for segment in result.segments {
        check(cancel)?;
        validate_text(&segment.text)?;
        bytes = bytes
            .checked_add(segment.text.len())
            .filter(|value| *value <= MAX_ASR_RESPONSE_BYTES)
            .ok_or(PreparedSpeechError::InvalidData)?;
        if !ids.insert(segment.id) {
            return Err(PreparedSpeechError::InvalidData);
        }
        let start = seconds_us(segment.start)?;
        let end = seconds_us(segment.end)?;
        if previous_end.is_some_and(|previous| start < previous) {
            return Err(PreparedSpeechError::InvalidData);
        }
        let owned = timed(start, end, metadata)?;
        previous_end = Some(end);
        if owned {
            cues.push(PreparedSpeechCue {
                start_us: start,
                duration_us: end - start,
                text: segment.text,
            });
        }
    }
    Ok(cues)
}
fn offset_us(value: &str) -> Result<u64, PreparedSpeechError> {
    let seconds = value
        .strip_suffix('s')
        .filter(|value| !value.is_empty() && value.len() <= 32)
        .ok_or(PreparedSpeechError::InvalidData)?;
    let (whole, fraction) = seconds
        .split_once('.')
        .map_or((seconds, None), |(whole, fraction)| (whole, Some(fraction)));
    if whole.is_empty() || !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PreparedSpeechError::InvalidData);
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| PreparedSpeechError::InvalidData)?;
    let base = whole
        .checked_mul(MICROSECONDS_PER_SECOND)
        .ok_or(PreparedSpeechError::InvalidData)?;
    let mut micros = 0u64;
    if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 9
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(PreparedSpeechError::InvalidData);
        }
        for index in 0..6 {
            micros = micros * 10
                + fraction
                    .as_bytes()
                    .get(index)
                    .map_or(0, |byte| u64::from(*byte - b'0'));
        }
        if fraction.as_bytes().get(6).is_some_and(|byte| *byte >= b'5') {
            micros += 1;
        }
    }
    base.checked_add(micros)
        .filter(|value| *value <= u64::from(MAX_CHUNK_DURATION_MS) * 1000)
        .ok_or(PreparedSpeechError::InvalidData)
}
struct WordGroup {
    start: u64,
    end: u64,
    text: String,
}
fn flush_group(
    group: &mut Option<WordGroup>,
    cues: &mut Vec<PreparedSpeechCue>,
) -> Result<(), PreparedSpeechError> {
    if let Some(group) = group.take() {
        if cues.len() >= MAX_ASR_CUES_PER_CHUNK {
            return Err(PreparedSpeechError::InvalidData);
        }
        validate_text(&group.text)?;
        cues.push(PreparedSpeechCue {
            start_us: group.start,
            duration_us: group.end - group.start,
            text: group.text.into_boxed_str(),
        });
    }
    Ok(())
}
fn parse_gemini(
    body: &[u8],
    metadata: &Metadata,
    cancel: &ResolverCancellation,
) -> Result<Vec<PreparedSpeechCue>, PreparedSpeechError> {
    let result: GeminiResult =
        serde_json::from_slice(body).map_err(|_| PreparedSpeechError::InvalidData)?;
    check(cancel)?;
    if result.status.as_ref() != "completed" {
        return Err(PreparedSpeechError::InvalidData);
    }
    let mut cues = Vec::new();
    let mut previous_end = None;
    let mut total_bytes = 0usize;
    let mut words = 0usize;
    let mut saw_text_output = false;
    for step in result.steps {
        let GeminiStep::ModelOutput { content } = step;
        for item in content {
            check(cancel)?;
            let GeminiContent::Text { text, annotations } = item;
            saw_text_output = true;
            aggregate_text(&text)?;
            total_bytes = total_bytes
                .checked_add(text.len())
                .filter(|value| *value <= MAX_ASR_RESPONSE_BYTES)
                .ok_or(PreparedSpeechError::InvalidData)?;
            let mut cursor = 0usize;
            let mut group = None;
            for annotation in annotations {
                check(cancel)?;
                words = words
                    .checked_add(1)
                    .filter(|value| *value <= MAX_NATIVE_WORDS)
                    .ok_or(PreparedSpeechError::InvalidData)?;
                let GeminiWord::WordInfo {
                    text: word,
                    start_offset,
                    end_offset,
                    start_index,
                    end_index,
                } = annotation;
                validate_text(&word)?;
                let (start_index, end_index) = match (start_index, end_index) {
                    (Some(start), Some(end)) => (start, end),
                    (None, None) => {
                        let tail = text.get(cursor..).ok_or(PreparedSpeechError::InvalidData)?;
                        let start = cursor + tail.len()
                            - tail.trim_start_matches(char::is_whitespace).len();
                        (
                            start,
                            start
                                .checked_add(word.len())
                                .ok_or(PreparedSpeechError::InvalidData)?,
                        )
                    }
                    _ => return Err(PreparedSpeechError::InvalidData),
                };
                if start_index < cursor
                    || end_index <= start_index
                    || !text
                        .get(cursor..start_index)
                        .is_some_and(|gap| gap.chars().all(char::is_whitespace))
                    || text.get(start_index..end_index) != Some(word.as_ref())
                {
                    return Err(PreparedSpeechError::InvalidData);
                }
                let start = offset_us(&start_offset)?;
                let end = offset_us(&end_offset)?;
                if previous_end.is_some_and(|previous| start < previous) {
                    return Err(PreparedSpeechError::InvalidData);
                }
                let owned = timed(start, end, metadata)?;
                previous_end = Some(end);
                if owned {
                    let separator = text
                        .get(cursor..start_index)
                        .ok_or(PreparedSpeechError::InvalidData)?;
                    let added = separator
                        .len()
                        .checked_add(word.len())
                        .ok_or(PreparedSpeechError::InvalidData)?;
                    if group.as_ref().is_some_and(|group: &WordGroup| {
                        group
                            .text
                            .len()
                            .checked_add(added)
                            .is_none_or(|len| len > MAX_CUE_BYTES)
                    }) {
                        flush_group(&mut group, &mut cues)?;
                    }
                    if let Some(group) = &mut group {
                        group.text.push_str(separator);
                        group.text.push_str(&word);
                        group.end = end;
                    } else {
                        group = Some(WordGroup {
                            start,
                            end,
                            text: word.into(),
                        });
                    }
                } else {
                    flush_group(&mut group, &mut cues)?;
                }
                cursor = end_index;
            }
            if !text
                .get(cursor..)
                .is_some_and(|tail| tail.chars().all(char::is_whitespace))
            {
                return Err(PreparedSpeechError::InvalidData);
            }
            flush_group(&mut group, &mut cues)?;
        }
    }
    if !saw_text_output {
        return Err(PreparedSpeechError::InvalidData);
    }
    Ok(cues)
}

#[cfg(test)]
#[path = "prepared_speech_wire_tests.rs"]
mod tests;
