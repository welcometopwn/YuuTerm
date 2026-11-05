use super::*;
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
mod features;

struct Fixture {
    certificate: CertificateDer<'static>,
    server: Arc<rustls::ServerConfig>,
}
impl Fixture {
    fn new(name: &str, expired: bool) -> Self {
        Self::new_version(name, expired, &rustls::version::TLS13)
    }
    fn new_version(
        name: &str,
        expired: bool,
        version: &'static rustls::SupportedProtocolVersion,
    ) -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![name.into()]).unwrap();
        if expired {
            params.not_after = rcgen::date_time_ymd(2000, 1, 1);
        }
        let cert = params.self_signed(&key).unwrap();
        Self::from_der_version(cert.der().clone(), key, version)
    }
    fn from_der(certificate: CertificateDer<'static>, key: KeyPair) -> Self {
        Self::from_der_version(certificate, key, &rustls::version::TLS13)
    }
    fn from_der_version(
        certificate: CertificateDer<'static>,
        key: KeyPair,
        version: &'static rustls::SupportedProtocolVersion,
    ) -> Self {
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[version])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
        Self {
            certificate,
            server: Arc::new(server),
        }
    }
    fn options(&self, port: u16, auth: TlsAuth, trusted: bool) -> Options {
        let mut roots = rustls::RootCertStore::empty();
        if trusted {
            roots.add(self.certificate.clone()).unwrap();
        }
        Options {
            host: "127.0.0.1".into(),
            port,
            auth,
            roots,
        }
    }
    fn review(&self, port: u16) -> CertificateReview {
        CertificateReview {
            endpoint: format!("[127.0.0.1]:{port}"),
            sha256: trust::fingerprint(&self.certificate),
            certificate_der: self.certificate.to_vec(),
        }
    }
}
fn credentials(auth: TlsAuth) -> Credentials {
    Credentials {
        username: Zeroizing::new(if matches!(auth, TlsAuth::Plain) {
            "fixture".into()
        } else {
            String::new()
        }),
        password: Zeroizing::new("password".into()),
    }
}
async fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
async fn store(dir: &tempfile::TempDir) -> VncTrust {
    VncTrust::load(dir.path().join("vnc-pins.json"))
        .await
        .unwrap()
}
async fn event(handle: &mut Handle) -> Event {
    tokio::time::timeout(Duration::from_secs(5), handle.events.recv())
        .await
        .expect("VNC event deadline")
        .expect("VNC event channel closed")
}

async fn tls_peer(
    listener: &TcpListener,
    config: Arc<rustls::ServerConfig>,
    auth: TlsAuth,
) -> Result<tokio_rustls::server::TlsStream<TcpStream>> {
    let (mut s, _) = listener.accept().await?;
    for part in b"RFB 003.008\n".chunks(3) {
        s.write_all(part).await?;
        tokio::task::yield_now().await;
    }
    let mut banner = [0; 12];
    s.read_exact(&mut banner).await?;
    assert_eq!(&banner, b"RFB 003.008\n");
    s.write_all(&[2, 1, 19]).await?;
    assert_eq!(s.read_u8().await?, 19);
    s.write_all(&[0, 2]).await?;
    assert_eq!(s.read_u16().await?, 2);
    s.write_all(&[0, 2]).await?;
    s.write_u32(261).await?;
    s.write_u32(262).await?;
    assert_eq!(
        s.read_u32().await?,
        if matches!(auth, TlsAuth::Plain) {
            262
        } else {
            261
        }
    );
    s.write_u8(1).await?;
    s.flush().await?;
    Ok(TlsAcceptor::from(config).accept(s).await?)
}
async fn authentication(
    s: &mut (impl AsyncRead + AsyncWrite + Unpin),
    auth: TlsAuth,
    success: bool,
) -> Result<()> {
    match auth {
        TlsAuth::Plain => {
            assert_eq!(s.read_u32().await?, 7);
            assert_eq!(s.read_u32().await?, 8);
            let mut bytes = [0; 15];
            s.read_exact(&mut bytes).await?;
            assert_eq!(&bytes, b"fixturepassword");
        }
        TlsAuth::Vnc => {
            s.write_all(&(0u8..16).collect::<Vec<_>>()).await?;
            s.flush().await?;
            let mut response = [0; 16];
            s.read_exact(&mut response).await?;
            // Independent OpenSSL DES-ECB vector, bit-reversed "password" key.
            assert_eq!(
                response,
                [
                    0xb8, 0x66, 0x92, 0x41, 0x25, 0xc8, 0xee, 0xbb, 0x9d, 0xeb, 0xc1, 0xdb, 0x61,
                    0xc5, 0x38, 0xe2
                ]
            );
        }
    }
    s.write_u32(if success { 0 } else { 1 }).await?;
    s.flush().await?;
    Ok(())
}
async fn init(s: &mut (impl AsyncRead + AsyncWrite + Unpin)) -> Result<()> {
    assert_eq!(s.read_u8().await?, 1);
    s.write_u16(3).await?;
    s.write_u16(2).await?;
    s.write_all(&PIXEL_FORMAT).await?;
    s.write_u32(7).await?;
    s.write_all(b"fixture").await?;
    s.flush().await?;
    let mut set_format = [0; 20];
    s.read_exact(&mut set_format).await?;
    assert_eq!(&set_format[..4], &[0, 0, 0, 0]);
    assert_eq!(&set_format[4..], &PIXEL_FORMAT);
    let mut encodings = [0; 28];
    s.read_exact(&mut encodings).await?;
    assert_eq!(
        encodings,
        [
            2, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, 16, 0, 0, 0, 1, 0, 0, 0, 0, 255, 255, 254, 204, 255,
            255, 255, 17
        ]
    );
    Ok(())
}
async fn request(s: &mut (impl AsyncRead + Unpin), incremental: bool) -> Result<()> {
    let mut request = [0; 10];
    s.read_exact(&mut request).await?;
    assert_eq!(request, [3, u8::from(incremental), 0, 0, 0, 0, 0, 3, 0, 2]);
    Ok(())
}
fn rect(wire: &mut Vec<u8>, xywh: [u16; 4], encoding: i32) {
    for value in xywh {
        wire.extend_from_slice(&value.to_be_bytes());
    }
    wire.extend_from_slice(&encoding.to_be_bytes());
}
fn raw_update() -> Vec<u8> {
    let mut wire = vec![0, 0, 0, 1];
    rect(&mut wire, [0, 0, 3, 2], 0);
    for pixel in 1u8..=6 {
        wire.extend_from_slice(&[pixel, pixel + 10, pixel + 20, 0]);
    }
    wire
}
async fn no_application_bytes(listener: TcpListener, config: Arc<rustls::ServerConfig>) {
    if let Ok(mut tls) = tls_peer(&listener, config, TlsAuth::Plain).await {
        let mut byte = [0];
        let result = tokio::time::timeout(Duration::from_secs(2), tls.read(&mut byte))
            .await
            .unwrap();
        assert!(
            !matches!(result, Ok(n) if n > 0),
            "Credentials sent before trust validation"
        );
    }
}

#[tokio::test]
async fn unknown_requires_explicit_pin_retry_persists_and_changed_certificate_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let first = Fixture::new("127.0.0.1", false);
    let l = listener().await;
    let port = l.local_addr().unwrap().port();
    let server = first.server.clone();
    // Keep the same host:port across all three connection incarnations.
    let (go, wait) = tokio::sync::oneshot::channel();
    let (go2, wait2) = tokio::sync::oneshot::channel();
    let changed = Fixture::new("127.0.0.1", false);
    let changed_server = changed.server.clone();
    let peer = tokio::spawn(async move {
        if let Ok(mut s) = tls_peer(&l, server.clone(), TlsAuth::Plain).await {
            assert!(s.read_u8().await.is_err());
        }
        wait.await.unwrap();
        let mut s = tls_peer(&l, server, TlsAuth::Plain).await.unwrap();
        authentication(&mut s, TlsAuth::Plain, true).await.unwrap();
        init(&mut s).await.unwrap();
        request(&mut s, false).await.unwrap();
        s.write_all(&raw_update()).await.unwrap();
        s.flush().await.unwrap();
        wait2.await.unwrap();
        drop(s);
        no_application_bytes(l, changed_server).await;
    });
    let mut handle = start(
        first.options(port, TlsAuth::Plain, false),
        credentials(TlsAuth::Plain),
        trust.clone(),
    )
    .unwrap();
    let first_id = handle.id;
    let Event::TrustRequired { id, review } = event(&mut handle).await else {
        panic!("Expected trust review")
    };
    assert_eq!(id, first_id);
    assert_eq!(review.sha256, first.review(port).sha256);
    handle.shutdown().await.unwrap();
    trust.approve(review).await.unwrap();
    assert!(trust.approve(changed.review(port)).await.is_err());
    let reloaded = store(&dir).await;
    go.send(()).unwrap();
    let mut handle = start(
        first.options(port, TlsAuth::Plain, false),
        credentials(TlsAuth::Plain),
        reloaded.clone(),
    )
    .unwrap();
    assert_ne!(first_id, handle.id);
    assert!(
        handle
            .try_input(
                first_id,
                Input::Key {
                    down: true,
                    keysym: 65
                }
            )
            .is_err()
    );
    assert!(matches!(
        event(&mut handle).await,
        Event::Connected {
            width: 3,
            height: 2,
            ..
        }
    ));
    assert!(matches!(event(&mut handle).await, Event::Frame { .. }));
    handle.shutdown().await.unwrap();
    go2.send(()).unwrap();
    let mut handle = start(
        changed.options(port, TlsAuth::Plain, true),
        credentials(TlsAuth::Plain),
        reloaded,
    )
    .unwrap();
    let Event::Closed { reason, .. } = event(&mut handle).await else {
        panic!("Changed cert did not block")
    };
    assert!(reason.contains("CHANGED"));
    handle.shutdown().await.unwrap();
    peer.await.unwrap();
}

#[tokio::test]
async fn wrong_name_and_expired_certificates_block_even_with_explicit_pins() {
    for (name, expired) in [("wrong.invalid", false), ("127.0.0.1", true)] {
        for pinned in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let trust = store(&dir).await;
            let fixture = Fixture::new(name, expired);
            let l = listener().await;
            let port = l.local_addr().unwrap().port();
            if pinned {
                trust.approve(fixture.review(port)).await.unwrap();
            }
            let server = fixture.server.clone();
            let peer = tokio::spawn(no_application_bytes(l, server));
            let mut handle = start(
                fixture.options(port, TlsAuth::Plain, false),
                credentials(TlsAuth::Plain),
                trust,
            )
            .unwrap();
            assert!(
                matches!(event(&mut handle).await, Event::Closed { .. }),
                "Invalid identity was offered for approval"
            );
            handle.shutdown().await.unwrap();
            peer.await.unwrap();
        }
    }
}

#[tokio::test]
async fn tls_plain_and_vnc_authentication_success_failure_and_framebuffer_input() {
    for (auth, version) in [
        (TlsAuth::Plain, &rustls::version::TLS13),
        (TlsAuth::Vnc, &rustls::version::TLS13),
        (TlsAuth::Plain, &rustls::version::TLS12),
        (TlsAuth::Vnc, &rustls::version::TLS12),
    ] {
        for success in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let trust = store(&dir).await;
            let fixture = Fixture::new_version("127.0.0.1", false, version);
            let l = listener().await;
            let port = l.local_addr().unwrap().port();
            let server = fixture.server.clone();
            let (finish, wait) = tokio::sync::oneshot::channel();
            let peer = tokio::spawn(async move {
                let mut s = tls_peer(&l, server, auth).await.unwrap();
                authentication(&mut s, auth, success).await.unwrap();
                if !success {
                    assert!(s.read_u8().await.is_err());
                    return;
                }
                init(&mut s).await.unwrap();
                request(&mut s, false).await.unwrap();
                for chunk in raw_update().chunks(3) {
                    s.write_all(chunk).await.unwrap();
                    s.flush().await.unwrap();
                }
                request(&mut s, true).await.unwrap();
                // Overlapping CopyRect: copy first two columns to last two.
                let mut update = vec![0, 0, 0, 1];
                rect(&mut update, [1, 0, 2, 2], 1);
                update.extend_from_slice(&[0, 0, 0, 0]);
                s.write_all(&update).await.unwrap();
                s.flush().await.unwrap();
                request(&mut s, true).await.unwrap();
                let mut key = [0; 8];
                s.read_exact(&mut key).await.unwrap();
                assert_eq!(key, [4, 1, 0, 0, 0, 0, 0, 65]);
                let mut pointer = [0; 6];
                s.read_exact(&mut pointer).await.unwrap();
                assert_eq!(pointer, [5, 1, 0, 2, 0, 1]);
                wait.await.unwrap();
            });
            let mut handle =
                start(fixture.options(port, auth, true), credentials(auth), trust).unwrap();
            if success {
                assert!(matches!(event(&mut handle).await, Event::Connected { .. }));
                let Event::Frame {
                    id,
                    framebuffer: first,
                } = event(&mut handle).await
                else {
                    panic!("Missing Raw frame")
                };
                assert_eq!(id, handle.id);
                assert_eq!(&first.rgba[..4], &[21, 11, 1, 255]);
                let Event::Frame {
                    framebuffer: second,
                    ..
                } = event(&mut handle).await
                else {
                    panic!("Missing CopyRect frame")
                };
                assert_eq!(
                    second
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| p[2])
                        .collect::<Vec<_>>(),
                    [1, 1, 2, 4, 4, 5]
                );
                assert_eq!(
                    first
                        .rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| p[2])
                        .collect::<Vec<_>>(),
                    [1, 2, 3, 4, 5, 6]
                );
                handle
                    .try_input(
                        handle.id,
                        Input::Key {
                            down: true,
                            keysym: 65,
                        },
                    )
                    .unwrap();
                handle
                    .try_input(
                        handle.id,
                        Input::Pointer {
                            buttons: 1,
                            x: 2,
                            y: 1,
                        },
                    )
                    .unwrap();
                // Let peer receive both input messages before closing it.
                tokio::task::yield_now().await;
                finish.send(()).unwrap();
            } else {
                let Event::Closed {
                    reason,
                    password_rejected,
                    ..
                } = event(&mut handle).await
                else {
                    panic!("Auth failure accepted")
                };
                assert!(reason.contains("authentication refused"));
                assert!(password_rejected);
            }
            peer.await.unwrap();
            handle.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn real_tls_ca_chain_is_validated_and_identity_is_pinned_before_credentials() {
    let mut ca = CertificateParams::new(vec!["fixture-ca.invalid".into()]).unwrap();
    ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca = rcgen::CertifiedIssuer::self_signed(ca, KeyPair::generate().unwrap()).unwrap();
    let key = KeyPair::generate().unwrap();
    let params = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    let cert = params.signed_by(&key, &ca).unwrap();
    let fixture = Fixture::from_der(cert.der().clone(), key);
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let l = listener().await;
    let port = l.local_addr().unwrap().port();
    let server = fixture.server.clone();
    let path = dir.path().join("vnc-pins.json");
    let peer = tokio::spawn(async move {
        let mut s = tls_peer(&l, server, TlsAuth::Plain).await.unwrap();
        authentication(&mut s, TlsAuth::Plain, false).await.unwrap();
        assert!(std::fs::read_to_string(path).unwrap().contains("127.0.0.1"));
    });
    let mut options = fixture.options(port, TlsAuth::Plain, false);
    options.roots.add(ca.der().clone()).unwrap();
    let mut handle = start(options, credentials(TlsAuth::Plain), trust).unwrap();
    let Event::Closed { reason, .. } = event(&mut handle).await else {
        panic!("CA chain rejected")
    };
    assert!(reason.contains("authentication refused"));
    peer.await.unwrap();
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn decoder_all_truncated_prefixes_bounds_and_unknown_encodings_fail() {
    let wire = raw_update();
    for length in 0..wire.len() {
        let mut frame = Framebuffer::new(3, 2).unwrap();
        let mut reader = &wire[..length];
        assert!(
            frame.update(&mut reader).await.is_err(),
            "Accepted prefix {length}"
        );
    }
    for (area, encoding, tail) in [
        ([2, 0, 2, 1], 0, vec![]),
        ([0, 0, 1, 1], 1, vec![0, 3, 0, 0]),
        ([0, 0, 1, 1], 99, vec![]),
        ([0, 0, 0, 1], 0, vec![]),
    ] {
        let mut wire = vec![0, 0, 0, 1];
        rect(&mut wire, area, encoding);
        wire.extend(tail);
        assert!(
            Framebuffer::new(3, 2)
                .unwrap()
                .update(&mut wire.as_slice())
                .await
                .is_err()
        );
    }
    assert!(Framebuffer::new(u16::MAX, u16::MAX).is_err());
    assert!(Framebuffer::new(0, 1).is_err());
    assert!(
        Framebuffer::new(3, 2)
            .unwrap()
            .update(&mut [0, 0, 0x10, 1].as_slice())
            .await
            .is_err()
    );
    // Vertical overlap must retain source rows too.
    let mut frame = Framebuffer::new(2, 3).unwrap();
    for (i, p) in frame.rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        p[0] = i as u8;
    }
    let mut wire = vec![0, 0, 0, 1];
    rect(&mut wire, [0, 1, 2, 2], 1);
    wire.extend_from_slice(&[0, 0, 0, 0]);
    frame.update(&mut wire.as_slice()).await.unwrap();
    assert_eq!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .collect::<Vec<_>>(),
        [0, 1, 0, 1, 2, 3]
    );
    // Small wire payload cannot trigger unbounded CopyRect work.
    let mut frame = Framebuffer::new(2048, 2048).unwrap();
    let mut wire = vec![0, 0, 0, 5];
    for _ in 0..5 {
        rect(&mut wire, [0, 0, 2048, 2048], 1);
        wire.extend_from_slice(&[0, 0, 0, 0]);
    }
    let error = frame.update(&mut wire.as_slice()).await.unwrap_err();
    assert!(error.to_string().contains("work exceeds limit"));
}

#[tokio::test]
async fn real_tls_truncated_frame_is_never_published_and_worker_can_cancel_under_backpressure() {
    for truncate in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let trust = store(&dir).await;
        let fixture = Fixture::new("127.0.0.1", false);
        let l = listener().await;
        let port = l.local_addr().unwrap().port();
        let server = fixture.server.clone();
        let peer = tokio::spawn(async move {
            let mut s = tls_peer(&l, server, TlsAuth::Plain).await.unwrap();
            authentication(&mut s, TlsAuth::Plain, true).await.unwrap();
            init(&mut s).await.unwrap();
            for i in 0..5 {
                if request(&mut s, i > 0).await.is_err() {
                    break;
                }
                let wire = raw_update();
                if truncate {
                    s.write_all(&wire[..wire.len() - 1]).await.unwrap();
                    s.shutdown().await.unwrap();
                    break;
                }
                s.write_all(&wire).await.unwrap();
                s.flush().await.unwrap();
            }
        });
        let mut handle = start(
            fixture.options(port, TlsAuth::Plain, true),
            credentials(TlsAuth::Plain),
            trust,
        )
        .unwrap();
        if truncate {
            assert!(matches!(event(&mut handle).await, Event::Connected { .. }));
            assert!(matches!(event(&mut handle).await, Event::Closed { .. }));
        } else {
            // Connected and first Frame fill capacity 2; next frame send blocks.
            tokio::time::timeout(Duration::from_secs(3), async {
                while handle.events.len() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        tokio::time::timeout(Duration::from_secs(2), handle.shutdown())
            .await
            .unwrap()
            .unwrap();
        peer.await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn decoder_trickling_cannot_reset_update_deadline() {
    let (mut client, mut peer) = tokio::io::duplex(64);
    let writer = tokio::spawn(async move {
        for byte in raw_update() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if peer.write_u8(byte).await.is_err() {
                break;
            }
        }
    });
    let now = tokio::time::Instant::now();
    let mut frame = Framebuffer::new(3, 2).unwrap();
    assert!(
        tokio::time::timeout(UPDATE_TIMEOUT, frame.update(&mut client))
            .await
            .is_err()
    );
    assert_eq!(now.elapsed(), UPDATE_TIMEOUT);
    writer.abort();
}

#[tokio::test]
async fn cancellation_during_tls_and_authentication_closes_socket_without_waiting_for_deadline() {
    for during_tls in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let trust = store(&dir).await;
        let fixture = Fixture::new("127.0.0.1", false);
        let l = listener().await;
        let port = l.local_addr().unwrap().port();
        let server = fixture.server.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            if during_tls {
                let (mut socket, _) = l.accept().await.unwrap();
                socket.write_all(b"RFB 003.008\n").await.unwrap();
                let mut banner = [0; 12];
                socket.read_exact(&mut banner).await.unwrap();
                socket.write_all(&[1, 19]).await.unwrap();
                assert_eq!(socket.read_u8().await.unwrap(), 19);
                socket.write_all(&[0, 2]).await.unwrap();
                socket.read_u16().await.unwrap();
                socket.write_all(&[0, 1, 0, 0, 1, 6]).await.unwrap();
                socket.read_u32().await.unwrap();
                socket.write_u8(1).await.unwrap();
                socket.flush().await.unwrap();
                ready.send(()).unwrap();
                let mut bytes = Vec::new();
                let result = socket.read_to_end(&mut bytes).await;
                // Windows may reset a socket with unread handshake bytes.
                assert!(
                    result.is_ok()
                        || result.unwrap_err().kind() == std::io::ErrorKind::ConnectionReset
                );
            } else {
                let mut s = tls_peer(&l, server, TlsAuth::Plain).await.unwrap();
                assert_eq!(s.read_u32().await.unwrap(), 7);
                assert_eq!(s.read_u32().await.unwrap(), 8);
                let mut credentials = [0; 15];
                s.read_exact(&mut credentials).await.unwrap();
                ready.send(()).unwrap();
                assert!(s.read_u8().await.is_err());
            }
        });
        let handle = start(
            fixture.options(port, TlsAuth::Plain, true),
            credentials(TlsAuth::Plain),
            trust,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), wait)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), handle.shutdown())
            .await
            .unwrap()
            .unwrap();
        peer.await.unwrap();
    }
}

#[test]
fn des_bit_reversal_matches_standard_known_answer() {
    // Standard DES known answer: key 133457799BBCDFF1, plaintext 0123456789ABCDEF.
    let password: Vec<u8> = [0x13u8, 0x34, 0x57, 0x79, 0x9b, 0xbc, 0xdf, 0xf1]
        .iter()
        .map(|b| b.reverse_bits())
        .collect();
    let challenge = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];
    assert_eq!(
        &vnc_response(&password, challenge)[..8],
        &[0x85, 0xe8, 0x13, 0x54, 0x0f, 0x0a, 0xb4, 0x05]
    );
}

#[tokio::test]
async fn interactive_auth_waits_for_explicit_credentials_and_release_all_sends_key_ups_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let fixture = Fixture::new("127.0.0.1", false);
    let l = listener().await;
    let port = l.local_addr().unwrap().port();
    let server = fixture.server.clone();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut s = tls_peer(&l, server, TlsAuth::Plain).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(60), s.read_u8())
                .await
                .is_err()
        );
        ready.send(()).unwrap();
        authentication(&mut s, TlsAuth::Plain, true).await.unwrap();
        init(&mut s).await.unwrap();
        request(&mut s, false).await.unwrap();
        s.write_all(&raw_update()).await.unwrap();
        s.flush().await.unwrap();
        request(&mut s, true).await.unwrap();
        let mut key = [0; 8];
        s.read_exact(&mut key).await.unwrap();
        assert_eq!(key, [4, 1, 0, 0, 0, 0, 0xff, 0xe1]);
        let mut pointer = [0; 6];
        s.read_exact(&mut pointer).await.unwrap();
        assert_eq!(pointer, [5, 1, 0, 2, 0, 1]);
        s.read_exact(&mut key).await.unwrap();
        assert_eq!(key, [4, 0, 0, 0, 0, 0, 0xff, 0xe1]);
        // Buttons are released where the pointer is, not at the origin.
        s.read_exact(&mut pointer).await.unwrap();
        assert_eq!(pointer, [5, 0, 0, 2, 0, 1]);
    });
    let mut handle = start_interactive(fixture.options(port, TlsAuth::Plain, true), trust).unwrap();
    assert!(matches!(
        event(&mut handle).await,
        Event::CredentialsRequired { .. }
    ));
    wait.await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::time::resume();
    handle
        .provide_credentials(credentials(TlsAuth::Plain))
        .unwrap();
    assert!(matches!(event(&mut handle).await, Event::Connected { .. }));
    assert!(matches!(event(&mut handle).await, Event::Frame { .. }));
    handle
        .try_input(
            handle.id,
            Input::Key {
                down: true,
                keysym: 0xffe1,
            },
        )
        .unwrap();
    handle
        .try_input(
            handle.id,
            Input::Pointer {
                buttons: 1,
                x: 2,
                y: 1,
            },
        )
        .unwrap();
    handle.try_input(handle.id, Input::ReleaseAll).unwrap();
    peer.await.unwrap();
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn worker_setup_and_framebuffer_deadlines_close_real_loopback_peers() {
    for setup in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let trust = store(&dir).await;
        let fixture = Fixture::new("127.0.0.1", false);
        let l = listener().await;
        let port = l.local_addr().unwrap().port();
        let server = fixture.server.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let mut s = tls_peer(&l, server, TlsAuth::Plain).await.unwrap();
            if setup {
                assert_eq!(s.read_u32().await.unwrap(), 7);
                assert_eq!(s.read_u32().await.unwrap(), 8);
                let mut bytes = [0; 15];
                s.read_exact(&mut bytes).await.unwrap();
            } else {
                authentication(&mut s, TlsAuth::Plain, true).await.unwrap();
                init(&mut s).await.unwrap();
                request(&mut s, false).await.unwrap();
                s.write_all(&raw_update()[..17]).await.unwrap();
                s.flush().await.unwrap();
            }
            ready.send(()).unwrap();
            assert!(s.read_u8().await.is_err());
        });
        let mut handle = start(
            fixture.options(port, TlsAuth::Plain, true),
            credentials(TlsAuth::Plain),
            trust,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), wait)
            .await
            .unwrap()
            .unwrap();
        if !setup {
            assert!(matches!(event(&mut handle).await, Event::Connected { .. }));
        }
        // Let the already-flushed first byte reach the reader before advancing
        // virtual time: silence itself no longer starts a framebuffer deadline.
        tokio::time::sleep(Duration::from_millis(10)).await;
        tokio::time::pause();
        tokio::time::advance(
            if setup { SETUP_TIMEOUT } else { UPDATE_TIMEOUT } + Duration::from_secs(1),
        )
        .await;
        let Event::Closed { reason, .. } = event(&mut handle).await else {
            panic!("Deadline did not close session")
        };
        assert!(
            reason.contains(if setup {
                "setup deadline"
            } else {
                "framebuffer deadline"
            }),
            "{reason}"
        );
        tokio::time::resume();
        handle.shutdown().await.unwrap();
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_credentials_corrupt_store_and_conflicting_concurrent_approvals_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let fixture = Fixture::new("127.0.0.1", false);
    let options = || fixture.options(5900, TlsAuth::Vnc, false);
    let invalid = Credentials {
        username: Zeroizing::new(String::new()),
        password: Zeroizing::new("ninebytes".into()),
    };
    assert!(start(options(), invalid, trust.clone()).is_err());
    let invalid = Credentials {
        username: Zeroizing::new("x".repeat(4097)),
        password: Zeroizing::new(String::new()),
    };
    assert!(
        start(
            fixture.options(5900, TlsAuth::Plain, false),
            invalid,
            trust.clone()
        )
        .is_err()
    );
    let other = Fixture::new("127.0.0.1", false);
    let (a, b) = tokio::join!(
        trust.approve(fixture.review(5900)),
        trust.approve(other.review(5900))
    );
    assert_ne!(a.is_ok(), b.is_ok());
    std::fs::write(dir.path().join("vnc-pins.json"), b"broken").unwrap();
    assert!(
        VncTrust::load(dir.path().join("vnc-pins.json"))
            .await
            .is_err()
    );
}

#[test]
fn transport_failures_explain_the_lost_connection() {
    let eof = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
    assert_eq!(
        close_reason(&eof),
        "Connection lost: the VNC server closed the connection"
    );
    let reset = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
        .context("VNC input deadline exceeded");
    assert_eq!(
        close_reason(&reset),
        "Connection lost: the VNC server reset the connection (VNC input deadline exceeded)"
    );
    let refused = anyhow::anyhow!("VNC authentication refused");
    assert_eq!(close_reason(&refused), "VNC authentication refused");
}

#[tokio::test]
async fn loopback_transport_keeps_original_certificate_name_and_pin_endpoint() {
    let fixture = Fixture::new("target.example", false);
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let listener = listener().await;
    let address = listener.local_addr().unwrap();
    let server = fixture.server.clone();
    let peer = tokio::spawn(async move {
        let _ = tls_peer(&listener, server, TlsAuth::Plain).await;
    });
    let mut options = fixture.options(5900, TlsAuth::Plain, false);
    options.host = "target.example".into();
    let mut handle = start_interactive_via(options, trust, address).unwrap();
    match event(&mut handle).await {
        Event::TrustRequired { review, .. } => {
            assert_eq!(review.endpoint, "[target.example]:5900");
            assert_eq!(review.sha256, trust::fingerprint(&fixture.certificate));
        }
        _ => panic!("Expected target certificate review through loopback transport"),
    }
    handle.shutdown().await.unwrap();
    peer.await.unwrap();
}
