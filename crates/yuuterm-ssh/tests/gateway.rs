//! SSH gateway chains over loopback: every hop is a virtual
//! fixture server; the gateway bridges `direct-tcpip` to the next fixture.
pub mod support;
use russh::keys::{Algorithm, PrivateKey, ssh_key::LineEnding};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::Fixture;
use yuuterm_core::{ManualGateway, Profile, SshAuth};
use yuuterm_ssh::{Command, Engine, Hop, LiveSession, Snapshot};

struct Lab {
    _dir: tempfile::TempDir,
    key: PathBuf,
    engine: Engine,
    gateway: Fixture,
    second: Fixture,
    target: Fixture,
}

async fn lab() -> Lab {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gateway-tests");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let path = dir.path().join("fixture-key");
    key.write_openssh_file(&path, LineEnding::LF).unwrap();
    Lab {
        engine: Engine::new(dir.path().join("hosts.json")).unwrap(),
        gateway: Fixture::start(key.public_key().clone()).await,
        second: Fixture::start(key.public_key().clone()).await,
        target: Fixture::start(key.public_key().clone()).await,
        key: path,
        _dir: dir,
    }
}

impl Lab {
    fn hop(&self, fixture: &Fixture) -> Hop {
        Hop {
            gateway: ManualGateway {
                host: "127.0.0.1".into(),
                port: fixture.port,
                username: "fixture".into(),
                auth: SshAuth::KeyFile,
                key_file: self.key.to_string_lossy().into(),
                ..Default::default()
            },
            password: None,
        }
    }
    fn profile(&self) -> Profile {
        let mut p = Profile {
            id: 1,
            name: "Target".into(),
            host: "127.0.0.1".into(),
            port: self.target.port,
            username: "fixture".into(),
            ..Default::default()
        };
        p.ssh.auth = SshAuth::KeyFile;
        p.ssh.key_file = self.key.to_string_lossy().into();
        p
    }
    /// Keep the output receiver: a dropped renderer ends the session.
    fn connect(&self, hops: Vec<Hop>) -> (LiveSession, tokio::sync::mpsc::Receiver<Vec<u8>>) {
        self.engine
            .connect_streaming_via(self.profile(), hops, Arc::new(|| {}), None)
            .unwrap()
    }
}

async fn wait(session: &LiveSession, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = session.snapshot();
            if predicate(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out: {}", session.snapshot().status))
}

/// Approve every host-key prompt until connected; returns the roles seen.
async fn approve_all(session: &LiveSession) -> Vec<String> {
    let mut roles = Vec::new();
    loop {
        let s = wait(session, |s| s.trust.is_some() || s.connected || s.finished).await;
        if let Some(trust) = s.trust {
            roles.push(trust.role.clone());
            session.trust(true);
            wait(session, |s| s.trust.is_none()).await;
        } else {
            assert!(s.connected, "not connected: {}", s.status);
            return roles;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_hop_terminal_and_sftp() {
    let lab = lab().await;
    let (session, _session_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    approve_all(&session).await;
    wait(&session, |s| !s.files.is_empty()).await;
    session
        .send(Command::Input(b"echo-through-gateway\r".to_vec()))
        .unwrap();
    assert_eq!(
        *lab.gateway.direct_tcpip.lock().unwrap(),
        vec![("127.0.0.1".to_string(), u32::from(lab.target.port))]
    );
    assert_eq!(lab.target.auth.load(Ordering::SeqCst), 1);
    assert_eq!(lab.gateway.auth.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_hops_reach_target() {
    let lab = lab().await;
    let (session, _session_out) = lab.connect(vec![lab.hop(&lab.gateway), lab.hop(&lab.second)]);
    let roles = approve_all(&session).await;
    assert_eq!(roles, ["Gateway 1 of 2", "Gateway 2 of 2", "Target"]);
    assert_eq!(
        *lab.gateway.direct_tcpip.lock().unwrap(),
        vec![("127.0.0.1".to_string(), u32::from(lab.second.port))]
    );
    assert_eq!(
        *lab.second.direct_tcpip.lock().unwrap(),
        vec![("127.0.0.1".to_string(), u32::from(lab.target.port))]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trust_prompt_per_hop_with_role() {
    let lab = lab().await;
    let (session, _session_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    let first = wait(&session, |s| s.trust.is_some()).await.trust.unwrap();
    assert_eq!(first.role, "Gateway 1 of 1");
    assert_eq!(first.host, format!("[127.0.0.1]:{}", lab.gateway.port));
    session.trust(true);
    let second = wait(&session, |s| {
        s.trust.as_ref().is_some_and(|t| t.role == "Target")
    })
    .await
    .trust
    .unwrap();
    assert_eq!(second.host, format!("[127.0.0.1]:{}", lab.target.port));
    let direct = lab.engine.connect(lab.profile(), Arc::new(|| {})).unwrap();
    session.trust(true);
    wait(&session, |s| s.connected).await;
    let role = wait(&direct, |s| s.trust.is_some() || s.connected).await;
    assert!(role.connected, "direct connection shares target trust");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_gateway_key_blocks() {
    let lab = lab().await;
    let (session, _session_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    approve_all(&session).await;
    session.disconnect();
    lab.gateway.rotate.store(true, Ordering::SeqCst);
    let (again, _again_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    let end = wait(&again, |s| s.finished).await;
    assert!(
        end.status
            .starts_with(&format!("Gateway 1 (127.0.0.1:{})", lab.gateway.port)),
        "{}",
        end.status
    );
    assert!(end.status.contains("HOST KEY CHANGED"), "{}", end.status);
    assert_eq!(
        lab.target.auth.load(Ordering::SeqCst),
        1,
        "target never reached again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_gateway_password_is_attributed() {
    let lab = lab().await;
    let mut hop = lab.hop(&lab.gateway);
    hop.gateway.auth = SshAuth::Password;
    hop.password = Some(zeroize::Zeroizing::new("wrong".into()));
    let (session, _session_out) = lab.connect(vec![hop]);
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    let end = wait(&session, |s| s.finished).await;
    assert!(end.password_rejected);
    assert_eq!(end.rejected_gateway, Some(0));
    assert!(end.status.starts_with("Gateway 1 ("), "{}", end.status);
    assert!(!end.status.contains("wrong"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_tunnel_names_target() {
    let lab = lab().await;
    lab.gateway.allow_forwarding.store(false, Ordering::SeqCst);
    let (session, _session_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    wait(&session, |s| s.trust.is_some()).await;
    session.trust(true);
    let end = wait(&session, |s| s.finished).await;
    assert!(
        end.status.contains(&format!(
            "refused a tunnel to 127.0.0.1:{}",
            lab.target.port
        )),
        "{}",
        end.status
    );
    assert!(!end.password_rejected);
    assert_eq!(end.rejected_gateway, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_gateway_trust_prompt_closes_chain() {
    let lab = lab().await;
    // The non-streaming variant keeps its own headless terminal.
    let session = lab
        .engine
        .connect_via(
            lab.profile(),
            vec![lab.hop(&lab.gateway)],
            Arc::new(|| {}),
            None,
        )
        .unwrap();
    wait(&session, |s| s.trust.is_some()).await;
    session.disconnect();
    wait(&session, |s| s.finished).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(lab.gateway.direct_tcpip.lock().unwrap().is_empty());
    assert_eq!(lab.target.auth.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_builds_new_chain() {
    let lab = lab().await;
    let (first, _first_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    approve_all(&first).await;
    first.disconnect();
    wait(&first, |s| s.finished).await;
    let (second, _second_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    approve_all(&second).await;
    assert_ne!(first.id, second.id);
    assert_eq!(lab.gateway.direct_tcpip.lock().unwrap().len(), 2);
    assert_eq!(lab.gateway.auth.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_names_gateway_stage() {
    let lab = lab().await;
    let (session, _session_out) = lab.connect(vec![lab.hop(&lab.gateway)]);
    let mut seen = Vec::new();
    let mut updates = session.subscribe_snapshot();
    let observe = async {
        loop {
            let s = updates.borrow_and_update().clone();
            seen.push(s.status.clone());
            if s.trust.is_some() {
                session.trust(true);
            }
            if s.connected || s.finished {
                break;
            }
            if updates.changed().await.is_err() {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), observe)
        .await
        .unwrap();
    assert!(
        seen.iter().any(|s| s == "Connecting to gateway 1 of 1…"),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|s| s == "Authenticating at gateway 1…"),
        "{seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_forward_preserves_bytes_across_multiple_connections_and_cancels() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let lab = lab().await;
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut profile = lab.profile();
    profile.protocol = yuuterm_core::Protocol::Rdp;
    profile.port = target.local_addr().unwrap().port();
    let (session, address) = lab
        .engine
        .forward_via(
            profile,
            vec![lab.hop(&lab.gateway), lab.hop(&lab.second)],
            Arc::new(|| {}),
        )
        .unwrap();
    assert!(address.ip().is_loopback());
    assert_eq!(
        approve_all(&session).await,
        ["Gateway 1 of 2", "Gateway 2 of 2"]
    );
    for _ in 0..2 {
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (mut server, _) = target.accept().await.unwrap();
        let expected: Vec<u8> = (0..100_000).map(|n| (n % 256) as u8).collect();
        let bytes = expected.clone();
        let writer = tokio::spawn(async move {
            server.write_all(&bytes).await.unwrap();
        });
        let mut actual = vec![0; expected.len()];
        tokio::time::timeout(Duration::from_secs(5), client.read_exact(&mut actual))
            .await
            .unwrap()
            .unwrap();
        writer.await.unwrap();
        assert_eq!(actual, expected);
    }
    session.disconnect();
    wait(&session, |s| s.finished).await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn telnet_waits_for_gateway_trust_then_negotiates_real_bytes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let lab = lab().await;
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut profile = lab.profile();
    profile.protocol = yuuterm_core::Protocol::Telnet;
    profile.port = target.local_addr().unwrap().port();
    let (session, mut output) = lab
        .engine
        .connect_streaming_via(profile, vec![lab.hop(&lab.gateway)], Arc::new(|| {}), None)
        .unwrap();
    let prompt = wait(&session, |s| s.trust.is_some()).await;
    assert!(
        !prompt.connected,
        "A loopback socket must not claim target connectivity before gateway trust"
    );
    session.trust(true);
    let (mut peer, _) = target.accept().await.unwrap();
    peer.write_all(b"\xff\xfb\x01gateway-telnet\r\n")
        .await
        .unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        while !bytes.windows(14).any(|s| s == b"gateway-telnet") {
            bytes.extend(output.recv().await.unwrap());
        }
        bytes
    })
    .await
    .unwrap();
    assert!(bytes.windows(14).any(|s| s == b"gateway-telnet"));
    let mut negotiation = [0; 3];
    peer.read_exact(&mut negotiation).await.unwrap();
    assert_eq!(negotiation, [255, 253, 1]);
    session.disconnect();
    wait(&session, |s| s.finished).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forward_reconnect_changed_key_and_pending_cancellation() {
    let lab = lab().await;
    let mut p = lab.profile();
    p.protocol = yuuterm_core::Protocol::Vnc;
    let create = || {
        lab.engine
            .forward_via(p.clone(), vec![lab.hop(&lab.gateway)], Arc::new(|| {}))
            .unwrap()
    };
    let (first, _) = create();
    assert_eq!(approve_all(&first).await, ["Gateway 1 of 1"]);
    first.disconnect();
    wait(&first, |s| s.finished).await;
    let (second, _) = create();
    approve_all(&second).await;
    assert_ne!(first.id, second.id);
    second.disconnect();
    wait(&second, |s| s.finished).await;
    lab.gateway.rotate.store(true, Ordering::SeqCst);
    let (changed, address) = create();
    let refused = wait(&changed, |s| s.finished).await;
    assert!(!refused.connected);
    assert!(refused.trust.is_none());
    assert!(
        refused.status.contains("HOST KEY CHANGED"),
        "{}",
        refused.status
    );
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    let fresh_lab = self::lab().await;
    let mut p = fresh_lab.profile();
    p.protocol = yuuterm_core::Protocol::Rdp;
    let (pending, address) = fresh_lab
        .engine
        .forward_via(p, vec![fresh_lab.hop(&fresh_lab.gateway)], Arc::new(|| {}))
        .unwrap();
    wait(&pending, |s| s.trust.is_some()).await;
    pending.disconnect();
    wait(&pending, |s| s.finished).await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(fresh_lab.gateway.direct_tcpip.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_forward_fails_with_gateway_and_target_context() {
    let lab = lab().await;
    lab.gateway.allow_forwarding.store(false, Ordering::SeqCst);
    let mut p = lab.profile();
    p.protocol = yuuterm_core::Protocol::Rdp;
    let (session, address) = lab
        .engine
        .forward_via(p, vec![lab.hop(&lab.gateway)], Arc::new(|| {}))
        .unwrap();
    approve_all(&session).await;
    let _socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let end = wait(&session, |s| s.finished).await;
    assert!(end.status.contains("Gateway 1"));
    assert!(
        end.status
            .contains(&format!("127.0.0.1:{}", lab.target.port))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forward_limits_concurrent_channels_and_cancels_active_sockets() {
    use tokio::io::AsyncReadExt;
    let lab = lab().await;
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut profile = lab.profile();
    profile.protocol = yuuterm_core::Protocol::Vnc;
    profile.port = target.local_addr().unwrap().port();
    let (session, address) = lab
        .engine
        .forward_via(profile, vec![lab.hop(&lab.gateway)], Arc::new(|| {}))
        .unwrap();
    approve_all(&session).await;
    let mut clients = Vec::new();
    let mut peers = Vec::new();
    for _ in 0..4 {
        clients.push(tokio::net::TcpStream::connect(address).await.unwrap());
        peers.push(
            tokio::time::timeout(Duration::from_secs(2), target.accept())
                .await
                .unwrap()
                .unwrap()
                .0,
        );
    }
    let mut fifth = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), fifth.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    session.disconnect();
    wait(&session, |s| s.finished).await;
    for mut client in clients {
        let closed = tokio::time::timeout(Duration::from_secs(2), client.read(&mut byte))
            .await
            .unwrap();
        assert!(closed.is_err() || closed.unwrap() == 0);
    }
    drop(peers);
}
