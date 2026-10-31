//! Real SSH/SFTP packets over loopback; virtual server fixtures, not user hosts.
pub mod support;
use russh::keys::{
    Algorithm, PrivateKey,
    ssh_key::{HashAlg, LineEnding},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::Fixture;
use yuuterm_core::{Profile, SshAuth};
use yuuterm_ssh::{Command, Engine, LiveSession, Snapshot};
async fn wait(session: &LiveSession, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = session.snapshot();
            if predicate(&snapshot) {
                return snapshot;
            }
            assert!(
                !snapshot.finished,
                "Unexpected disconnect: {}",
                snapshot.status
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let s = session.snapshot();
        panic!(
            "fixture timed out: status={}, files={}, cells={}",
            s.status,
            s.files_status,
            text(&s)
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recursive_download_refuses_aggregate_names_and_oversized_child_paths() {
    use tokio_util::sync::CancellationToken;
    use yuuterm_ssh::files::FileAction;
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/security-review-tests");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("fixture-key");
    key.write_openssh_file(&key_path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Adversarial tree".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = key_path.to_string_lossy().into();
    let session = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.files.len() == 2).await;
    let attrs = russh_sftp::protocol::FileAttributes {
        size: Some(0),
        permissions: Some(0o100600),
        ..Default::default()
    };
    for (test, count, length, expected) in [
        ("names", 1100, 1024, "1 MiB filename budget"),
        ("path", 1, 4090, "4096-byte limit"),
    ] {
        {
            let mut files = fixture.files.lock().unwrap();
            files.retain(|path, _| !path.starts_with("/fixture/logs/"));
            for i in 0..count {
                let name = format!("{i:04}-{}", "x".repeat(length - 5));
                files.insert(
                    format!("/fixture/logs/{name}"),
                    support::FixtureFile {
                        data: vec![],
                        attrs: attrs.clone(),
                    },
                );
            }
        }
        let local = dir.path().join(test);
        let error = tokio::time::timeout(
            Duration::from_secs(15),
            session.files().execute(
                FileAction::DownloadTree {
                    path: "/fixture/logs".into(),
                    local: local.clone(),
                },
                CancellationToken::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
        assert_eq!(
            std::fs::read_dir(local).unwrap().count(),
            0,
            "reject before any file publication"
        );
    }
    session.disconnect();
}

fn text(snapshot: &Snapshot) -> String {
    snapshot
        .terminal
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_browse_publishes_canonical_paths_and_preserves_failed_listings() {
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("fixture-key");
    key.write_openssh_file(&key_path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Browse acknowledgement".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = key_path.to_string_lossy().into();
    let session = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.path == "/fixture" && s.files.len() == 2).await;
    support::add_navigation_tree(&fixture.files);
    let files = session.files();
    assert_eq!(
        files.browse("/fixture/logs/one".into()).await.unwrap(),
        "/fixture/logs/one"
    );
    assert_eq!(session.snapshot().path, "/fixture/logs/one");
    let before = session.snapshot().files;
    assert!(
        files
            .browse("/missing-fixture-directory".into())
            .await
            .is_err()
    );
    assert_eq!(session.snapshot().path, "/fixture/logs/one");
    assert!(Arc::ptr_eq(&before, &session.snapshot().files));
    let first = files.browse("/fixture/logs/one/..".into()).await.unwrap();
    let second = files.browse(format!("{first}/..")).await.unwrap();
    assert_eq!(first, "/fixture/logs");
    assert_eq!(second, "/fixture");
    assert_eq!(session.snapshot().path, "/fixture");
    session.disconnect();
    wait(&session, |s| s.finished).await;
    assert!(files.browse("/fixture/logs".into()).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn x11_opt_in_auth_rewrite_channel_limits_and_disconnect() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use yuuterm_ssh::x11::LocalX11;
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("test-key");
    key.write_openssh_file(&key_path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "X11 transport fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = key_path.to_string_lossy().into();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let real = [0x57; 16];
    let session = engine
        .connect_x11(
            profile.clone(),
            Arc::new(|| {}),
            LocalX11::new(endpoint, real).unwrap(),
        )
        .unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    let listed = wait(&session, |s| s.connected && s.files.len() == 2).await;
    // Listing metadata must survive the actual SFTP reply and desktop JSON seam.
    let file = listed
        .files
        .iter()
        .find(|f| f.name == "read me.txt")
        .unwrap();
    let wire = serde_json::to_value(file).unwrap();
    assert_eq!(wire["size"], 42);
    assert_eq!(wire["uid"], 1000);
    assert_eq!(wire["gid"], 1000);
    assert_eq!(wire["owner"], "fixture-user");
    assert_eq!(wire["group"], "fixture-group");
    assert_eq!(wire["mtime"], 1700000000);
    let directory = listed.files.iter().find(|f| f.name == "logs").unwrap();
    let wire = serde_json::to_value(directory).unwrap();
    assert!(wire["uid"].is_null() && wire["gid"].is_null() && wire["mtime"].is_null());
    let cookie = {
        let request = fixture.x11.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert_eq!(request.protocol, "MIT-MAGIC-COOKIE-1");
        assert!(request.before_pty && !request.single);
        assert_eq!(request.screen, 0);
        assert_eq!(request.cookie.len(), 32);
        let mut fake = [0; 16];
        for (i, slot) in fake.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&request.cookie[i * 2..i * 2 + 2], 16).unwrap();
        }
        assert!(fake != real, "Local cookie must never be sent over SSH");
        fake
    };
    fn setup(cookie: [u8; 16]) -> Vec<u8> {
        let mut data = vec![0; 48];
        data[0] = b'l';
        data[2] = 11;
        data[6] = 18;
        data[8] = 16;
        data[12..30].copy_from_slice(b"MIT-MAGIC-COOKIE-1");
        data[32..].copy_from_slice(&cookie);
        data
    }
    let server = fixture.x11.server.lock().unwrap().clone().unwrap();
    let mut stream = tokio::time::timeout(
        Duration::from_secs(2),
        server.channel_open_x11("ignored-originator.invalid", 123),
    )
    .await
    .unwrap()
    .unwrap()
    .into_stream();
    let mut wire = setup(cookie);
    wire.extend_from_slice(b"ping");
    stream.write_all(&wire[..7]).await.unwrap();
    stream.write_all(&wire[7..]).await.unwrap();
    let (mut local, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut received = [0; 52];
    local.read_exact(&mut received).await.unwrap();
    assert!(received[32..48].iter().all(|b| *b == 0x57));
    assert_eq!(&received[48..], b"ping");
    local.write_all(b"pong").await.unwrap();
    let mut reply = [0; 4];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"pong");
    assert_eq!(session.x11_stats().unwrap().authenticated, 1);
    let mut wrong = server
        .channel_open_x11("127.0.0.1", 123)
        .await
        .unwrap()
        .into_stream();
    wrong.write_all(&setup([0; 16])).await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), wrong.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(80), listener.accept())
            .await
            .is_err(),
        "Invalid authentication must not connect locally"
    );
    let mut pending = Vec::new();
    for _ in 0..7 {
        pending.push(
            server
                .channel_open_x11("127.0.0.1", 123)
                .await
                .unwrap()
                .into_stream(),
        );
    }
    assert!(server.channel_open_x11("127.0.0.1", 123).await.is_err());
    assert_eq!(session.x11_stats().unwrap().active, 8);
    session
        .send(Command::Input(b"x11-terminal-alive".to_vec()))
        .unwrap();
    wait(&session, |s| text(s).contains("x11-terminal-alive")).await;
    session.disconnect();
    wait(&session, |s| s.finished).await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), local.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while session.x11_stats().unwrap().active != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(pending);
    *fixture.x11.request.lock().unwrap() = None;
    let disabled = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    wait(&disabled, |s| s.connected).await;
    assert!(disabled.x11_stats().is_none());
    assert!(fixture.x11.request.lock().unwrap().is_none());
    let server = fixture.x11.server.lock().unwrap().clone().unwrap();
    assert!(server.channel_open_x11("127.0.0.1", 123).await.is_err());
    disabled.disconnect();
    wait(&disabled, |s| s.finished).await;
    fixture.x11.reject.store(true, Ordering::SeqCst);
    let refused = engine
        .connect_x11(
            profile,
            Arc::new(|| {}),
            LocalX11::new(endpoint, real).unwrap(),
        )
        .unwrap();
    let result = wait(&refused, |s| s.finished).await;
    assert!(result.status.contains("Server rejected X11 forwarding"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_file_actions_transfer_conflict_and_reconnect_boundaries() {
    use tokio_util::sync::CancellationToken;
    use yuuterm_ssh::files::{FileAction as A, FileResult};
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("key");
    key.write_openssh_file(&key_path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "File actions fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = key_path.to_string_lossy().into();
    let session = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.files.len() == 2).await;
    let files = session.files();
    async fn operation(files: &yuuterm_ssh::files::FileClient, a: A) -> Result<FileResult, String> {
        tokio::time::timeout(
            Duration::from_secs(8),
            files.execute(a, CancellationToken::new()),
        )
        .await
        .expect("SFTP action timeout")
    }
    let original = "/fixture/read me.txt".to_owned();
    let properties = operation(
        &files,
        A::Properties {
            path: original.clone(),
        },
    )
    .await
    .unwrap()
    .properties
    .unwrap();
    assert_eq!(properties.uid, Some(1000));
    assert_eq!(properties.permissions, Some(0o100600));
    operation(
        &files,
        A::Permissions {
            path: original.clone(),
            mode: 0o4750,
            recursive: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fixture.files.lock().unwrap()[&original].attrs.permissions,
        Some(0o104750)
    );
    let renamed = "/fixture/renamed file.txt".to_owned();
    operation(
        &files,
        A::Rename {
            path: original.clone(),
            name: "renamed file.txt".into(),
        },
    )
    .await
    .unwrap();
    assert!(!fixture.files.lock().unwrap().contains_key(&original));
    assert!(fixture.files.lock().unwrap().contains_key(&renamed));
    assert!(
        operation(
            &files,
            A::Rename {
                path: renamed.clone(),
                name: "logs".into()
            }
        )
        .await
        .unwrap_err()
        .contains("exists")
    );
    assert!(
        operation(
            &files,
            A::Rename {
                path: renamed.clone(),
                name: "../escape".into()
            }
        )
        .await
        .is_err()
    );
    let local = dir.path().join("edit.txt");
    let baseline = operation(
        &files,
        A::Download {
            path: renamed.clone(),
            local: local.clone(),
            edit: true,
        },
    )
    .await
    .unwrap()
    .fingerprint
    .unwrap();
    assert_eq!(std::fs::read(&local).unwrap().len(), 42);
    std::fs::write(&local, b"saved local version").unwrap();
    let uploaded = operation(
        &files,
        A::Replace {
            expected_local: None,
            path: renamed.clone(),
            local: local.clone(),
            expected: baseline,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fixture.files.lock().unwrap()[&renamed].data,
        b"saved local version"
    );
    assert_eq!(
        fixture.files.lock().unwrap()[&renamed].attrs.permissions,
        Some(0o104750)
    );
    let baseline = uploaded.fingerprint.unwrap();
    std::fs::write(&local, b"second save").unwrap();
    let baseline = operation(
        &files,
        A::Replace {
            expected_local: None,
            path: renamed.clone(),
            local: local.clone(),
            expected: baseline,
        },
    )
    .await
    .unwrap()
    .fingerprint
    .unwrap();
    // Same size/mtime, different bytes: content hashing must still detect this.
    fixture
        .files
        .lock()
        .unwrap()
        .get_mut(&renamed)
        .unwrap()
        .data[0] = b'X';
    assert!(
        operation(
            &files,
            A::Replace {
                expected_local: None,
                path: renamed.clone(),
                local: local.clone(),
                expected: baseline
            }
        )
        .await
        .unwrap_err()
        .contains("Conflict")
    );
    assert_eq!(fixture.files.lock().unwrap()[&renamed].data[0], b'X');
    assert!(local.exists());
    let binary: Vec<u8> = (0..100000).map(|i| (i % 251) as u8).collect();
    std::fs::write(&local, &binary).unwrap();
    let target = "/fixture/binary.bin".to_owned();
    operation(
        &files,
        A::Upload {
            path: target.clone(),
            local: local.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(fixture.files.lock().unwrap()[&target].data, binary);
    assert!(
        operation(
            &files,
            A::Upload {
                path: target.clone(),
                local: local.clone()
            }
        )
        .await
        .unwrap_err()
        .contains("exists")
    );
    let downloaded = dir.path().join("download.bin");
    operation(
        &files,
        A::Download {
            path: target.clone(),
            local: downloaded.clone(),
            edit: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(downloaded).unwrap(), binary);
    let mut symlink = fixture.files.lock().unwrap()[&target].clone();
    symlink.attrs.permissions = Some(0o120777);
    fixture
        .files
        .lock()
        .unwrap()
        .insert("/fixture/link".into(), symlink);
    assert!(
        operation(
            &files,
            A::Download {
                path: "/fixture/link".into(),
                local: dir.path().join("link"),
                edit: false
            }
        )
        .await
        .is_err()
    );
    assert!(
        operation(
            &files,
            A::Permissions {
                path: "/fixture/link".into(),
                mode: 0o600,
                recursive: true,
            }
        )
        .await
        .is_err()
    );
    {
        let mut tree = fixture.files.lock().unwrap();
        let file = tree[&target].clone();
        let mut directory = file.clone();
        directory.attrs.permissions = Some(0o040700);
        let mut link = file.clone();
        link.attrs.permissions = Some(0o120777);
        for (path, entry) in [
            ("/fixture/tree", &directory),
            ("/fixture/tree/a.txt", &file),
            ("/fixture/tree/sub", &directory),
            ("/fixture/tree/sub/b.txt", &file),
            ("/fixture/tree/link", &link),
        ] {
            tree.insert(path.into(), entry.clone());
        }
    }
    let tree = operation(
        &files,
        A::Permissions {
            path: "/fixture/tree".into(),
            mode: 0o750,
            recursive: true,
        },
    )
    .await
    .unwrap();
    assert_eq!((tree.changed, tree.skipped), (4, 1));
    {
        let tree = fixture.files.lock().unwrap();
        let mode = |path: &str| tree[path].attrs.permissions.unwrap();
        assert_eq!(mode("/fixture/tree"), 0o040750);
        assert_eq!(mode("/fixture/tree/sub"), 0o040750);
        assert_eq!(mode("/fixture/tree/a.txt") & 0o7777, 0o750);
        assert_eq!(mode("/fixture/tree/sub/b.txt") & 0o7777, 0o750);
        assert_eq!(mode("/fixture/tree/link"), 0o120777);
    }
    fixture
        .files
        .lock()
        .unwrap()
        .retain(|path, _| !path.starts_with("/fixture/tree"));
    let child = fixture.files.lock().unwrap()[&target].clone();
    fixture
        .files
        .lock()
        .unwrap()
        .insert("/fixture/logs/child".into(), child);
    assert!(
        operation(
            &files,
            A::Delete {
                path: "/fixture/logs".into()
            }
        )
        .await
        .is_err()
    );
    operation(
        &files,
        A::Delete {
            path: "/fixture/logs/child".into(),
        },
    )
    .await
    .unwrap();
    operation(
        &files,
        A::Delete {
            path: "/fixture/logs".into(),
        },
    )
    .await
    .unwrap();
    operation(
        &files,
        A::Delete {
            path: target.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!fixture.files.lock().unwrap().contains_key(&target));
    session.disconnect();
    wait(&session, |s| s.finished).await;
    assert!(
        operation(
            &files,
            A::Delete {
                path: renamed.clone()
            }
        )
        .await
        .is_err()
    );
    assert!(fixture.files.lock().unwrap().contains_key(&renamed));
    fixture.posix.store(false, Ordering::SeqCst);
    let again = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&again, |s| s.connected && !s.path.is_empty()).await;
    let files = again.files();
    let baseline = operation(
        &files,
        A::Download {
            path: renamed.clone(),
            local: local.clone(),
            edit: true,
        },
    )
    .await
    .unwrap()
    .fingerprint
    .unwrap();
    assert!(
        operation(
            &files,
            A::Replace {
                expected_local: None,
                path: renamed.clone(),
                local: local.clone(),
                expected: baseline
            }
        )
        .await
        .unwrap_err()
        .contains("posix-rename")
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        files
            .execute(
                A::Delete {
                    path: renamed.clone()
                },
                cancelled
            )
            .await
            .is_err()
    );
    let slow = fixture.files.lock().unwrap()[&renamed].clone();
    fixture
        .files
        .lock()
        .unwrap()
        .insert("/fixture/slow.bin".into(), slow);
    let cancel = CancellationToken::new();
    let worker = files.clone();
    let signal = cancel.clone();
    let destination = dir.path().join("slow-download");
    let downloading = tokio::spawn(async move {
        worker
            .execute(
                A::Download {
                    path: "/fixture/slow.bin".into(),
                    local: destination,
                    edit: false,
                },
                signal,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if fixture.files.lock().unwrap()["/fixture/slow.bin"]
                .attrs
                .mtime
                == Some(1700000999)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), downloading)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    wait(&again, |s| s.files_status.contains("unavailable")).await;
    assert!(again.snapshot().connected);
    again
        .send(Command::Input(b"after-file-cancel".to_vec()))
        .unwrap();
    wait(&again, |s| text(s).contains("after-file-cancel")).await;
    again.disconnect();
    wait(&again, |s| s.finished).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn raw_stream_preserves_vt_bytes_input_and_cancels_with_full_output_queue() {
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let path = dir.path().join("ephemeral-stream-key");
    key.write_openssh_file(&path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Stream fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = path.to_string_lossy().into();
    let wake = Arc::new(tokio::sync::Notify::new());
    let signal = wake.clone();
    let (session, mut output) = engine
        .connect_streaming(profile, Arc::new(move || signal.notify_one()))
        .unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.connected && s.files.len() == 2).await;
    let ready = tokio::time::timeout(Duration::from_secs(3), output.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(ready.windows(5).any(|w| w == b"Ready"));
    assert!(
        ready.contains(&0x1b),
        "VT escapes must reach the frontend unchanged"
    );
    // Remove the coalesced startup notification, then wait for actual new bytes.
    tokio::time::timeout(Duration::from_secs(1), wake.notified())
        .await
        .unwrap();
    let changed = wake.notified();
    session
        .send(Command::Input(b"stream-order\r".to_vec()))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), changed)
        .await
        .expect("Streaming SSH output did not wake its renderer");
    let echoed = tokio::time::timeout(Duration::from_secs(3), output.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, b"stream-order\r");
    // Stop consuming: a stalled renderer must have a bounded queue and remain cancellable.
    for _ in 0..100 {
        loop {
            if session.send(Command::Input(vec![b'x'; 1024])).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if output.len() == output.max_capacity() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(output.len(), output.max_capacity());
    session.disconnect();
    wait(&session, |s| s.finished).await;
    assert!(!session.snapshot().connected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ssh_trust_auth_terminal_resize_sftp_reconnect_and_changed_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let path = dir.path().join("ephemeral-test-key");
    key.write_openssh_file(&path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Loopback fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = path.to_string_lossy().into();
    let session = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(false);
    let rejected = wait(&session, |s| s.finished).await;
    assert!(!rejected.connected);
    assert_eq!(fixture.auth.load(Ordering::SeqCst), 0);
    assert!(!dir.path().join("hosts.json").exists());
    let cancelled = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    wait(&cancelled, |s| s.trust.is_some()).await;
    cancelled.disconnect();
    wait(&cancelled, |s| s.finished).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.connections.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Cancelling host verification must close transport");
    let session = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    assert_eq!(
        fixture.auth.load(Ordering::SeqCst),
        0,
        "No authentication before host approval"
    );
    session.trust(true);
    assert!(
        session
            .send(Command::Input(vec![b'x'; 16 * 1024 + 1]))
            .is_err()
    );
    assert!(session.send(Command::Browse("bad\0path".into())).is_err());
    let snapshot = wait(&session, |s| {
        s.connected && s.files.len() == 2 && text(s).contains("Ready")
    })
    .await;
    assert_eq!(snapshot.path, "/fixture");
    assert!(snapshot.files[0].directory);
    assert_eq!(snapshot.files[1].name, "read me.txt");
    session.send(Command::Input(b"hello\r".to_vec())).unwrap();
    wait(&session, |s| text(s).contains("hello")).await;
    session
        .send(Command::Resize {
            rows: 30,
            cols: 100,
        })
        .unwrap();
    wait(&session, |s| {
        s.terminal.rows == 30 && s.terminal.cols == 100
    })
    .await;
    assert!(fixture.resized.load(Ordering::SeqCst));
    session
        .send(Command::Browse("/does-not-exist".into()))
        .unwrap();
    let failed = wait(&session, |s| s.files_status.contains("Could not list")).await;
    assert_eq!(failed.path, "/fixture");
    assert_eq!(failed.files.len(), 2);
    session.disconnect();
    wait(&session, |s| s.finished).await;
    let again = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    assert_ne!(again.id, session.id);
    wait(&again, |s| s.connected).await; // saved host trust, no prompt
    again.disconnect();
    wait(&again, |s| s.finished).await;
    fixture.rotate.store(true, Ordering::SeqCst);
    let count = fixture.auth.load(Ordering::SeqCst);
    let changed = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    let blocked = wait(&changed, |s| s.finished).await;
    assert!(
        blocked.status.contains("HOST KEY CHANGED"),
        "{}",
        blocked.status
    );
    assert_eq!(fixture.auth.load(Ordering::SeqCst), count);
    assert!(blocked.trust.is_none());
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.connections.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Disconnect must close fixture connections");
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires the user's unlocked Windows OpenSSH agent"]
async fn windows_agent_signs_without_exporting_private_key() {
    let mut agent =
        russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
            .await
            .unwrap();
    let key = agent
        .request_identities()
        .await
        .unwrap()
        .into_iter()
        .find_map(|k| match k {
            russh::keys::agent::AgentIdentity::PublicKey { key, .. } => Some(key),
            _ => None,
        })
        .expect("No agent identities");
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let fixture = Fixture::start(key).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Agent loopback fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.identity_fingerprint = fingerprint;
    let session = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.connected && s.files.len() == 2).await;
    assert_eq!(fixture.auth.load(Ordering::SeqCst), 1);
    session.disconnect();
    wait(&session, |s| s.finished).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_key_file_authenticates_with_sha2() {
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Rsa { hash: None }).unwrap();
    let path = dir.path().join("ephemeral-rsa-key");
    key.write_openssh_file(&path, LineEnding::LF).unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "RSA loopback fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = path.to_string_lossy().into();
    let session = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.connected && s.files.len() == 2).await;
    assert_eq!(fixture.auth.load(Ordering::SeqCst), 1);
    session.disconnect();
    wait(&session, |s| s.finished).await;
}

#[tokio::test]
async fn sftp_symlink_open_uses_target_without_replacing_link() {
    use tokio_util::sync::CancellationToken;
    use yuuterm_ssh::files::FileAction as A;
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("identity");
    std::fs::write(
        &key_path,
        key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    {
        let mut files = fixture.files.lock().unwrap();
        let mut link = files["/fixture/read me.txt"].clone();
        link.attrs.permissions = Some(0o120777);
        link.data = b"/fixture/read me.txt".to_vec();
        files.insert("/fixture/file-link".into(), link);
    }
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Symlink fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = key_path.to_string_lossy().into();
    let session = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.files.len() == 3).await;
    let local = dir.path().join("link-download.txt");
    let downloaded = session
        .files()
        .execute(
            A::Download {
                path: "/fixture/file-link".into(),
                local: local.clone(),
                edit: true,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(&local).unwrap(),
        fixture.files.lock().unwrap()["/fixture/read me.txt"].data
    );
    assert!(
        fixture.files.lock().unwrap()["/fixture/file-link"]
            .attrs
            .is_symlink()
    );
    support::add_links(&fixture.files);
    let client = session.files();
    let resolved = client
        .execute(
            A::Resolve {
                path: "/fixture/file-link".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .resolved
        .unwrap();
    assert_eq!(resolved.path, "/fixture/read me.txt");
    assert!(!resolved.directory);
    let directory = client
        .execute(
            A::Resolve {
                path: "/fixture/dir-link".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .resolved
        .unwrap();
    assert!(directory.directory);
    assert_eq!(directory.path, "/fixture/logs");
    for path in ["/fixture/broken-link", "/fixture/cycle-link"] {
        assert!(
            client
                .execute(A::Resolve { path: path.into() }, CancellationToken::new())
                .await
                .is_err()
        );
    }
    let original = fixture.files.lock().unwrap()["/fixture/read me.txt"]
        .data
        .clone();
    client
        .execute(
            A::Create {
                path: "/fixture/new file.txt".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    {
        let files = fixture.files.lock().unwrap();
        assert!(files["/fixture/new file.txt"].data.is_empty());
        assert_eq!(
            files["/fixture/new file.txt"].attrs.permissions.unwrap() & 0o777,
            0o600
        );
    }
    for path in [
        "/fixture/new file.txt",
        "/fixture/read me.txt",
        "/fixture/file-link",
        "/fixture/logs",
    ] {
        assert!(
            client
                .execute(A::Create { path: path.into() }, CancellationToken::new())
                .await
                .is_err(),
            "Create must not replace an existing destination"
        );
    }
    assert_eq!(
        fixture.files.lock().unwrap()["/fixture/read me.txt"].data,
        original
    );
    client
        .execute(
            A::Delete {
                path: "/fixture/new file.txt".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    wait(&session, |s| s.files_status == "6 entries").await;
    std::fs::write(&local, b"saved local edit").unwrap();
    let expected = downloaded.fingerprint.unwrap();
    let prior_status = session.snapshot().files_status.clone();
    let metadata = client
        .execute(
            A::Revision {
                path: resolved.path.clone(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .fingerprint
        .unwrap();
    assert_eq!(metadata, expected.rsplit_once(':').unwrap().0);
    assert_eq!(session.snapshot().files_status, prior_status);
    let verified = client
        .execute(
            A::Verify {
                path: resolved.path.clone(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .fingerprint
        .unwrap();
    assert_eq!(verified, expected);
    {
        let mut files = fixture.files.lock().unwrap();
        files.get_mut("/fixture/read me.txt").unwrap().data[0] ^= 1;
    }
    let changed = client
        .execute(
            A::Verify {
                path: resolved.path.clone(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
        .fingerprint
        .unwrap();
    assert_ne!(
        changed, expected,
        "Full verification must detect equal-size changes even when metadata is unchanged"
    );
    {
        let mut files = fixture.files.lock().unwrap();
        files.get_mut("/fixture/read me.txt").unwrap().data = original.clone();
    }
    assert!(
        client
            .execute(
                A::Revision {
                    path: "/fixture/file-link".into()
                },
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert!(
        client
            .execute(
                A::Replace {
                    path: resolved.path.clone(),
                    local: local.clone(),
                    expected: expected.clone(),
                    expected_local: Some("0".repeat(64))
                },
                CancellationToken::new()
            )
            .await
            .unwrap_err()
            .contains("Local file changed again")
    );
    assert_eq!(
        fixture.files.lock().unwrap()["/fixture/read me.txt"].data,
        original
    );
    let revision = yuuterm_ssh::files::local_revision(&local).await.unwrap();
    client
        .execute(
            A::Replace {
                path: resolved.path.clone(),
                local: local.clone(),
                expected,
                expected_local: Some(revision),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        fixture.files.lock().unwrap()["/fixture/read me.txt"].data,
        b"saved local edit"
    );
    assert!(
        fixture.files.lock().unwrap()["/fixture/file-link"]
            .attrs
            .is_symlink()
    );
    client
        .execute(
            A::Rename {
                path: "/fixture/file-link".into(),
                name: "renamed-link".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    client
        .execute(
            A::Delete {
                path: "/fixture/renamed-link".into(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(
        fixture
            .files
            .lock()
            .unwrap()
            .contains_key("/fixture/read me.txt")
    );
    session.disconnect();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_replacement_skips_redundant_chown_but_refuses_owner_or_mode_loss() {
    use tokio_util::sync::CancellationToken;
    use yuuterm_ssh::files::FileAction as A;
    let dir = tempfile::tempdir().unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let key_path = dir.path().join("identity");
    std::fs::write(
        &key_path,
        key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    let fixture = Fixture::start(key.public_key().clone()).await;
    {
        let mut data = fixture.files.lock().unwrap();
        let marker = data["/fixture/read me.txt"].clone();
        data.insert("/fixture/reject-chown".into(), marker);
    }
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let mut profile = Profile {
        id: 1,
        name: "Ownership fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.auth = SshAuth::KeyFile;
    profile.ssh.key_file = key_path.to_string_lossy().into();
    let session = engine.connect(profile, Arc::new(|| {})).unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    wait(&session, |s| s.connected && s.files.len() == 3).await;
    for attempt in 0..3 {
        {
            let mut data = fixture.files.lock().unwrap();
            let file = data.get_mut("/fixture/read me.txt").unwrap();
            file.attrs.uid = Some(if attempt == 1 { 2000 } else { 1000 });
            file.attrs.permissions = Some(if attempt == 2 { 0o100640 } else { 0o100600 });
            if attempt == 2 {
                let marker = file.clone();
                data.insert("/fixture/ignore-mode".into(), marker);
            }
        }
        let local = dir.path().join(format!("edit-{attempt}.txt"));
        let baseline = session
            .files()
            .execute(
                A::Download {
                    path: "/fixture/read me.txt".into(),
                    local: local.clone(),
                    edit: true,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap()
            .fingerprint
            .unwrap();
        let before = fixture.files.lock().unwrap()["/fixture/read me.txt"]
            .data
            .clone();
        std::fs::write(&local, format!("new saved content {attempt}")).unwrap();
        let result = session
            .files()
            .execute(
                A::Replace {
                    path: "/fixture/read me.txt".into(),
                    local,
                    expected: baseline,
                    expected_local: None,
                },
                CancellationToken::new(),
            )
            .await;
        if attempt == 0 {
            assert!(
                result.is_ok(),
                "Matching ownership must not issue rejected chown"
            );
            assert_eq!(
                fixture.files.lock().unwrap()["/fixture/read me.txt"].data,
                b"new saved content 0"
            );
        } else {
            assert!(result.unwrap_err().contains(if attempt == 1 {
                "Could not preserve"
            } else {
                "did not preserve"
            }));
            assert_eq!(
                fixture.files.lock().unwrap()["/fixture/read me.txt"].data,
                before
            );
        }
    }
    session.disconnect();
}
