//! Actual loopback ranges with self-owned synthetic format bytes, not codecs.
use super::*;
use crate::{audio::capture::SessionId, media::input::MediaInput};
use std::{sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

fn identity() -> StreamIdentity {
    StreamIdentity {
        session: SessionId(935),
        generation: 3,
    }
}
fn cpu() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(1))
}
fn gate(
    reader: &mut BlockingReader,
    cancel: &ResolverCancellation,
) -> Result<LocalAudioFormat, MediaError> {
    local_audio_format::probe(reader, reader.media.byte_len(), cancel)
}
enum Behavior {
    Normal,
    Changed,
    Status(u16),
    Truncated,
    Paused(Option<oneshot::Sender<()>>, oneshot::Receiver<()>),
}
struct Endpoint {
    address: std::net::SocketAddr,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
    requests: Arc<Mutex<Vec<(u64, usize)>>>,
}
impl Endpoint {
    async fn new(bytes: Vec<u8>, mut behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, mut stopped) = oneshot::channel();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let task = tokio::spawn(async move {
            let mut index = 0;
            loop {
                let (mut socket, _) = tokio::select! {biased;
                    _ = &mut stopped => break,
                    result = listener.accept() => result.unwrap(),
                };
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                    header.push(byte[0]);
                    assert!(header.len() <= 8192);
                }
                let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
                let span = header
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .unwrap();
                let (first, last) = span.split_once('-').unwrap();
                let (first, last): (usize, usize) = (first.parse().unwrap(), last.parse().unwrap());
                assert!(first <= last && last < bytes.len());
                {
                    let mut rows = seen.lock().unwrap();
                    assert!(rows.len() < 80);
                    rows.push((first as u64, last - first + 1));
                }
                assert!(!header.contains("authorization:"));
                assert!(!header.contains("cookie:"));
                if index > 0 {
                    assert!(header.contains("if-match: \"v1\"\r\n"));
                }
                let tag = if index > 0 && matches!(behavior, Behavior::Changed) {
                    "\"v2\""
                } else {
                    "\"v1\""
                };
                if index > 0
                    && let Behavior::Status(status) = behavior
                {
                    socket.write_all(format!("HTTP/1.1 {status} Synthetic\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                    index += 1;
                    continue;
                }
                let length = last - first + 1;
                socket.write_all(format!("HTTP/1.1 206 Partial\r\nContent-Range: bytes {first}-{last}/{}\r\nETag: {tag}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",bytes.len()).as_bytes()).await.unwrap();
                if index > 0
                    && let Behavior::Paused(ready, resume) = &mut behavior
                {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(());
                    }
                    let mut byte = [0];
                    tokio::select! {biased;
                        _ = &mut stopped => break,
                        closed = socket.read(&mut byte) => {
                            assert!(matches!(closed, Ok(0) | Err(_)));
                            index += 1;
                            continue;
                        },
                        _ = resume => {},
                    }
                }
                let end = if index > 0 && matches!(behavior, Behavior::Truncated) {
                    last
                } else {
                    last + 1
                };
                let _ = socket.write_all(&bytes[first..end]).await;
                index += 1;
            }
        });
        Self {
            address,
            stop,
            task,
            requests,
        }
    }
    async fn open(
        &self,
    ) -> (
        RemoteHttpMedia,
        ResolverCancellation,
        Arc<Semaphore>,
        Arc<Semaphore>,
    ) {
        let cancel = ResolverCancellation::default();
        let owners = Arc::new(Semaphore::new(1));
        let buffers = Arc::new(Semaphore::new(4));
        let media = RemoteHttpMedia::open_for_test(
            MediaInput::remote(&format!(
                "http://{}/wrong.extension?private=SYNTHETIC",
                self.address
            ))
            .unwrap(),
            identity(),
            cancel.clone(),
            owners.clone(),
            buffers.clone(),
        )
        .await
        .unwrap();
        (media, cancel, owners, buffers)
    }
    async fn finish(self) {
        let _ = self.stop.send(());
        self.task.await.unwrap();
    }
}
fn mp3() -> Vec<u8> {
    let mut bytes = vec![0; 834];
    for offset in [0, 417] {
        bytes[offset..offset + 4].copy_from_slice(&[0xff, 0xfb, 0x90, 0xc0]);
    }
    bytes
}
fn adts() -> Vec<u8> {
    let mut bytes = vec![0; 200];
    for offset in [0, 100] {
        bytes[offset..offset + 7].copy_from_slice(&[0xff, 0xf1, 0x4c, 0x80, 12, 0x9f, 0xfc]);
    }
    bytes
}

#[tokio::test]
async fn remote_probe_content_gates_all_four_formats_and_returns_the_same_source() {
    for (bytes, expected) in [
        (b"RIFF\x04\0\0\0WAVE".to_vec(), RemoteAudioFormat::Wave),
        (mp3(), RemoteAudioFormat::Mp3),
        (adts(), RemoteAudioFormat::Adts),
        (
            local_audio_format::mp4_audio_probe_fixture(),
            RemoteAudioFormat::Mp4,
        ),
    ] {
        let endpoint = Endpoint::new(bytes.clone(), Behavior::Normal).await;
        let (media, cancel, owners, buffers) = endpoint.open().await;
        let lane = cpu();
        let result = run(media, lane.clone(), gate).await.unwrap();
        assert_eq!(result.identity(), identity());
        assert_eq!(result.byte_len(), bytes.len() as u64);
        assert_eq!(result.format(), expected);
        assert!(!cancel.is_cancelled());
        assert_eq!(owners.available_permits(), 0);
        assert_eq!(lane.available_permits(), 1);
        assert_eq!(buffers.available_permits(), 4);
        let mut media = result.into_media();
        let sequence = media.next_sequence();
        assert!(sequence > 1);
        let range = media
            .read_range(RemoteRangeRequest::new(identity(), sequence, 0, 4).unwrap())
            .await
            .unwrap();
        assert_eq!(range.bytes().unwrap(), &bytes[..4]);
        drop(media);
        assert!(cancel.is_cancelled());
        assert_eq!(range.bytes().err(), Some(RemoteMediaError::Cancelled));
        drop(range);
        assert_eq!(owners.available_permits(), 1);
        endpoint.finish().await;
    }
}

#[tokio::test]
async fn remote_probe_rejects_unknown_truncated_and_inconsistent_content_without_a_decoder() {
    let mut changed = mp3();
    changed[417 + 2] = 0x94;
    let mut truncated_mp4 = local_audio_format::mp4_audio_probe_fixture();
    truncated_mp4.pop();
    for bytes in [
        vec![0; 100],
        b"#EXTM3U\nhttps://invalid.example/segment".to_vec(),
        mp3()[..500].to_vec(),
        changed,
        truncated_mp4,
    ] {
        let endpoint = Endpoint::new(bytes, Behavior::Normal).await;
        let (media, cancel, owners, buffers) = endpoint.open().await;
        assert!(matches!(
            run(media, cpu(), gate).await,
            Err(RemoteProbeError::Media(MediaError::UnsupportedMedia))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(owners.available_permits(), 1);
        assert_eq!(buffers.available_permits(), 4);
        endpoint.finish().await;
    }
}

#[tokio::test]
async fn remote_probe_range_errors_keep_their_type_even_when_the_source_is_cancelled() {
    for (behavior, expected) in [
        (Behavior::Changed, RemoteMediaError::SourceChanged),
        (Behavior::Status(412), RemoteMediaError::SourceChanged),
        (Behavior::Status(429), RemoteMediaError::RateLimited),
        (Behavior::Truncated, RemoteMediaError::Network),
    ] {
        let endpoint = Endpoint::new(mp3(), behavior).await;
        let (media, cancel, owners, buffers) = endpoint.open().await;
        assert!(
            matches!(run(media,cpu(),gate).await,Err(RemoteProbeError::Remote(error)) if error == expected)
        );
        assert!(cancel.is_cancelled());
        assert_eq!(owners.available_permits(), 1);
        assert_eq!(buffers.available_permits(), 4);
        endpoint.finish().await;
    }
}

#[tokio::test]
async fn remote_probe_bridge_splits_large_reads_and_handles_seek_empty_reads_and_eof() {
    let bytes: Vec<u8> = (0..150_000).map(|n| (n % 251) as u8).collect();
    let endpoint = Endpoint::new(bytes.clone(), Behavior::Normal).await;
    let (media, _, _, buffers) = endpoint.open().await;
    // Explicit adapter test, not a production format/decoder success claim.
    let result = run(media, cpu(), move |reader, _| {
        assert_eq!(reader.seek(SeekFrom::End(-3)).unwrap(), 149_997);
        let mut tail = [0; 3];
        reader.read_exact(&mut tail).unwrap();
        assert_eq!(tail, bytes[149_997..]);
        assert_eq!(reader.read(&mut tail).unwrap(), 0);
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        reader.rewind().unwrap();
        let mut output = vec![0; bytes.len()];
        reader.read_exact(&mut output).unwrap();
        assert_eq!(output, bytes);
        assert_eq!(reader.seek(SeekFrom::Current(-150_000)).unwrap(), 0);
        Ok(LocalAudioFormat::Wave)
    })
    .await
    .unwrap();
    assert_eq!(buffers.available_permits(), 4);
    drop(result);
    let ranges = endpoint.requests.lock().unwrap().clone();
    assert_eq!(
        ranges,
        vec![
            (0, 1),
            (149_997, 3),
            (0, 65_536),
            (65_536, 65_536),
            (131_072, 18_928)
        ]
    );
    endpoint.finish().await;
}

#[tokio::test]
async fn remote_probe_invalid_seeks_are_terminal_before_any_further_request() {
    for from in [
        SeekFrom::Start(u64::MAX),
        SeekFrom::Current(-1),
        SeekFrom::End(1),
        SeekFrom::End(i64::MIN),
    ] {
        let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
        let (media, cancel, owners, _) = endpoint.open().await;
        let result = run(media, cpu(), move |reader, _| {
            assert!(reader.seek(from).is_err());
            assert!(reader.rewind().is_err());
            assert!(reader.read(&mut [0; 1]).is_err());
            Ok(LocalAudioFormat::Wave)
        })
        .await;
        assert!(matches!(
            result,
            Err(RemoteProbeError::Remote(RemoteMediaError::InvalidRequest))
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(owners.available_permits(), 1);
        assert_eq!(endpoint.requests.lock().unwrap().len(), 1);
        endpoint.finish().await;
    }
}

#[tokio::test]
async fn remote_probe_initial_cpu_overload_and_unpolled_drop_do_not_start_a_worker_read() {
    for unpolled in [false, true] {
        let endpoint = Endpoint::new(mp3(), Behavior::Normal).await;
        let (media, cancel, owners, _) = endpoint.open().await;
        let lane = cpu();
        let held = lane.clone().try_acquire_owned().unwrap();
        let future = run(media, lane.clone(), gate);
        if unpolled {
            drop(future);
        } else {
            assert!(matches!(
                future.await,
                Err(RemoteProbeError::Media(MediaError::Overloaded))
            ));
        }
        assert!(cancel.is_cancelled());
        assert_eq!(owners.available_permits(), 1);
        assert_eq!(endpoint.requests.lock().unwrap().len(), 1);
        drop(held);
        endpoint.finish().await;
    }
}

#[tokio::test]
async fn remote_probe_network_wait_releases_cpu_and_reacquisition_never_queues() {
    let (ready, started) = oneshot::channel();
    let (resume, proceed) = oneshot::channel();
    let endpoint = Endpoint::new(mp3(), Behavior::Paused(Some(ready), proceed)).await;
    let (media, cancel, owners, buffers) = endpoint.open().await;
    let lane = cpu();
    let future = run(media, lane.clone(), gate);
    tokio::pin!(future);
    tokio::select! { result=&mut future=>panic!("early result: {:?}",result.err()), _=started=>{} }
    assert_eq!(lane.available_permits(), 1);
    let held = lane.clone().try_acquire_owned().unwrap();
    resume.send(()).unwrap();
    assert!(matches!(
        future.await,
        Err(RemoteProbeError::Media(MediaError::Overloaded))
    ));
    assert!(cancel.is_cancelled());
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(buffers.available_permits(), 4);
    drop(held);
    endpoint.finish().await;
}

#[tokio::test]
async fn remote_probe_inflight_network_cancel_joins_exit_without_partial_metadata() {
    let (ready, started) = oneshot::channel();
    let (_resume, proceed) = oneshot::channel();
    let endpoint = Endpoint::new(mp3(), Behavior::Paused(Some(ready), proceed)).await;
    let (media, cancel, owners, buffers) = endpoint.open().await;
    let lane = cpu();
    let future = run(media, lane.clone(), gate);
    tokio::pin!(future);
    tokio::select! { result=&mut future=>panic!("early result: {:?}",result.err()), _=started=>{} }
    cancel.cancel();
    assert!(matches!(
        future.await,
        Err(RemoteProbeError::Remote(RemoteMediaError::Cancelled))
    ));
    assert_eq!(lane.available_permits(), 1);
    assert_eq!(owners.available_permits(), 1);
    assert_eq!(buffers.available_permits(), 4);
    endpoint.finish().await;
}

#[path = "remote_probe_lifecycle_tests.rs"]
mod lifecycle;
