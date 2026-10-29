//! One process-wide CPU lane for Prepared reads, localization and media decode.
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;

/// Bound codec/file/CPU work together; independent components must not each
/// multiply the available core count. Capture and async network do not use it.
const MAX_MEDIA_WORKERS: usize = 2;
/// Bound native decoder lifetimes independently of CPU work. A demand-driven
/// owner may retain its COM apartment and input pin while waiting for ASR;
/// that idle lifetime must not consume a CPU permit or permit unlimited actors.
const MAX_NATIVE_MEDIA_OWNERS: usize = 2;
// Production-entry fixtures share real CPU/native/remote-owner lanes. Serialize
// fixtures so unrelated tests cannot create incidental overload; dedicated
// worker/session tests inject their own lanes and exercise saturation directly.
#[cfg(test)]
pub(crate) static TEST_OWNERS: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn workers() -> Arc<Semaphore> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    WORKERS
        .get_or_init(|| Arc::new(Semaphore::new(worker_limit())))
        .clone()
}

/// Stable declared CPU capacity, independent of transient active permits. ASR
/// adapters use it to avoid scheduling two encoders against a one-core lane.
pub(crate) fn worker_limit() -> usize {
    static LIMIT: OnceLock<usize> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(MAX_MEDIA_WORKERS)
    })
}

pub(crate) fn native_owners() -> Arc<Semaphore> {
    static OWNERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    OWNERS
        .get_or_init(|| {
            let cores = std::thread::available_parallelism().map_or(1, usize::from);
            Arc::new(Semaphore::new(cores.min(MAX_NATIVE_MEDIA_OWNERS)))
        })
        .clone()
}
