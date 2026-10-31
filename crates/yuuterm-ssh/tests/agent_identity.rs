//! Generated named-pipe agent and SSH server; no user agent or private key files.
#![cfg(windows)]
#[path = "support/mod.rs"]
pub mod support;
use russh::keys::{
    Algorithm, PrivateKey,
    agent::{AgentIdentity, client::AgentClient},
    ssh_key::HashAlg,
};
use std::{sync::Arc, time::Duration};
use tokio::{net::windows::named_pipe::ServerOptions, sync::mpsc};
use yuuterm_core::Profile;
use yuuterm_ssh::Engine;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_authentication_still_uses_the_agent_pipe_without_a_client_private_key_file() {
    let pipe = format!(
        r"\\.\pipe\yuuterm-rsa-agent-fixture-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    );
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe)
        .unwrap();
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Rsa { hash: None }).unwrap();
    let public = key.public_key().clone();
    let blob = public.to_bytes().unwrap();
    let signs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = signs.clone();
    let server = tokio::spawn(async move {
        use russh::keys::signature::Signer;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        fn string(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend((bytes.len() as u32).to_be_bytes());
            out.extend(bytes);
        }
        fn take<'a>(bytes: &mut &'a [u8]) -> &'a [u8] {
            let n = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            let result = &bytes[4..4 + n];
            *bytes = &bytes[4 + n..];
            result
        }
        first.connect().await.unwrap();
        let mut pipe = first;
        while let Ok(n) = pipe.read_u32().await {
            assert!(n <= 65536);
            let mut bytes = vec![0; n as usize];
            pipe.read_exact(&mut bytes).await.unwrap();
            let mut response = Vec::new();
            match bytes[0] {
                11 => {
                    response.push(12);
                    response.extend(1u32.to_be_bytes());
                    string(&mut response, &blob);
                    string(&mut response, b"generated RSA fixture");
                }
                13 => {
                    let mut fields = &bytes[1..];
                    assert_eq!(take(&mut fields), blob);
                    let data = take(&mut fields);
                    let flags = u32::from_be_bytes(fields.try_into().unwrap());
                    let (hash, label) = match flags {
                        2 => (HashAlg::Sha256, b"rsa-sha2-256".as_slice()),
                        4 => (HashAlg::Sha512, b"rsa-sha2-512".as_slice()),
                        _ => panic!("RSA-SHA2 required"),
                    };
                    let russh::keys::ssh_key::private::KeypairData::Rsa(pair) = key.key_data()
                    else {
                        unreachable!()
                    };
                    let signature = (pair, Some(hash)).try_sign(data).unwrap();
                    let mut signed = Vec::new();
                    string(&mut signed, label);
                    string(&mut signed, signature.as_bytes());
                    response.push(14);
                    string(&mut response, &signed);
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                _ => panic!("Unexpected fixture agent request"),
            }
            pipe.write_u32(response.len() as u32).await.unwrap();
            pipe.write_all(&response).await.unwrap();
        }
    });
    let fixture = support::Fixture::start(public.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let mut profile = Profile {
        name: "RSA agent fixture".into(),
        host: "127.0.0.1".into(),
        port: fixture.port,
        username: "fixture".into(),
        ..Default::default()
    };
    profile.ssh.agent_pipe = pipe;
    profile.ssh.identity_fingerprint = public.fingerprint(HashAlg::Sha256).to_string();
    assert!(profile.ssh.key_file.is_empty());
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let live = engine.connect(profile, Arc::new(|| {})).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let s = live.snapshot();
            if s.trust.is_some() {
                live.trust(true);
            }
            if s.connected && s.files.len() >= 2 {
                break;
            }
            assert!(!s.finished, "{}", s.status);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.auth.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(fixture.offers.lock().unwrap().len(), 1);
    live.disconnect();
    assert_eq!(signs.load(std::sync::atomic::Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_seventh_agent_identity_is_the_only_key_offered_and_missing_pub_fails_closed() {
    let pipe = format!(
        r"\\.\pipe\yuuterm-agent-fixture-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    );
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe)
        .unwrap();
    let (send, receive) = mpsc::channel(8);
    let server = tokio::spawn(russh::keys::agent::server::serve(
        tokio_stream::wrappers::ReceiverStream::new(receive),
        (),
    ));
    let name = pipe.clone();
    let listener = tokio::spawn(async move {
        let mut next = first;
        loop {
            next.connect().await.unwrap();
            let ready = next;
            next = ServerOptions::new().create(&name).unwrap();
            if send.send(Ok(ready)).await.is_err() {
                break;
            }
        }
    });
    let mut agent = AgentClient::connect_named_pipe(&pipe).await.unwrap();
    for _ in 0..7 {
        agent
            .add_identity(
                &PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap(),
                &[],
            )
            .await
            .unwrap();
    }
    let keys = agent.request_identities().await.unwrap();
    assert_eq!(keys.len(), 7);
    let chosen = match keys.last().unwrap() {
        AgentIdentity::PublicKey { key, .. } => key.clone(),
        _ => panic!("Generated fixture keys are plain identities"),
    };
    let fingerprint = chosen.fingerprint(HashAlg::Sha256).to_string();
    let fixture = support::Fixture::start(chosen.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let pubkey = dir.path().join("chosen.pub");
    std::fs::write(&pubkey, chosen.to_openssh().unwrap()).unwrap();
    let config = dir.path().join("config");
    std::fs::write(&config,format!("Host *\n IdentitiesOnly yes\nHost selected-fixture\n HostName 127.0.0.1\n User fixture\n Port {}\n IdentityFile \"{}\"\n",fixture.port,pubkey.display())).unwrap();
    let mut profile = Profile {
        name: "Identity policy fixture".into(),
        host: "127.0.0.1".into(),
        username: "fixture".into(),
        port: fixture.port,
        ..Default::default()
    };
    profile.ssh.agent_pipe = pipe;
    profile.ssh.config_file = config.to_string_lossy().into_owned();
    let engine = Engine::new(dir.path().join("hosts.json")).unwrap();
    let session = engine.connect(profile.clone(), Arc::new(|| {})).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let s = session.snapshot();
            if s.trust.is_some() {
                session.trust(true);
            }
            if s.connected {
                break;
            }
            assert!(!s.finished, "{}", s.status);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(*fixture.offers.lock().unwrap(), vec![fingerprint.clone()]);
    assert_eq!(fixture.auth.load(std::sync::atomic::Ordering::SeqCst), 1);
    session.disconnect();
    std::fs::remove_file(pubkey).unwrap();
    fixture.offers.lock().unwrap().clear();
    let rejected = engine.connect(profile, Arc::new(|| {})).unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let s = rejected.snapshot();
            if s.finished {
                break s.status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(status.contains("IdentitiesOnly=yes"), "{status}");
    assert!(
        fixture.offers.lock().unwrap().is_empty(),
        "No other agent key may be offered after public-key lookup fails"
    );
    listener.abort();
    server.abort();
}
