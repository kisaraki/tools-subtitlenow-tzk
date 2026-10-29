use std::{sync::Arc, time::Duration};

use subtitlenow::{audio::sessions::*, ui::source_list::write_update};

fn source() -> AudioSource {
    AudioSource {
        id: AudioSourceId {
            endpoint: "device-a".into(),
            instance: "session-a".into(),
        },
        endpoint_name: Some("測試裝置".into()),
        process_id: 123,
        process_created: Some(1),
        process_scope: ProcessScope::SingleProcess,
        application_name: Some("測試程式".into()),
        window_title: None,
        icon: None,
        state: SessionState::Active,
        peak: Some(0.5),
        volume: Some(1.0),
        muted: Some(false),
    }
}

#[test]
fn source_identity_is_not_just_pid_or_window_title() {
    let first = source();
    let mut other = first.clone();
    other.id.endpoint = "device-b".into();
    assert_ne!(first.id, other.id);
    other.id = first.id.clone();
    other.id.instance = "session-b".into();
    assert_ne!(first.id, other.id);
}

#[test]
fn missing_meter_is_not_silence_and_active_stream_is_not_signal() {
    let mut source = source();
    assert_eq!(source.signal_present(), Some(true));
    source.peak = Some(0.0);
    assert_eq!(source.signal_present(), Some(false));
    source.peak = None;
    assert_eq!(source.signal_present(), None);
    source.state = SessionState::Inactive;
    assert_eq!(source.signal_present(), Some(false));
    for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.01] {
        assert_eq!(normalized_level(invalid), None);
    }
    assert_eq!(normalized_level(0.0), Some(0.0));
    assert_eq!(normalized_level(1.0), Some(1.0));
}

#[test]
fn diagnostic_view_escapes_application_metadata_and_reports_missing_fields() {
    let mut source = source();
    source.application_name = Some("程式\x1b[2J\n偽造列".into());
    source.peak = None;
    source.process_scope = ProcessScope::MultipleProcesses;
    source.icon = Some(Arc::new(ApplicationIcon {
        width: 1,
        height: 1,
        bgra: vec![0, 0, 0, 0],
    }));
    let update = DiscoveryUpdate {
        run_id: 1,
        sequence: 2,
        elapsed: Duration::from_millis(500),
        result: Ok(ScanReport {
            sources: vec![source],
            device_count: 1,
            notifications_lost: true,
            ..ScanReport::default()
        }),
    };
    let mut output = Vec::new();
    write_update(&mut output, &update).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains('\x1b'));
    assert!(!output.contains("\n偽造列"));
    assert!(output.contains("峰值 無法取得"));
    assert!(output.contains("PID 為初始建立者"));
    assert!(output.contains("部分資料無法取得"));
    assert!(output.contains("圖示：已取得"));
}

#[test]
fn diagnostic_view_propagates_output_failure() {
    struct FailingWriter;
    impl std::io::Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let update = DiscoveryUpdate {
        run_id: 1,
        sequence: 1,
        elapsed: Duration::ZERO,
        result: Ok(ScanReport::default()),
    };
    assert_eq!(
        write_update(&mut FailingWriter, &update)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
}
