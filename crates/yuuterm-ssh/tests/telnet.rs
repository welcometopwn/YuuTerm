use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use yuuterm_core::{Profile, Protocol};
use yuuterm_ssh::{Command, Engine};

#[tokio::test]
async fn refused_tcp_connect_publishes_a_final_reason_and_never_connected() {
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(dir.path().join("trust.json")).unwrap();
    let (live, mut output) = engine
        .connect_streaming(
            Profile {
                name: "Refused TELNET fixture".into(),
                protocol: Protocol::Telnet,
                host: "127.0.0.1".into(),
                port,
                ..Default::default()
            },
            Arc::new(|| {}),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !live.snapshot().finished {
            assert!(!live.snapshot().connected);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let final_state = live.snapshot();
    assert!(!final_state.connected);
    assert!(final_state.trust.is_none());
    assert!(
        final_state.status.contains("10061"),
        "{}",
        final_state.status
    );
    assert!(!final_state.status.contains("Connecting"));
    assert!(output.recv().await.is_none());
}

#[tokio::test]
async fn real_tcp_telnet_negotiation_input_resize_eof_and_no_ssh_dependencies() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.set_nodelay(true).unwrap();
        let negotiation = [
            255, 251, 1, 255, 251, 0, 255, 253, 0, 255, 253, 24, 255, 253, 31, 255, 250, 24, 1,
            255, 240,
        ];
        for byte in negotiation {
            socket.write_all(&[byte]).await.unwrap();
            tokio::task::yield_now().await;
        }
        socket
            .write_all(b"\x1b[32mReady\x1b[0m\r\nGr\xc3")
            .await
            .unwrap();
        socket.write_all(b"\xbc\xc3\x9fe\r\n").await.unwrap();
        let mut received = Vec::new();
        let mut chunk = [0u8; 256];
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                assert_ne!(n, 0);
                received.extend_from_slice(&chunk[..n]);
                if received.windows(8).any(|w| w == b"one\xff\xfftwo")
                    && received
                        .windows(9)
                        .any(|w| w == [255, 250, 31, 0, 240, 0, 100, 255, 240])
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(received.windows(14).any(|w| w == b"XTERM-256COLOR"));
        socket.write_all(&[255, 255, b'!']).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    // A malformed SSH trust store must not affect TELNET or trigger agent auth.
    let pins = dir.path().join("host-keys.json");
    std::fs::write(&pins, b"invalid").unwrap();
    let engine = Engine::new(pins).unwrap();
    let profile = Profile {
        name: "TELNET fixture".into(),
        protocol: Protocol::Telnet,
        host: "127.0.0.1".into(),
        port,
        ..Default::default()
    };
    let (live, mut output) = engine.connect_streaming(profile, Arc::new(|| {})).unwrap();
    let mut rendered = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !String::from_utf8_lossy(&rendered).contains("Grüße") {
            rendered.extend(output.recv().await.unwrap());
        }
    })
    .await
    .unwrap();
    assert!(live.snapshot().connected);
    assert!(live.snapshot().status.contains("Unencrypted"));
    assert!(live.snapshot().trust.is_none());
    assert!(live.snapshot().files.is_empty());
    live.send(Command::Resize {
        rows: 65535,
        cols: 65535,
    })
    .unwrap();
    live.send(Command::Input(b"one\xfftwo".to_vec())).unwrap();
    server.await.unwrap();
    while let Some(chunk) = output.recv().await {
        rendered.extend(chunk);
    }
    assert!(rendered.ends_with(&[255, b'!']));
    assert!(!rendered.windows(3).any(|w| w == [255, 251, 1]));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !live.snapshot().finished {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!live.snapshot().connected);
    assert!(live.send(Command::Input(b"stale".to_vec())).is_err());
}

#[tokio::test]
async fn disconnect_cancels_a_telnet_connection_with_a_full_renderer_queue() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let bytes = [b'x'; 16384];
        // A read half independently detects cancellation, even if the sender fills buffers.
        let (mut read, mut write) = stream.split();
        tokio::select! {
            result=async {let mut b=[0u8;16];loop {let n=read.read(&mut b).await.unwrap_or(0);if n==0 {break;}}}=>result,
            _=async {loop {if write.write_all(&bytes).await.is_err(){break;}}}=>{},
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(dir.path().join("pins.json")).unwrap();
    let (live, output) = engine
        .connect_streaming(
            Profile {
                name: "TELNET flood fixture".into(),
                protocol: Protocol::Telnet,
                host: "127.0.0.1".into(),
                port,
                ..Default::default()
            },
            Arc::new(|| {}),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while output.len() < 32 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    live.disconnect();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !live.snapshot().finished {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}
