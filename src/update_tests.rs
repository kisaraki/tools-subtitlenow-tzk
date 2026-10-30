use super::*;
use serde_json::{Value, json};
fn metadata(version: &str) -> Value {
    json!({"tag_name":format!("v{version}"), "draft":false, "prerelease":false,
        "assets":[{"name":"setup.exe", "browser_download_url":format!("{REPOSITORY}/releases/download/v{version}/setup.exe"),
        "size":4, "digest":format!("sha256:{:x}",Sha256::digest(b"test")), "state":"uploaded"}]})
}
fn parse(value: &Value) -> Result<Option<Release>> {
    parse_release(&serde_json::to_vec(value).unwrap(), "0.1.1")
}
#[test]
fn versions_compare_numerically_and_never_downgrade() {
    assert_eq!(
        parse(&metadata("0.1.10")).unwrap().unwrap().version,
        Version::new(0, 1, 10)
    );
    assert!(parse(&metadata("0.1.1")).unwrap().is_none());
    assert!(parse(&metadata("0.1.0")).unwrap().is_none());
    for version in ["1.0.0-beta.1", "1.0.0+build.1", "01.2.3", "garbage"] {
        assert_eq!(
            parse(&metadata(version)).unwrap_err(),
            UpdateError::InvalidRelease
        );
    }
}
#[test]
fn fail_closed_on_untrusted_incomplete_or_ambiguous_release() {
    for (key, value) in [("draft", json!(true)), ("prerelease", json!(true))] {
        let mut data = metadata("0.2.0");
        data[key] = value;
        assert_eq!(parse(&data).unwrap_err(), UpdateError::InvalidRelease);
    }
    for (key, value) in [
        ("digest", Value::Null),
        ("digest", json!("sha256:bad")),
        ("size", json!(0)),
        ("size", json!(MAX_INSTALLER + 1)),
        ("state", json!("new")),
        ("name", json!("evil.exe")),
        (
            "browser_download_url",
            json!("https://evil.example/setup.exe"),
        ),
        (
            "browser_download_url",
            json!(format!("{REPOSITORY}/releases/download/v0.1.0/setup.exe")),
        ),
    ] {
        let mut data = metadata("0.2.0");
        data["assets"][0][key] = value;
        assert_eq!(parse(&data).unwrap_err(), UpdateError::InvalidRelease);
    }
    let mut data = metadata("0.2.0");
    let asset = data["assets"][0].clone();
    data["assets"].as_array_mut().unwrap().push(asset);
    assert_eq!(parse(&data).unwrap_err(), UpdateError::InvalidRelease);
    assert_eq!(
        parse_release(&vec![b' '; MAX_METADATA + 1], VERSION).unwrap_err(),
        UpdateError::InvalidRelease
    );
}
#[test]
fn redirects_require_https_exact_host_and_no_credentials() {
    for url in [
        "http://github.com/a",
        "https://github.com.evil.example/a",
        "https://user@github.com/a",
        "https://github.com:444/a",
        "https://127.0.0.1/a",
    ] {
        assert!(!allowed_download_url(&url.parse().unwrap()));
    }
    assert!(allowed_download_url(
        &"https://release-assets.githubusercontent.com/file?token=opaque"
            .parse()
            .unwrap()
    ));
}
#[test]
fn integrity_rejects_truncated_corrupt_and_oversized_files() {
    let release = parse(&metadata("0.2.0")).unwrap().unwrap();
    assert!(verify_download(4, &Sha256::digest(b"test"), &release).is_ok());
    assert_eq!(
        verify_download(4, &Sha256::digest(b"evil"), &release),
        Err(UpdateError::Integrity)
    );
    assert_eq!(
        verify_download(3, &Sha256::digest(b"test"), &release),
        Err(UpdateError::Integrity)
    );
}
#[tokio::test]
async fn verified_download_is_staged_and_removed_when_not_launched() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0; 2048];
        assert!(stream.read(&mut buf).await.unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest")
            .await
            .unwrap();
    });
    let mut release = parse(&metadata("0.2.0")).unwrap().unwrap();
    release.url = format!("http://{address}/setup.exe");
    let installer = download_with(Client::new(), release).await.unwrap();
    let path = installer.path();
    assert_eq!(std::fs::read(&path).unwrap(), b"test");
    drop(installer);
    assert!(!path.exists());
    server.await.unwrap();
}
#[test]
fn staged_installer_launch_failure_cleans_up() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    assert_eq!(
        StagedInstaller { directory }.launch(),
        Err(UpdateError::Launch)
    );
    assert!(!path.exists());
}

#[test]
fn cancelling_pending_work_joins_and_releases_owned_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_owned();
    let (started, ready) = std::sync::mpsc::sync_channel(1);
    let job = UpdateJob::spawn(move || async move {
        let _owned = directory;
        started.send(()).unwrap();
        std::future::pending::<Result<Outcome>>().await
    })
    .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let start = std::time::Instant::now();
    drop(job);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!path.exists());
}

#[tokio::test]
#[ignore = "Explicit release verification: downloads public GitHub installer, never executes it"]
async fn published_release_download_passes_production_digest_verification() {
    let mut response = client(false).unwrap().get(LATEST).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.unwrap() {
        assert!(bytes.len() + chunk.len() <= MAX_METADATA);
        bytes.extend_from_slice(&chunk);
    }
    let release = parse_release(&bytes, "0.0.0").unwrap().unwrap();
    let installer = download(release).await.unwrap();
    let path = installer.path();
    assert!(path.is_file());
    drop(installer);
    assert!(!path.exists());
}
