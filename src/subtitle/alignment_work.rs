//! One process-wide CPU admission lane shared by exact and fuzzy alignment.
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;

const MAX_ALIGNMENT_JOBS: usize = 2;

pub(super) fn workers() -> Arc<Semaphore> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    WORKERS
        .get_or_init(|| {
            let cores = std::thread::available_parallelism().map_or(1, usize::from);
            Arc::new(Semaphore::new(cores.min(MAX_ALIGNMENT_JOBS)))
        })
        .clone()
}
