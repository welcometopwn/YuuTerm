use super::*;
fn layout(reason: u16, status: u16, w: u16, h: u16) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0, 1];
    rect(&mut bytes, [reason, status, w, h], -308);
    bytes.extend([1, 0, 0, 0]);
    bytes.extend(42u32.to_be_bytes());
    for n in [0u16, 0, w, h] {
        bytes.extend(n.to_be_bytes());
    }
    bytes.extend(0x8000_0001u32.to_be_bytes());
    bytes
}
#[tokio::test]
async fn extended_layout_bounds_refusal_and_every_truncated_prefix_are_checked() {
    let bytes = layout(0, 0, 4, 3);
    for n in 0..bytes.len() {
        let mut f = Framebuffer::new(3, 2).unwrap();
        assert!(f.update(&mut &bytes[..n]).await.is_err(), "prefix {n}");
    }
    let mut f = Framebuffer::new(3, 2).unwrap();
    f.update(&mut &bytes[..]).await.unwrap();
    assert_eq!((f.width, f.height), (4, 3));
    assert_eq!(f.resize.unwrap().id, 42);
    assert_eq!(f.resize.unwrap().flags, 0x8000_0001);
    f.rgba[0] = 123;
    f.update(&mut &bytes[..]).await.unwrap();
    assert_eq!(f.rgba[0], 123);
    f.update(&mut &layout(1, 1, 0, 0)[..]).await.unwrap();
    assert_eq!((f.width, f.height), (4, 3));
    assert_eq!(f.resize_status, Some(1));
    assert!(f.resize.is_none());
    assert!(
        f.update(&mut &layout(0, 0, 65535, 65535)[..])
            .await
            .is_err()
    );
    let mut excess = bytes;
    excess[16] = 17;
    assert!(f.update(&mut &excess[..]).await.is_err());
}
fn cursor(hotspot: [u16; 2], w: u16, h: u16, mask: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0, 1];
    rect(&mut bytes, [hotspot[0], hotspot[1], w, h], -239);
    for i in 0..w as u32 * h as u32 {
        bytes.extend([i as u8, 0x20, 0x30, 0]); // BGRX
    }
    bytes.extend(mask);
    bytes
}
#[tokio::test]
async fn cursor_shapes_apply_the_mask_survive_resize_and_reject_bad_hotspots() {
    // 9x2: the mask stride is two bytes; first row all visible, second row only x=8.
    let bytes = cursor([4, 1], 9, 2, &[0xff, 0x80, 0x00, 0x80]);
    for n in 0..bytes.len() {
        let mut f = Framebuffer::new(3, 2).unwrap();
        assert!(f.update(&mut &bytes[..n]).await.is_err(), "prefix {n}");
    }
    let mut f = Framebuffer::new(3, 2).unwrap();
    let pixels = f.rgba.clone();
    f.update(&mut &bytes[..]).await.unwrap();
    assert_eq!(f.rgba, pixels, "cursor never paints into the framebuffer");
    let c = f.cursor.clone().unwrap();
    assert_eq!((c.hotspot_x, c.hotspot_y, c.width, c.height), (4, 1, 9, 2));
    assert_eq!(&c.rgba[..4], &[0x30, 0x20, 0, 255]);
    let alpha = |x: usize, y: usize| c.rgba[(y * 9 + x) * 4 + 3];
    assert_eq!((alpha(8, 0), alpha(0, 1), alpha(8, 1)), (255, 0, 255));
    f.update(&mut &layout(0, 0, 4, 3)[..]).await.unwrap();
    assert!(
        f.cursor.is_some(),
        "a desktop resize keeps the cursor shape"
    );
    f.update(&mut &cursor([0, 0], 0, 0, &[])[..]).await.unwrap();
    assert_eq!(f.cursor.unwrap().width, 0, "empty shape hides the cursor");
    let mut f = Framebuffer::new(3, 2).unwrap();
    assert!(
        f.update(&mut &cursor([2, 0], 2, 1, &[0xc0])[..])
            .await
            .is_err()
    );
    let big = cursor([0, 0], 200, 1, &[0xff; 25]);
    f.update(&mut &big[..]).await.unwrap();
    assert!(
        f.cursor.is_none(),
        "shapes over 128 px fall back to the local arrow"
    );
    let mut huge = cursor([0, 0], 1, 1, &[0x80]);
    huge[8..10].copy_from_slice(&257u16.to_be_bytes());
    assert!(f.update(&mut &huge[..]).await.is_err());
}
#[tokio::test]
async fn clipboard_and_message_lengths_are_bounded_without_exposing_server_text() {
    assert!(validate_clipboard("Grüße\n").is_ok());
    assert!(validate_clipboard("€").is_err());
    assert!(validate_clipboard(&"x".repeat(MAX_CLIPBOARD_BYTES + 1)).is_err());
    let mut f = Framebuffer::new(1, 1).unwrap();
    let mut bytes = vec![3, 0, 0, 0];
    bytes.extend(6u32.to_be_bytes());
    bytes.extend(b"secret");
    for n in 0..bytes.len() {
        assert!(f.update(&mut &bytes[..n]).await.is_err());
    }
    f.update(&mut &bytes[..]).await.unwrap();
    assert_eq!(f.rgba, [0, 0, 0, 255]);
    let mut excess = vec![3, 0, 0, 0];
    excess.extend((MAX_CLIPBOARD_BYTES as u32 + 1).to_be_bytes());
    assert!(f.update(&mut &excess[..]).await.is_err());
}
#[tokio::test]
async fn real_tls_clipboard_and_extended_resize_preserve_screen_identity_and_refuse_safely() {
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let fixture = Fixture::new("127.0.0.1", false);
    let l = listener().await;
    let port = l.local_addr().unwrap().port();
    let config = fixture.server.clone();
    let (observed, received) = tokio::sync::oneshot::channel();
    let (advertise, begin) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut s = tls_peer(&l, config, TlsAuth::Plain).await.unwrap();
        authentication(&mut s, TlsAuth::Plain, true).await.unwrap();
        init(&mut s).await.unwrap();
        request(&mut s, false).await.unwrap();
        begin.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), s.read_u8())
                .await
                .is_err(),
            "No SetDesktopSize before capability confirmation"
        );
        s.write_all(&layout(0, 0, 3, 2)).await.unwrap();
        s.flush().await.unwrap();
        request(&mut s, true).await.unwrap();
        // Server clipboard and Bell do not cause an update request or UI event.
        s.write_all(&[
            3, 0, 0, 0, 0, 0, 0, 6, b's', b'e', b'c', b'r', b'e', b't', 2,
        ])
        .await
        .unwrap();
        s.flush().await.unwrap();
        assert_eq!(s.read_u8().await.unwrap(), 6);
        let mut header = [0; 3];
        s.read_exact(&mut header).await.unwrap();
        assert_eq!(header, [0; 3]);
        assert_eq!(s.read_u32().await.unwrap(), 5);
        let mut text = [0; 5];
        s.read_exact(&mut text).await.unwrap();
        assert_eq!(&text, b"Gr\xfc\xdfe");
        let mut resize = [0; 24];
        s.read_exact(&mut resize).await.unwrap();
        assert_eq!(&resize[..8], &[251, 0, 0, 4, 0, 3, 1, 0]);
        assert_eq!(&resize[8..12], &42u32.to_be_bytes());
        assert_eq!(&resize[20..], &0x8000_0001u32.to_be_bytes());
        s.write_all(&layout(1, 0, 4, 3)).await.unwrap();
        s.flush().await.unwrap();
        let mut update = [0; 10];
        s.read_exact(&mut update).await.unwrap();
        assert_eq!(update, [3, 1, 0, 0, 0, 0, 0, 4, 0, 3]);
        s.write_all(&layout(1, 3, 0, 0)).await.unwrap();
        s.flush().await.unwrap();
        observed.send(()).unwrap();
        let mut discard = [0; 10];
        let _ = s.read_exact(&mut discard).await;
        std::future::pending::<()>().await;
    });
    let mut handle = start(
        fixture.options(port, TlsAuth::Plain, true),
        credentials(TlsAuth::Plain),
        trust,
    )
    .unwrap();
    assert!(matches!(event(&mut handle).await, Event::Connected { .. }));
    handle
        .try_input(
            handle.id,
            Input::Resize {
                width: 4,
                height: 3,
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while handle.input.capacity() < 64 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    advertise.send(()).unwrap();
    let Event::Frame { framebuffer, .. } = event(&mut handle).await else {
        panic!("layout expected")
    };
    assert!(framebuffer.resize.is_some());
    handle
        .try_input(
            handle.id,
            Input::Clipboard {
                text: Zeroizing::new("Grüße".into()),
            },
        )
        .unwrap();
    handle
        .try_input(
            handle.id,
            Input::Resize {
                width: 4,
                height: 3,
            },
        )
        .unwrap();
    let Event::Frame { framebuffer, .. } = event(&mut handle).await else {
        panic!("resize ack expected")
    };
    assert_eq!((framebuffer.width, framebuffer.height), (4, 3));
    let Event::Frame { framebuffer, .. } = event(&mut handle).await else {
        panic!("refusal expected")
    };
    assert_eq!(framebuffer.resize_status, Some(3));
    assert_eq!((framebuffer.width, framebuffer.height), (4, 3));
    received.await.unwrap();
    handle.shutdown().await.unwrap();
    peer.abort();
}
#[tokio::test]
async fn real_tls_static_desktop_survives_an_hour_of_idle_and_cancel_still_closes_it() {
    let dir = tempfile::tempdir().unwrap();
    let trust = store(&dir).await;
    let fixture = Fixture::new("127.0.0.1", false);
    let l = listener().await;
    let port = l.local_addr().unwrap().port();
    let config = fixture.server.clone();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let (resume, go) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut s = tls_peer(&l, config, TlsAuth::Plain).await.unwrap();
        authentication(&mut s, TlsAuth::Plain, true).await.unwrap();
        init(&mut s).await.unwrap();
        request(&mut s, false).await.unwrap();
        ready.send(()).unwrap();
        go.await.unwrap();
        s.write_all(&raw_update()).await.unwrap();
        s.flush().await.unwrap();
        std::future::pending::<()>().await;
    });
    let mut handle = start(
        fixture.options(port, TlsAuth::Plain, true),
        credentials(TlsAuth::Plain),
        trust,
    )
    .unwrap();
    assert!(matches!(event(&mut handle).await, Event::Connected { .. }));
    wait.await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3600)).await;
    tokio::task::yield_now().await;
    assert!(matches!(
        handle.events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    tokio::time::resume();
    resume.send(()).unwrap();
    assert!(matches!(event(&mut handle).await, Event::Frame { .. }));
    handle.shutdown().await.unwrap();
    peer.abort();
}
