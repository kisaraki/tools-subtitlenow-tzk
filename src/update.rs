//! Explicit, bounded application updates. No capture data, keys or provider clients.
use std::{path::PathBuf, sync::mpsc, thread::JoinHandle, time::Duration};

use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Write;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHOR: &str = "kisaraki";
pub const WEBSITE: &str = "https://kisaraki.github.io/tools-subtitlenow-tzk/";
pub const REPOSITORY: &str = "https://github.com/kisaraki/tools-subtitlenow-tzk";
const LATEST: &str = "https://api.github.com/repos/kisaraki/tools-subtitlenow-tzk/releases/latest";
const MAX_METADATA: usize = 256 * 1024;
const MAX_INSTALLER: u64 = 128 * 1024 * 1024;
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const TEMP_PREFIX: &str = "subtitlenow-update-";
const STALE_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpdateError {
    #[error("無法連線至 GitHub，請確認網路後再試。")]
    Network,
    #[error("目前尚無可下載的正式版本。")]
    NoRelease,
    #[error("GitHub 暫時限制要求，請稍後再試。")]
    RateLimited,
    #[error("更新資料無效或缺少安全驗證資訊，已停止更新。")]
    InvalidRelease,
    #[error("安裝檔大小或 SHA-256 不符，已刪除下載並停止更新。")]
    Integrity,
    #[error("無法建立或寫入暫存安裝檔，請確認磁碟空間與權限。")]
    Storage,
    #[error("無法啟動安裝程式；目前版本仍可使用。")]
    Launch,
    #[error("更新作業已取消。")]
    Cancelled,
    #[error("無法啟動更新工作。")]
    Worker,
}
type Result<T> = std::result::Result<T, UpdateError>;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    url: String,
    size: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Metadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
    state: String,
}
fn parse_release(bytes: &[u8], current: &str) -> Result<Option<Release>> {
    if bytes.len() > MAX_METADATA {
        return Err(UpdateError::InvalidRelease);
    }
    let metadata: Metadata =
        serde_json::from_slice(bytes).map_err(|_| UpdateError::InvalidRelease)?;
    let version = Version::parse(
        metadata
            .tag_name
            .strip_prefix('v')
            .ok_or(UpdateError::InvalidRelease)?,
    )
    .map_err(|_| UpdateError::InvalidRelease)?;
    let current = Version::parse(current).map_err(|_| UpdateError::InvalidRelease)?;
    if metadata.draft || metadata.prerelease || !version.pre.is_empty() || !version.build.is_empty()
    {
        return Err(UpdateError::InvalidRelease);
    }
    if version <= current {
        return Ok(None);
    }
    let mut assets = metadata
        .assets
        .into_iter()
        .filter(|asset| asset.name == "setup.exe");
    let asset = assets.next().ok_or(UpdateError::InvalidRelease)?;
    let expected_url = format!("{REPOSITORY}/releases/download/v{version}/setup.exe");
    let sha256 = asset
        .digest
        .and_then(|digest| digest.strip_prefix("sha256:").map(str::to_owned))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(UpdateError::InvalidRelease)?;
    if assets.next().is_some()
        || asset.size == 0
        || asset.size > MAX_INSTALLER
        || asset.state != "uploaded"
        || asset.browser_download_url != expected_url
    {
        return Err(UpdateError::InvalidRelease);
    }
    Ok(Some(Release {
        version,
        url: expected_url,
        size: asset.size,
        sha256,
    }))
}
fn allowed_download_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn client(download: bool) -> Result<Client> {
    Client::builder()
        .https_only(true)
        .user_agent(concat!("SubtitleNow/", env!("CARGO_PKG_VERSION")))
        .referer(false)
        .connect_timeout(Duration::from_secs(10))
        .timeout(if download {
            DOWNLOAD_TIMEOUT
        } else {
            CHECK_TIMEOUT
        })
        .redirect(if download {
            reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() <= 4 && allowed_download_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("untrusted update redirect")
                }
            })
        } else {
            reqwest::redirect::Policy::none()
        })
        .build()
        .map_err(|_| UpdateError::Network)
}
fn check_status(status: reqwest::StatusCode) -> Result<()> {
    match status.as_u16() {
        200 => Ok(()),
        404 => Err(UpdateError::NoRelease),
        403 | 429 => Err(UpdateError::RateLimited),
        _ => Err(UpdateError::Network),
    }
}
async fn check() -> Result<Option<Release>> {
    let mut response = client(false)?
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    check_status(response.status())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        if chunk.len() > MAX_METADATA - bytes.len() {
            return Err(UpdateError::InvalidRelease);
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_release(&bytes, VERSION)
}
pub struct StagedInstaller {
    directory: tempfile::TempDir,
}
impl StagedInstaller {
    pub fn path(&self) -> PathBuf {
        self.directory.path().join("setup.exe")
    }
    /// Only verified downloads produce this type. No shell or remote arguments.
    pub fn launch(self) -> Result<()> {
        std::process::Command::new(self.path())
            .arg("/NORESTART")
            .spawn()
            .map_err(|_| UpdateError::Launch)?;
        // Setup must survive this app exiting. A later startup/check cleans only
        // our own stale staging folders; unfinished/cancelled downloads use RAII.
        let _ = self.directory.keep();
        Ok(())
    }
}
async fn download(release: Release) -> Result<StagedInstaller> {
    download_with(client(true)?, release).await
}
async fn download_with(client: Client, release: Release) -> Result<StagedInstaller> {
    let mut response = client
        .get(&release.url)
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    check_status(response.status())?;
    if response
        .content_length()
        .is_some_and(|size| size != release.size)
    {
        return Err(UpdateError::Integrity);
    }
    let directory = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir()
        .map_err(|_| UpdateError::Storage)?;
    let path = directory.path().join("setup.exe");
    // Small bounded writes run only on our dedicated worker. Synchronous file
    // ownership ensures cancellation drops the handle before TempDir cleanup.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| UpdateError::Storage)?;
    let mut size = 0u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UpdateError::Network)? {
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or(UpdateError::Integrity)?;
        if size > release.size {
            return Err(UpdateError::Integrity);
        }
        hash.update(&chunk);
        file.write_all(&chunk).map_err(|_| UpdateError::Storage)?;
    }
    verify_download(size, &hash.finalize(), &release)?;
    file.sync_all().map_err(|_| UpdateError::Storage)?;
    drop(file);
    Ok(StagedInstaller { directory })
}
fn verify_download(size: u64, hash: &[u8], release: &Release) -> Result<()> {
    let actual = hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if size != release.size || !actual.eq_ignore_ascii_case(&release.sha256) {
        return Err(UpdateError::Integrity);
    }
    Ok(())
}

/// Never recurse into unknown content or links. Only a stale directory containing
/// one regular setup.exe can be removed; active downloads are younger than 24h.
pub fn cleanup_stale() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || !meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_AGE)
        {
            continue;
        }
        let path = entry.path();
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        let files: Vec<_> = files.collect();
        if files.len() != 1 {
            continue;
        }
        let Ok(file) = &files[0] else {
            continue;
        };
        if file.file_name() != "setup.exe" || !file.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        if std::fs::remove_file(file.path()).is_ok() {
            let _ = std::fs::remove_dir(path);
        }
    }
}
pub enum Outcome {
    Checked(Option<Release>),
    Downloaded(StagedInstaller),
}
pub struct UpdateJob {
    receiver: mpsc::Receiver<Result<Outcome>>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl UpdateJob {
    pub fn start(release: Option<Release>) -> Result<Self> {
        Self::spawn(move || async move {
            match release {
                Some(release) => download(release).await.map(Outcome::Downloaded),
                None => check().await.map(Outcome::Checked),
            }
        })
    }
    fn spawn<F, Work>(work: F) -> Result<Self>
    where
        F: FnOnce() -> Work + Send + 'static,
        Work: std::future::Future<Output = Result<Outcome>>,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("application-update".into())
            .spawn(move || {
                cleanup_stale();
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(async {
                        tokio::select! { biased;
                            _ = cancelled => Err(UpdateError::Cancelled),
                                result = work() => result,
                        }
                    }),
                    Err(_) => Err(UpdateError::Worker),
                };
                let _ = sender.try_send(result);
            })
            .map_err(|_| UpdateError::Worker)?;
        Ok(Self {
            receiver,
            cancel: Some(cancel),
            thread: Some(thread),
        })
    }
    pub fn poll(&self) -> Option<Result<Outcome>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(UpdateError::Worker)),
        }
    }
}
impl Drop for UpdateJob {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
