//! Deterministic transport/parse ownership checks on synthetic loopback text.
use super::*;
use futures_util::FutureExt;
use std::{
    future::Future,
    sync::{Mutex, mpsc as blocking_channel},
};
use tokio::sync::oneshot;

const LIFECYCLE_WAIT: Duration = Duration::from_secs(5);
const SHORT_DEADLINE: Duration = Duration::from_millis(250);
const SRT: &[u8] = b"1\n00:00:00,001 --> 00:00:00,051\nsynthetic subtitle\n";

fn run_lifecycle(work: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap()
        .block_on(work);
}
fn policy(cpu: Arc<Semaphore>, budget: Arc<Semaphore>) -> FetchPolicy {
    FetchPolicy {
        cpu,
        budget,
        timeout: LIFECYCLE_WAIT,
        before_parse: None,
    }
}
async fn event<T>(received: oneshot::Receiver<T>) -> T {
    tokio::time::timeout(LIFECYCLE_WAIT, received)
        .await
        .unwrap()
        .unwrap()
}
async fn finished_fetch(
    task: tokio::task::JoinHandle<Result<PreparedRemoteSubtitle, RemoteSubtitleError>>,
) -> Result<PreparedRemoteSubtitle, RemoteSubtitleError> {
    tokio::time::timeout(LIFECYCLE_WAIT, task)
        .await
        .unwrap()
        .unwrap()
}
async fn aborted_fetch(
    task: tokio::task::JoinHandle<Result<PreparedRemoteSubtitle, RemoteSubtitleError>>,
) {
    let Err(error) = task.await else {
        panic!("synthetic fetch unexpectedly completed");
    };
    assert!(error.is_cancelled());
}
async fn capacity_returned(pool: &Arc<Semaphore>, count: u32) {
    let permit = tokio::time::timeout(LIFECYCLE_WAIT, pool.clone().acquire_many_owned(count))
        .await
        .unwrap()
        .unwrap();
    drop(permit);
}
async fn reply(server: &tokio::net::TcpListener) {
    serve(server, 200, "Content-Type: application/x-subrip\r\n", SRT).await;
}

#[test]
fn remote_subtitle_unpolled_drop_is_passive_and_never_cancels_parent() {
    run_lifecycle(async {
        let server = listener().await;
        let cpu = Arc::new(Semaphore::new(1));
        let budget = Arc::new(Semaphore::new(2));
        let parent = ResolverCancellation::default();
        let work = fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            1,
            parent.clone(),
            policy(cpu.clone(), budget.clone()),
        );
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(budget.available_permits(), 2);
        drop(work);
        assert!(!parent.is_cancelled());
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(budget.available_permits(), 2);
        no_request(&server).await;
    });
}

#[test]
fn remote_subtitle_drop_or_parent_cancel_closes_blocked_body_and_returns_capacity() {
    run_lifecycle(async {
        for cancel_parent in [false, true] {
            let server = listener().await;
            let cpu = Arc::new(Semaphore::new(1));
            let budget = Arc::new(Semaphore::new(2));
            let parent = ResolverCancellation::default();
            let task = tokio::spawn(fetch_with(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                1,
                parent.clone(),
                policy(cpu.clone(), budget.clone()),
            ));
            let mut socket = accept(&server).await;
            request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-subrip\r\nContent-Length: 99\r\nConnection: close\r\n\r\n1\n")
                .await
                .unwrap();
            assert_eq!(budget.available_permits(), 1);
            if cancel_parent {
                parent.cancel();
                assert_eq!(
                    finished_fetch(task).await.err(),
                    Some(RemoteSubtitleError::Cancelled)
                );
            } else {
                task.abort();
                aborted_fetch(task).await;
                assert!(!parent.is_cancelled());
            }
            closed(&mut socket).await;
            capacity_returned(&budget, 2).await;
            assert_eq!(cpu.available_permits(), 1);
        }
    });
}

#[test]
fn remote_subtitle_entire_request_deadline_closes_unfinished_body_without_parent_cancel() {
    run_lifecycle(async {
        let server = listener().await;
        let cpu = Arc::new(Semaphore::new(1));
        let budget = Arc::new(Semaphore::new(2));
        let parent = ResolverCancellation::default();
        let mut selected = policy(cpu.clone(), budget.clone());
        selected.timeout = SHORT_DEADLINE;
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            3,
            parent.clone(),
            selected,
        ));
        let mut socket = accept(&server).await;
        request(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-subrip\r\nContent-Length: 99\r\nConnection: close\r\n\r\n1\n")
            .await
            .unwrap();
        assert_eq!(
            finished_fetch(task).await.err(),
            Some(RemoteSubtitleError::Timeout)
        );
        closed(&mut socket).await;
        assert!(!parent.is_cancelled());
        capacity_returned(&budget, 2).await;
        assert_eq!(cpu.available_permits(), 1);
    });
}

#[test]
fn remote_subtitle_two_retained_results_bound_requests_and_drop_recovers_capacity() {
    run_lifecycle(async {
        let server = listener().await;
        let cpu = Arc::new(Semaphore::new(1));
        let budget = Arc::new(Semaphore::new(2));
        let parent = ResolverCancellation::default();
        let mut retained = Vec::new();
        for sequence in [1, 2] {
            let task = tokio::spawn(fetch_with(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                sequence,
                parent.clone(),
                policy(cpu.clone(), budget.clone()),
            ));
            reply(&server).await;
            let text = finished_fetch(task).await.unwrap();
            assert_eq!(text.sequence(), sequence);
            assert_eq!(text.identity(), identity());
            assert_eq!(text.original().unwrap().cues.len(), 1);
            retained.push(text);
        }
        assert_eq!(budget.available_permits(), 0);
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(
            fetch_with(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                3,
                parent.clone(),
                policy(cpu.clone(), budget.clone()),
            )
            .await
            .err(),
            Some(RemoteSubtitleError::Overloaded)
        );
        assert!(!parent.is_cancelled());
        assert!(retained.iter().all(|text| text.original().is_ok()));
        no_request(&server).await;
        drop(retained.pop());
        assert_eq!(budget.available_permits(), 1);
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            3,
            parent.clone(),
            policy(cpu.clone(), budget.clone()),
        ));
        reply(&server).await;
        let replacement = finished_fetch(task).await.unwrap();
        assert_eq!(replacement.sequence(), 3);
        drop(replacement);
        drop(retained);
        assert_eq!(budget.available_permits(), 2);
        assert!(!parent.is_cancelled());
    });
}

#[test]
fn remote_subtitle_cpu_saturation_returns_typed_error_and_releases_wire_lease() {
    run_lifecycle(async {
        let server = listener().await;
        let cpu = Arc::new(Semaphore::new(1));
        let budget = Arc::new(Semaphore::new(2));
        let held = cpu.clone().try_acquire_owned().unwrap();
        let parent = ResolverCancellation::default();
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            1,
            parent.clone(),
            policy(cpu.clone(), budget.clone()),
        ));
        reply(&server).await;
        assert_eq!(
            finished_fetch(task).await.err(),
            Some(RemoteSubtitleError::Overloaded)
        );
        assert_eq!(budget.available_permits(), 2);
        assert!(!parent.is_cancelled());
        drop(held);
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            2,
            parent,
            policy(cpu.clone(), budget.clone()),
        ));
        reply(&server).await;
        drop(finished_fetch(task).await.unwrap());
        assert_eq!(cpu.available_permits(), 1);
        assert_eq!(budget.available_permits(), 2);
    });
}

struct CleanupNotice(Option<oneshot::Sender<()>>);
impl Drop for CleanupNotice {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

#[test]
fn remote_subtitle_started_parse_holds_both_permits_until_actual_cleanup_after_drop_cancel_or_timeout()
 {
    run_lifecycle(async {
        for action in 0..3 {
            let server = listener().await;
            let cpu = Arc::new(Semaphore::new(1));
            let budget = Arc::new(Semaphore::new(1));
            let parent = ResolverCancellation::default();
            let (entered, started) = oneshot::channel();
            let entered = Mutex::new(Some(entered));
            let (cleaned, cleanup) = oneshot::channel();
            let notice = CleanupNotice(Some(cleaned));
            // Dropping the release sender also wakes a worker during test
            // unwinding, so a failed assertion cannot strand runtime shutdown.
            let (release, proceed) = blocking_channel::channel::<()>();
            let proceed = Mutex::new(proceed);
            let mut selected = policy(cpu.clone(), budget.clone());
            selected.timeout = if action == 2 {
                SHORT_DEADLINE
            } else {
                LIFECYCLE_WAIT
            };
            selected.before_parse = Some(Arc::new(move |child| {
                let _retain_notice = &notice;
                assert!(
                    entered
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send(child.clone())
                        .is_ok()
                );
                let _ = proceed.lock().unwrap().recv();
            }));
            let task = tokio::spawn(fetch_with(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                1,
                parent.clone(),
                selected,
            ));
            reply(&server).await;
            let child: ResolverCancellation = event(started).await;
            assert_eq!(cpu.available_permits(), 0);
            assert_eq!(budget.available_permits(), 0);
            match action {
                0 => {
                    task.abort();
                    aborted_fetch(task).await;
                }
                1 => {
                    parent.cancel();
                    assert_eq!(
                        finished_fetch(task).await.err(),
                        Some(RemoteSubtitleError::Cancelled)
                    );
                }
                2 => assert_eq!(
                    finished_fetch(task).await.err(),
                    Some(RemoteSubtitleError::Timeout)
                ),
                _ => unreachable!(),
            }
            assert!(child.is_cancelled());
            assert_eq!(parent.is_cancelled(), action == 1);
            assert_eq!(cpu.available_permits(), 0);
            assert_eq!(budget.available_permits(), 0);
            assert_eq!(
                fetch_with(
                    input(&server, SubtitleFormat::Srt, language()),
                    identity(),
                    2,
                    ResolverCancellation::default(),
                    policy(cpu.clone(), budget.clone()),
                )
                .await
                .err(),
                Some(RemoteSubtitleError::Overloaded)
            );
            drop(release);
            event(cleanup).await;
            capacity_returned(&cpu, 1).await;
            capacity_returned(&budget, 1).await;
            no_request(&server).await;
        }
    });
}

#[test]
fn remote_subtitle_public_entry_rejects_missing_runtime_and_invalid_identity_before_io() {
    for (selected_identity, sequence, expected) in [
        (identity(), 1, RemoteSubtitleError::RuntimeUnavailable),
        (
            StreamIdentity {
                session: SessionId(0),
                ..identity()
            },
            1,
            RemoteSubtitleError::InvalidRequest,
        ),
        (
            StreamIdentity {
                generation: 0,
                ..identity()
            },
            1,
            RemoteSubtitleError::InvalidRequest,
        ),
        (identity(), 0, RemoteSubtitleError::InvalidRequest),
    ] {
        let parent = ResolverCancellation::default();
        let input = PreparedSubtitleUrl::new(
            "https://example.invalid/private?ticket=synthetic-secret",
            SubtitleFormat::Srt,
            language(),
        )
        .unwrap();
        // The public entry must finish synchronously during its first poll.
        // No Tokio runtime, DNS lookup, HTTP connection or private pool is used.
        let result = fetch_remote_subtitle(input, selected_identity, sequence, parent.clone())
            .now_or_never()
            .expect("metadata/runtime preflight must not wait for network");
        assert_eq!(result.err(), Some(expected));
        assert!(!parent.is_cancelled());
    }
}

#[test]
fn remote_subtitle_public_entry_shares_media_cpu_and_retained_request_capacity() {
    let _serial = crate::media::work::TEST_OWNERS.lock().unwrap();
    run_lifecycle(async {
        let server = listener().await;
        let cpu = crate::media::work::workers();
        let budget = requests();
        let cpu_limit = crate::media::work::worker_limit() as u32;
        assert_eq!(cpu.available_permits(), cpu_limit as usize);
        assert_eq!(budget.available_permits(), MAX_REMOTE_SUBTITLE_REQUESTS);
        let held = cpu.clone().try_acquire_many_owned(cpu_limit).unwrap();
        let parent = ResolverCancellation::default();
        let task = tokio::spawn(fetch_remote_subtitle(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            1,
            parent.clone(),
        ));
        let observed = serve(&server, 200, "Content-Type: application/x-subrip\r\n", SRT).await;
        assert!(observed.starts_with("GET /private?ticket=synthetic-secret HTTP/1.1\r\n"));
        assert_eq!(
            finished_fetch(task).await.err(),
            Some(RemoteSubtitleError::Overloaded)
        );
        assert_eq!(budget.available_permits(), MAX_REMOTE_SUBTITLE_REQUESTS);
        assert!(!parent.is_cancelled());
        no_request(&server).await;
        drop(held);
        let mut retained = Vec::new();
        for sequence in [2, 3] {
            let task = tokio::spawn(fetch_remote_subtitle(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                sequence,
                parent.clone(),
            ));
            reply(&server).await;
            let text = finished_fetch(task).await.unwrap();
            assert_eq!(text.identity(), identity());
            assert_eq!(text.sequence(), sequence);
            let original = text.original().unwrap();
            assert_eq!(original.identity, identity());
            assert_eq!(original.cues[0].start_us, Some(1_000));
            assert_eq!(original.cues[0].duration_us, Some(50_000));
            retained.push(text);
        }
        assert_eq!(cpu.available_permits(), cpu_limit as usize);
        assert_eq!(budget.available_permits(), 0);
        assert_eq!(
            fetch_remote_subtitle(
                input(&server, SubtitleFormat::Srt, language()),
                identity(),
                4,
                parent.clone(),
            )
            .await
            .err(),
            Some(RemoteSubtitleError::Overloaded)
        );
        no_request(&server).await;
        drop(retained.pop());
        assert_eq!(budget.available_permits(), 1);
        let task = tokio::spawn(fetch_remote_subtitle(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            4,
            parent.clone(),
        ));
        reply(&server).await;
        let replacement = finished_fetch(task).await.unwrap();
        assert_eq!(replacement.sequence(), 4);
        assert!(replacement.original().is_ok());
        assert!(retained[0].original().is_ok());
        drop(replacement);
        drop(retained);
        assert!(!parent.is_cancelled());
        capacity_returned(&cpu, cpu_limit).await;
        capacity_returned(&budget, MAX_REMOTE_SUBTITLE_REQUESTS as u32).await;
        assert_eq!(cpu.available_permits(), cpu_limit as usize);
        assert_eq!(budget.available_permits(), MAX_REMOTE_SUBTITLE_REQUESTS);
    });
}

#[test]
fn remote_subtitle_queued_parse_drop_never_enters_parser_and_releases_capacity_on_cleanup() {
    run_lifecycle(async {
        let server = listener().await;
        let cpu = Arc::new(Semaphore::new(1));
        let budget = Arc::new(Semaphore::new(1));
        let (occupied, running) = oneshot::channel();
        let (release, proceed) = blocking_channel::channel::<()>();
        let blocker = tokio::task::spawn_blocking(move || {
            occupied.send(()).unwrap();
            let _ = proceed.recv();
        });
        event(running).await;
        let (entered, mut parser_started) = oneshot::channel();
        let entered = Mutex::new(Some(entered));
        let (cleaned, cleanup) = oneshot::channel();
        let notice = CleanupNotice(Some(cleaned));
        let mut selected = policy(cpu.clone(), budget.clone());
        selected.before_parse = Some(Arc::new(move |_| {
            let _retain_notice = &notice;
            entered.lock().unwrap().take().unwrap().send(()).unwrap();
        }));
        let parent = ResolverCancellation::default();
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            1,
            parent.clone(),
            selected,
        ));
        reply(&server).await;
        // The only blocking worker is occupied. Acquired CPU proves fetch has
        // reached scheduling; parser entry is impossible until this gate opens.
        tokio::time::timeout(LIFECYCLE_WAIT, async {
            while cpu.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            parser_started.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        task.abort();
        aborted_fetch(task).await;
        assert!(!parent.is_cancelled());
        drop(release);
        blocker.await.unwrap();
        event(cleanup).await;
        assert!(matches!(
            parser_started.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        capacity_returned(&cpu, 1).await;
        capacity_returned(&budget, 1).await;
    });
}

#[test]
fn remote_subtitle_old_session_cancel_retires_only_old_access_and_new_session_keeps_timing() {
    run_lifecycle(async {
        let server = listener().await;
        let cpu = Arc::new(Semaphore::new(1));
        let budget = Arc::new(Semaphore::new(2));
        let old_parent = ResolverCancellation::default();
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            identity(),
            7,
            old_parent.clone(),
            policy(cpu.clone(), budget.clone()),
        ));
        reply(&server).await;
        let old = finished_fetch(task).await.unwrap();
        let new_parent = ResolverCancellation::default();
        let current_identity = StreamIdentity {
            session: SessionId(identity().session.0 + 1),
            generation: identity().generation + 1,
        };
        let task = tokio::spawn(fetch_with(
            input(&server, SubtitleFormat::Srt, language()),
            current_identity,
            1,
            new_parent.clone(),
            policy(cpu.clone(), budget.clone()),
        ));
        reply(&server).await;
        let current = finished_fetch(task).await.unwrap();
        old_parent.cancel();
        assert_eq!(old.original().err(), Some(RemoteSubtitleError::Cancelled));
        let original = current.original().unwrap();
        assert_eq!(original.identity, current_identity);
        assert_eq!(current.sequence(), 1);
        assert_eq!(original.cues[0].start_us, Some(1_000));
        assert_eq!(original.cues[0].duration_us, Some(50_000));
        assert!(!new_parent.is_cancelled());
        // Retirement denies content access but does not release ownership while
        // stale bytes remain retained by a result owner.
        assert_eq!(budget.available_permits(), 0);
        drop(old);
        assert_eq!(budget.available_permits(), 1);
        assert!(current.original().is_ok());
        drop(current);
        assert_eq!(budget.available_permits(), 2);
        assert!(!new_parent.is_cancelled());
        for error in [
            RemoteSubtitleError::Cancelled,
            RemoteSubtitleError::Timeout,
            RemoteSubtitleError::Overloaded,
        ] {
            for representation in [error.to_string(), format!("{error:?}")] {
                for private in ["127.0.0.1", "https://", "private", "SYNTHETIC", "subtitle"] {
                    assert!(!representation.contains(private));
                }
            }
        }
    });
}
