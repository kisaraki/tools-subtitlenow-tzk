//! Self-owned RAM WAVE bytes and real loopback HTTP; no provider/media access.
use super::*;
use crate::{audio::capture::SessionId, media::input::MediaInput};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU16, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};
type Pause = (oneshot::Sender<()>, oneshot::Receiver<()>);
#[derive(Default)]
pub(crate) struct Control {
    pub status: AtomicU16,
    pub changed: AtomicBool,
    pub short: AtomicBool,
    pause: Mutex<Option<Pause>>,
}
pub(crate) struct Source {
    address: std::net::SocketAddr,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
    pub control: Arc<Control>,
    pub rows: Arc<Mutex<Vec<(u64, usize)>>>,
    pub cpu: Arc<Semaphore>,
    pub owners: Arc<Semaphore>,
    pub buffers: Arc<Semaphore>,
    pub pcm: Arc<Semaphore>,
}
pub(crate) fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(936),
        generation: 1,
    }
}
impl Source {
    pub async fn new(bytes: Vec<u8>) -> Self {
        let length = bytes.len() as u64;
        Self::with_length(bytes, length).await
    }
    /// Header-only sparse fixture. Requests beyond supplied bytes are zeros;
    /// used only for capacity preflight, not GB-scale codec/transfer evidence.
    pub async fn with_length(bytes: Vec<u8>, length: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, mut stopped) = oneshot::channel();
        let control = Arc::new(Control::default());
        let mode = control.clone();
        let rows = Arc::new(Mutex::new(Vec::new()));
        let seen = rows.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {biased;_= &mut stopped=>break,result=listener.accept()=>result.unwrap()};
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    header.push(byte[0]);
                    assert!(header.len() <= 8192);
                }
                let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
                assert!(!header.contains("authorization:"));
                assert!(!header.contains("cookie:"));
                if header.contains("if-match:") {
                    assert!(header.contains("if-match: \"v1\"\r\n"));
                }
                let span = header
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .unwrap();
                let (first, last) = span.split_once('-').unwrap();
                let (first, last): (usize, usize) = (first.parse().unwrap(), last.parse().unwrap());
                assert!(first <= last && (last as u64) < length);
                {
                    let mut rows = seen.lock().unwrap();
                    assert!(rows.len() < 512);
                    rows.push((first as u64, last - first + 1));
                }
                let status = mode.status.load(Ordering::Acquire);
                if status != 0 {
                    socket.write_all(format!("HTTP/1.1 {status} Synthetic\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                    continue;
                }
                let tag = if mode.changed.load(Ordering::Acquire) {
                    "\"v2\""
                } else {
                    "\"v1\""
                };
                socket.write_all(format!("HTTP/1.1 206 Partial\r\nContent-Range: bytes {first}-{last}/{length}\r\nETag: {tag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",last-first+1).as_bytes()).await.unwrap();
                let pause = mode.pause.lock().unwrap().take();
                if let Some((ready, resume)) = pause {
                    let _ = ready.send(());
                    let mut byte = [0];
                    tokio::select! {biased;
                        _=&mut stopped=>break,
                        result=socket.read(&mut byte)=>{assert!(matches!(result,Ok(0)|Err(_)));continue},
                        _=resume=>{}
                    }
                }
                let end = if mode.short.load(Ordering::Acquire) {
                    last
                } else {
                    last + 1
                };
                assert!(end - first <= MAX_REMOTE_RANGE_BYTES);
                let mut body = vec![0; end - first];
                if first < bytes.len() {
                    let count = (bytes.len() - first).min(body.len());
                    body[..count].copy_from_slice(&bytes[first..first + count]);
                }
                let _ = socket.write_all(&body).await;
            }
        });
        Self {
            address,
            stop,
            task,
            control,
            rows,
            cpu: Arc::new(Semaphore::new(1)),
            owners: Arc::new(Semaphore::new(1)),
            buffers: Arc::new(Semaphore::new(4)),
            pcm: Arc::new(Semaphore::new(MAX_REMOTE_PCM_BLOCKS)),
        }
    }
    pub fn media_input(&self) -> MediaInput {
        MediaInput::remote(&format!(
            "http://{}/synthetic.bin?private=SYNTHETIC",
            self.address
        ))
        .unwrap()
    }
    pub async fn input(&self, id: StreamIdentity) -> (ProbedRemoteAudio, ResolverCancellation) {
        let cancel = ResolverCancellation::default();
        let media = RemoteHttpMedia::open_for_test(
            self.media_input(),
            id,
            cancel.clone(),
            self.owners.clone(),
            self.buffers.clone(),
        )
        .await
        .unwrap();
        let input = crate::media::remote_probe::probe_for_test(media, self.cpu.clone())
            .await
            .unwrap();
        (input, cancel)
    }
    pub async fn decoder(&self) -> (RemoteWaveDecoder, ResolverCancellation) {
        let (input, cancel) = self.input(identity()).await;
        (
            RemoteWaveDecoder::open_with(input, self.cpu.clone(), self.pcm.clone())
                .await
                .unwrap(),
            cancel,
        )
    }
    pub fn pause(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (ready, started) = oneshot::channel();
        let (resume, proceed) = oneshot::channel();
        *self.control.pause.lock().unwrap() = Some((ready, proceed));
        (started, resume)
    }
    pub async fn finish(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}
pub(crate) fn samples(frames: usize, channels: u16) -> Vec<i16> {
    (0..frames * usize::from(channels))
        .map(|i| ((i % 65536) as i32 - 32768) as i16)
        .collect()
}
pub(crate) fn wave(rate: u32, channels: u16, pcm: &[i16]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend(b"RIFF");
    data.extend(
        (36 + pcm.len() * 2)
            .try_into()
            .map(u32::to_le_bytes)
            .unwrap(),
    );
    data.extend(b"WAVEfmt ");
    data.extend(16u32.to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(channels.to_le_bytes());
    data.extend(rate.to_le_bytes());
    data.extend((rate * u32::from(channels) * 2).to_le_bytes());
    data.extend((channels * 2).to_le_bytes());
    data.extend(16u16.to_le_bytes());
    data.extend(b"data");
    data.extend((pcm.len() as u32 * 2).to_le_bytes());
    data.extend(pcm.iter().flat_map(|v| v.to_le_bytes()));
    data
}
pub(crate) fn riff(chunks: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let mut out = b"RIFF\0\0\0\0WAVE".to_vec();
    for (kind, bytes) in chunks {
        out.extend(kind);
        out.extend((bytes.len() as u32).to_le_bytes());
        out.extend(&bytes);
        if bytes.len() % 2 != 0 {
            out.push(0);
        }
    }
    let length = out.len() as u32 - 8;
    out[4..8].copy_from_slice(&length.to_le_bytes());
    out
}
