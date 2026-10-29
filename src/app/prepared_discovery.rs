//! One caller-owned, cancellable local metadata scan. No credentials or cloud.
use crate::{
    ai::speech::StreamIdentity,
    audio::capture::SessionId,
    media::{
        MediaError,
        embedded_mp4::{Mp4SubtitleTracks, discover_local_mp4_subtitles},
        input::MediaInput,
    },
    resolver::contracts::ResolverCancellation,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread::JoinHandle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiscoveryState {
    #[default]
    Idle,
    Scanning,
    Stopping,
    Found(Mp4SubtitleTracks),
    Cancelled,
    Failed(MediaError),
}
impl DiscoveryState {
    pub const fn busy(self) -> bool {
        matches!(self, Self::Scanning | Self::Stopping)
    }
    pub fn label(self) -> String {
        match self {
            Self::Idle => String::new(),
            Self::Scanning => "正在本機探索 MP4 字幕軌；不使用雲端。".into(),
            Self::Stopping => "正在取消字幕軌探索。".into(),
            Self::Found(info) => {
                if info.len() > 1 {
                    return format!(
                        "找到 {} 條 tx3g 字幕軌；請從清單明確選擇，再核對語言。",
                        info.len()
                    );
                }
                let Some(info) = info.get(0) else {
                    return "沒有可選的字幕軌。".into();
                };
                use crate::media::embedded_mp4::Mp4SubtitleLanguage;
                let language = match info.language {
                    Mp4SubtitleLanguage::Chinese => "中文，請指定繁簡類型",
                    Mp4SubtitleLanguage::Source(_) => "已依容器填入來源語言，請核對",
                    Mp4SubtitleLanguage::Undetermined => "語言未定，請明確指定",
                };
                format!(
                    "找到 tx3g 軌 ID {}：{language}；尚未準備字幕。",
                    info.track_id
                )
            }
            Self::Cancelled => "字幕軌探索已取消。".into(),
            Self::Failed(error) => format!("字幕軌探索：{error}"),
        }
    }
}
struct Worker {
    identity: StreamIdentity,
    cancel: ResolverCancellation,
    thread: JoinHandle<()>,
    result: mpsc::Receiver<(StreamIdentity, Result<Mp4SubtitleTracks, MediaError>)>,
}
#[derive(Default)]
pub struct PreparedDiscovery {
    worker: Option<Worker>,
    state: DiscoveryState,
}
impl PreparedDiscovery {
    pub fn state(&self) -> DiscoveryState {
        self.state
    }
    pub fn start(&mut self, input: MediaInput) -> Result<(), MediaError> {
        self.start_with(move |identity, cancel| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| MediaError::WorkerFailed)?;
            runtime.block_on(discover_local_mp4_subtitles(input, identity, cancel))
            // Runtime Drop joins started blocking I/O before this owner is reaped.
        })
    }
    fn start_with<F>(&mut self, operation: F) -> Result<(), MediaError>
    where
        F: FnOnce(StreamIdentity, ResolverCancellation) -> Result<Mp4SubtitleTracks, MediaError>
            + Send
            + 'static,
    {
        if self.worker.is_some() {
            return Err(MediaError::Overloaded);
        }
        let identity = StreamIdentity {
            session: SessionId::allocate().map_err(|_| MediaError::SessionExhausted)?,
            generation: 1,
        };
        let cancel = ResolverCancellation::default();
        let scope = cancel.clone();
        let (sender, result) = mpsc::sync_channel(1); // One terminal metadata result.
        let thread = std::thread::Builder::new()
            .name("subtitle-track-discovery".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| operation(identity, scope)))
                    .unwrap_or(Err(MediaError::WorkerFailed));
                let _ = sender.send((identity, result));
            })
            .map_err(|_| {
                cancel.cancel();
                self.state = DiscoveryState::Failed(MediaError::WorkerFailed);
                MediaError::WorkerFailed
            })?;
        self.worker = Some(Worker {
            identity,
            cancel,
            thread,
            result,
        });
        self.state = DiscoveryState::Scanning;
        Ok(())
    }
    pub fn poll(&mut self) -> DiscoveryState {
        if !self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            return self.state;
        }
        let Some(worker) = self.worker.take() else {
            return self.state;
        };
        let joined = worker.thread.join();
        // The helper cancels on typed failure too; preserve its actual error.
        // An explicit Stop always retires even an already-buffered success.
        if self.state == DiscoveryState::Stopping {
            self.state = DiscoveryState::Cancelled;
            return self.state;
        }
        self.state = if joined.is_err() {
            DiscoveryState::Failed(MediaError::WorkerFailed)
        } else {
            match worker.result.try_recv() {
                Ok((identity, result)) if identity == worker.identity => match result {
                    Ok(info) if !worker.cancel.is_cancelled() => DiscoveryState::Found(info),
                    Ok(_) | Err(MediaError::Cancelled) => DiscoveryState::Cancelled,
                    Err(error) => DiscoveryState::Failed(error),
                },
                _ => DiscoveryState::Failed(MediaError::WorkerFailed),
            }
        };
        self.state
    }
    pub fn stop(&mut self) {
        self.state = if let Some(worker) = &self.worker {
            worker.cancel.cancel();
            DiscoveryState::Stopping
        } else {
            DiscoveryState::Idle
        };
    }
}
impl Drop for PreparedDiscovery {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.thread.join();
        }
    }
}

#[cfg(test)]
#[path = "prepared_discovery_tests.rs"]
mod tests;
