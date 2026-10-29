//! A single cancellable discovery worker and a latest-only snapshot mailbox.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tokio::sync::watch;

use super::sessions::{DISCOVERY_INTERVAL, DiscoveryError, DiscoveryUpdate, ScanReport};

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

/// Created, called and dropped on the worker; implementations need not be Send or Sync.
pub trait AudioSessionScanner {
    fn scan(&mut self, cancelled: &AtomicBool) -> Result<ScanReport, DiscoveryError>;
}

pub struct DiscoveryMonitor {
    updates: watch::Receiver<Option<Arc<DiscoveryUpdate>>>,
    cancelled: Arc<AtomicBool>,
    stop: SyncSender<()>,
    worker: Option<JoinHandle<()>>,
    observed: Option<(u64, u64)>,
}

impl DiscoveryMonitor {
    pub fn start<S, F>(factory: F) -> Result<Self, DiscoveryError>
    where
        S: AudioSessionScanner + 'static,
        F: FnOnce() -> Result<S, DiscoveryError> + Send + 'static,
    {
        Self::start_with_interval(factory, DISCOVERY_INTERVAL)
    }

    fn start_with_interval<S, F>(factory: F, interval: Duration) -> Result<Self, DiscoveryError>
    where
        S: AudioSessionScanner + 'static,
        F: FnOnce() -> Result<S, DiscoveryError> + Send + 'static,
    {
        let run_id = NEXT_RUN_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| DiscoveryError::InvalidData("discovery_id_exhausted"))?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (stop, stop_receiver) = mpsc::sync_channel(1);
        let (sender, updates) = watch::channel(None);
        let worker = thread::Builder::new()
            .name("audio-discovery".into())
            .spawn(move || {
                let started = Instant::now();
                let mut scanner = match factory() {
                    Ok(scanner) => scanner,
                    Err(error) => {
                        sender.send_replace(Some(Arc::new(DiscoveryUpdate {
                            run_id,
                            sequence: 1,
                            elapsed: started.elapsed(),
                            result: Err(error),
                        })));
                        return;
                    }
                };
                let mut sequence = 0_u64;
                while !worker_cancelled.load(Ordering::Acquire) {
                    let result = scanner.scan(&worker_cancelled);
                    if worker_cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let Some(next) = sequence.checked_add(1) else {
                        break;
                    };
                    sequence = next;
                    // The receiver clones Arc immediately; no growing queue or stale replay.
                    sender.send_replace(Some(Arc::new(DiscoveryUpdate {
                        run_id,
                        sequence,
                        elapsed: started.elapsed(),
                        result,
                    })));
                    match stop_receiver.recv_timeout(interval) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                // Scanner and all native resources drop here, on their owning thread.
            })
            .map_err(|_| DiscoveryError::WorkerStart)?;
        Ok(Self {
            updates,
            cancelled,
            stop,
            worker: Some(worker),
            observed: None,
        })
    }

    pub async fn next(&mut self) -> Result<Arc<DiscoveryUpdate>, DiscoveryError> {
        self.updates
            .changed()
            .await
            .map_err(|_| DiscoveryError::WorkerStopped)?;
        let latest = self.updates.borrow_and_update().clone();
        let latest = latest.ok_or(DiscoveryError::WorkerStopped)?;
        self.observed = Some((latest.run_id, latest.sequence));
        Ok(latest)
    }
    /// Main-thread polling clones one Arc and releases the mailbox borrow immediately.
    pub fn poll(&mut self) -> Result<Option<Arc<DiscoveryUpdate>>, DiscoveryError> {
        let latest = self.updates.borrow_and_update().clone();
        if let Some(latest) = latest
            && self.observed != Some((latest.run_id, latest.sequence))
        {
            self.observed = Some((latest.run_id, latest.sequence));
            return Ok(Some(latest));
        }
        if self.updates.has_changed().is_err() {
            return Err(DiscoveryError::WorkerStopped);
        }
        Ok(None)
    }

    pub fn stop(&mut self) -> Result<(), DiscoveryError> {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.stop.try_send(());
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| DiscoveryError::WorkerPanicked)?;
        }
        Ok(())
    }
}

impl Drop for DiscoveryMonitor {
    fn drop(&mut self) {
        // Stop is idempotent; never leave a worker behind on any caller's error path.
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{rc::Rc, sync::atomic::AtomicUsize};

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);
    const TEST_INTERVAL: Duration = Duration::from_millis(2);
    #[test]
    fn polling_delivers_terminal_initialization_error_once_then_reports_closure() {
        let mut monitor = DiscoveryMonitor::start_with_interval(
            || Err::<MockScanner, _>(DiscoveryError::WorkerStart),
            TEST_INTERVAL,
        )
        .unwrap();
        monitor.stop().unwrap();
        let update = monitor.poll().unwrap().unwrap();
        assert_eq!(update.result, Err(DiscoveryError::WorkerStart));
        assert!(matches!(monitor.poll(), Err(DiscoveryError::WorkerStopped)));
    }

    struct MockScanner {
        count: Arc<AtomicUsize>,
        dropped: Arc<AtomicBool>,
        // Proves the factory need not move a thread-affine scanner across threads.
        _thread_affine: Rc<()>,
    }

    impl AudioSessionScanner for MockScanner {
        fn scan(&mut self, _: &AtomicBool) -> Result<ScanReport, DiscoveryError> {
            let count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 2 {
                return Err(DiscoveryError::InvalidData("mock_failure"));
            }
            Ok(ScanReport {
                device_count: count,
                ..ScanReport::default()
            })
        }
    }

    impl Drop for MockScanner {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn mock() -> (DiscoveryMonitor, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let count = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicBool::new(false));
        let worker_count = Arc::clone(&count);
        let worker_dropped = Arc::clone(&dropped);
        let monitor = DiscoveryMonitor::start_with_interval(
            move || {
                Ok(MockScanner {
                    count: worker_count,
                    dropped: worker_dropped,
                    _thread_affine: Rc::new(()),
                })
            },
            TEST_INTERVAL,
        )
        .unwrap();
        (monitor, count, dropped)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn slow_consumer_receives_latest_snapshot_without_a_backlog() {
        let (mut monitor, count, dropped) = mock();
        runtime().block_on(async {
            tokio::time::timeout(TEST_TIMEOUT, async {
                while count.load(Ordering::SeqCst) < 10 {
                    tokio::time::sleep(TEST_INTERVAL).await;
                }
            })
            .await
            .unwrap();
            let update = tokio::time::timeout(TEST_TIMEOUT, monitor.next())
                .await
                .unwrap()
                .unwrap();
            assert!(update.sequence >= 9); // one scan may be between count and publication
            assert!(update.result.as_ref().unwrap().device_count >= 9);
        });
        monitor.stop().unwrap();
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn stop_wakes_long_wait_and_is_idempotent() {
        let dropped = Arc::new(AtomicBool::new(false));
        let worker_dropped = Arc::clone(&dropped);
        let mut monitor = DiscoveryMonitor::start_with_interval(
            move || {
                Ok(MockScanner {
                    count: Arc::new(AtomicUsize::new(0)),
                    dropped: worker_dropped,
                    _thread_affine: Rc::new(()),
                })
            },
            Duration::from_secs(3600),
        )
        .unwrap();
        runtime().block_on(async {
            tokio::time::timeout(TEST_TIMEOUT, monitor.next())
                .await
                .unwrap()
                .unwrap();
        });
        let start = Instant::now();
        monitor.stop().unwrap();
        monitor.stop().unwrap();
        assert!(start.elapsed() < TEST_TIMEOUT);
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn drop_joins_worker_and_restarting_has_a_new_identity() {
        let (mut first, _, first_dropped) = mock();
        let first_id = runtime().block_on(async {
            tokio::time::timeout(TEST_TIMEOUT, first.next())
                .await
                .unwrap()
                .unwrap()
                .run_id
        });
        drop(first);
        assert!(first_dropped.load(Ordering::SeqCst));
        let (mut second, _, second_dropped) = mock();
        let second_id = runtime().block_on(async {
            tokio::time::timeout(TEST_TIMEOUT, second.next())
                .await
                .unwrap()
                .unwrap()
                .run_id
        });
        assert_ne!(first_id, second_id);
        drop(second);
        assert!(second_dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn initialization_failure_is_delivered_then_channel_closes() {
        let mut monitor = DiscoveryMonitor::start(|| -> Result<MockScanner, _> {
            Err(DiscoveryError::InvalidData("mock_init"))
        })
        .unwrap();
        runtime().block_on(async {
            let update = tokio::time::timeout(TEST_TIMEOUT, monitor.next())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(update.result, Err(DiscoveryError::InvalidData("mock_init")));
            assert_eq!(
                monitor.next().await.unwrap_err(),
                DiscoveryError::WorkerStopped
            );
        });
        monitor.stop().unwrap();
    }

    struct ScriptedScanner {
        receiver: mpsc::Receiver<Result<ScanReport, DiscoveryError>>,
    }

    impl AudioSessionScanner for ScriptedScanner {
        fn scan(&mut self, cancelled: &AtomicBool) -> Result<ScanReport, DiscoveryError> {
            loop {
                if cancelled.load(Ordering::Acquire) {
                    return Err(DiscoveryError::Cancelled);
                }
                match self.receiver.recv_timeout(TEST_INTERVAL) {
                    Ok(report) => return report,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err(DiscoveryError::Cancelled),
                }
            }
        }
    }

    #[test]
    fn failures_replace_stale_results_and_recovery_preserves_order() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut monitor = DiscoveryMonitor::start_with_interval(
            move || Ok(ScriptedScanner { receiver }),
            TEST_INTERVAL,
        )
        .unwrap();
        runtime().block_on(async {
            let mut previous_time = Duration::ZERO;
            for (index, expected) in [
                Ok(ScanReport {
                    device_count: 1,
                    ..ScanReport::default()
                }),
                Err(DiscoveryError::InvalidData("disconnected")),
                Ok(ScanReport::default()),
            ]
            .into_iter()
            .enumerate()
            {
                sender.send(expected.clone()).unwrap();
                let update = tokio::time::timeout(TEST_TIMEOUT, monitor.next())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(update.result, expected);
                assert_eq!(update.sequence, index as u64 + 1);
                assert!(update.elapsed >= previous_time);
                previous_time = update.elapsed;
            }
        });
        // Cancellation interrupts the scripted in-flight scan, not just the poll interval.
        monitor.stop().unwrap();
    }

    #[test]
    fn worker_panic_is_reported_and_joined() {
        let mut monitor = DiscoveryMonitor::start(|| -> Result<MockScanner, DiscoveryError> {
            panic!("explicit test panic")
        })
        .unwrap();
        runtime().block_on(async {
            assert_eq!(
                tokio::time::timeout(TEST_TIMEOUT, monitor.next())
                    .await
                    .unwrap()
                    .unwrap_err(),
                DiscoveryError::WorkerStopped
            );
        });
        assert_eq!(monitor.stop(), Err(DiscoveryError::WorkerPanicked));
        monitor.stop().unwrap();
    }
}
