use super::*;
use flate2::{Compress, Compression, FlushCompress};
fn zip(stream: &mut Compress, data: &[u8]) -> Vec<u8> {
    let mut output = vec![0; data.len() * 2 + 128];
    let before_in = stream.total_in();
    let before_out = stream.total_out();
    stream
        .compress(data, &mut output, FlushCompress::Sync)
        .unwrap();
    assert_eq!(stream.total_in() - before_in, data.len() as u64);
    output.truncate((stream.total_out() - before_out) as usize);
    output
}
fn compressor() -> Compress {
    Compress::new(Compression::fast(), true)
}
fn compact_length(n: usize) -> Vec<u8> {
    let mut out = vec![(n & 127) as u8];
    if n > 127 {
        out[0] |= 128;
        out.push(((n >> 7) & 127) as u8);
        if n > 16383 {
            out[1] |= 128;
            out.push((n >> 14) as u8);
        }
    }
    out
}
fn update(w: u16, h: u16, encoding: i32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0, 1];
    for value in [0u16, 0, w, h] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&encoding.to_be_bytes());
    bytes.extend(payload);
    bytes
}
fn tight(stream: &mut Compress, control: u8, header: &[u8], pixels: &[u8]) -> Vec<u8> {
    let mut payload = vec![control];
    payload.extend(header);
    if pixels.len() < 12 {
        payload.extend(pixels);
    } else {
        let bytes = zip(stream, pixels);
        payload.extend(compact_length(bytes.len()));
        payload.extend(bytes);
    }
    payload
}
fn zrle(stream: &mut Compress, data: &[u8]) -> Vec<u8> {
    let bytes = zip(stream, data);
    let mut payload = (bytes.len() as u32).to_be_bytes().to_vec();
    payload.extend(bytes);
    payload
}
fn rgba(frame: &Framebuffer) -> Vec<[u8; 4]> {
    frame.rgba.as_chunks::<4>().0.to_vec()
}
async fn decode(w: u16, h: u16, encoding: i32, payload: &[u8]) -> Result<Framebuffer> {
    let mut frame = Framebuffer::new(w, h)?;
    let wire = update(w, h, encoding, payload);
    frame
        .update_with(&mut wire.as_slice(), &mut Decoder::default())
        .await?;
    Ok(frame)
}

#[tokio::test]
async fn tight_four_streams_persist_across_updates_and_resets_apply_to_fill_too() {
    let mut decoder = Decoder::default();
    let mut streams: [Compress; 4] = std::array::from_fn(|_| compressor());
    let mut frame = Framebuffer::new(4, 1).unwrap();
    for round in 0..3u8 {
        for (id, stream) in streams.iter_mut().enumerate() {
            let rgb = [round + 20, id as u8 + 40, 60];
            let payload = tight(stream, (id as u8) << 4, &[], &rgb.repeat(4));
            frame
                .update_with(&mut update(4, 1, 7, &payload).as_slice(), &mut decoder)
                .await
                .unwrap();
            assert_eq!(rgba(&frame), vec![[rgb[0], rgb[1], rgb[2], 255]; 4]);
            if round == 2 {
                assert!(
                    decode(4, 1, 7, &payload).await.is_err(),
                    "A new connection accepted another connection's zlib continuation"
                );
            }
        }
    }
    frame
        .update_with(
            &mut update(4, 1, 7, &[0x81, 1, 2, 3]).as_slice(),
            &mut decoder,
        )
        .await
        .unwrap();
    let payload = tight(&mut compressor(), 0, &[], &[4, 5, 6].repeat(4));
    frame
        .update_with(&mut update(4, 1, 7, &payload).as_slice(), &mut decoder)
        .await
        .unwrap();
    assert_eq!(rgba(&frame), vec![[4, 5, 6, 255]; 4]);
}

#[tokio::test]
async fn tight_inline_fill_palette_copy_and_gradient_are_exact() {
    assert_eq!(
        rgba(&decode(2, 1, 7, &[0, 1, 2, 3, 4, 5, 6]).await.unwrap()),
        [[1, 2, 3, 255], [4, 5, 6, 255]]
    );
    assert_eq!(
        rgba(&decode(2, 2, 7, &[0x80, 8, 9, 10]).await.unwrap()),
        vec![[8, 9, 10, 255]; 4]
    );
    // Two-color packed palette, each three-pixel row padded independently.
    let payload = tight(
        &mut compressor(),
        0x40,
        &[1, 1, 10, 20, 30, 40, 50, 60],
        &[0b01000000, 0b10100000],
    );
    let frame = decode(3, 2, 7, &payload).await.unwrap();
    assert_eq!(
        rgba(&frame),
        [
            [10, 20, 30, 255],
            [40, 50, 60, 255],
            [10, 20, 30, 255],
            [40, 50, 60, 255],
            [10, 20, 30, 255],
            [40, 50, 60, 255]
        ]
    );
    let mut palette = vec![1, 2];
    palette.extend([1, 2, 3, 4, 5, 6, 7, 8, 9]);
    let payload = tight(&mut compressor(), 0x40, &palette, &[2, 1, 0].repeat(4));
    assert_eq!(
        rgba(&decode(3, 4, 7, &payload).await.unwrap())[0],
        [7, 8, 9, 255]
    );
    // Differences for 2x2 pixels.
    let differences = [10, 20, 30, 10, 10, 10, 20, 20, 20, 10, 10, 10];
    let payload = tight(&mut compressor(), 0x40, &[2], &differences);
    assert_eq!(
        rgba(&decode(2, 2, 7, &payload).await.unwrap()),
        [
            [10, 20, 30, 255],
            [20, 30, 40, 255],
            [30, 40, 50, 255],
            [50, 60, 70, 255]
        ]
    );
    for (deltas, expected) in [
        ([250, 20, 20, 255], [250, 14, 14, 255]),
        ([10, 240, 240, 5], [10, 250, 250, 4]),
    ] {
        let data = deltas.into_iter().flat_map(|v| [v; 3]).collect::<Vec<_>>();
        let payload = tight(&mut compressor(), 0x40, &[2], &data);
        let frame = decode(2, 2, 7, &payload).await.unwrap();
        assert_eq!(
            rgba(&frame).into_iter().map(|p| p[0]).collect::<Vec<_>>(),
            expected
        );
    }
}

#[tokio::test]
async fn zrle_raw_solid_packed_palette_plain_and_palette_runs() {
    let cases: Vec<(Vec<u8>, [u8; 4])> = vec![
        ([vec![0], [3, 2, 1].repeat(6)].concat(), [1, 2, 3, 255]),
        (vec![1, 3, 2, 1], [1, 2, 3, 255]),
        (vec![2, 3, 2, 1, 6, 5, 4, 0, 0], [1, 2, 3, 255]),
        (vec![3, 3, 2, 1, 6, 5, 4, 9, 8, 7, 0, 0], [1, 2, 3, 255]),
        (
            [vec![5], [3, 2, 1].repeat(5), vec![0, 0, 0, 0]].concat(),
            [1, 2, 3, 255],
        ),
        (vec![128, 3, 2, 1, 5], [1, 2, 3, 255]),
        (vec![130, 3, 2, 1, 6, 5, 4, 0x81, 5], [4, 5, 6, 255]),
        (
            [vec![255], [3, 2, 1].repeat(127), vec![254, 5]].concat(),
            [1, 2, 3, 255],
        ),
    ];
    for (data, color) in cases {
        let payload = zrle(&mut compressor(), &data);
        assert_eq!(
            rgba(&decode(3, 2, 16, &payload).await.unwrap()),
            vec![color; 6]
        );
    }
    // Adjacent 64x64 tiles and one-pixel edge tiles.
    let data = [1, 3, 2, 1, 1, 6, 5, 4, 1, 9, 8, 7, 1, 12, 11, 10];
    let frame = decode(65, 65, 16, &zrle(&mut compressor(), &data))
        .await
        .unwrap();
    assert_eq!(&frame.rgba[(64 * 65 + 64) * 4..][..4], &[10, 11, 12, 255]);
}

#[tokio::test]
async fn zrle_state_persists_while_framebuffer_snapshots_are_independent() {
    let mut stream = compressor();
    let mut decoder = Decoder::default();
    let mut frame = Framebuffer::new(4, 1).unwrap();
    let data = [vec![0], [3, 2, 1].repeat(4)].concat();
    let first = zrle(&mut stream, &data);
    frame
        .update_with(&mut update(4, 1, 16, &first).as_slice(), &mut decoder)
        .await
        .unwrap();
    let old = frame.clone();
    let data = [vec![0], [6, 5, 4].repeat(4)].concat();
    let second = zrle(&mut stream, &data);
    frame
        .update_with(&mut update(4, 1, 16, &second).as_slice(), &mut decoder)
        .await
        .unwrap();
    assert_eq!(rgba(&old), vec![[1, 2, 3, 255]; 4]);
    assert_eq!(rgba(&frame), vec![[4, 5, 6, 255]; 4]);
    assert!(decode(4, 1, 16, &second).await.is_err());
}

#[tokio::test]
async fn every_compressed_prefix_truncated_tile_and_shortened_flush_is_refused() {
    let tight = tight(&mut compressor(), 0, &[], &[1, 2, 3].repeat(16));
    let zrle = zrle(&mut compressor(), &[vec![0], [3, 2, 1].repeat(16)].concat());
    for (encoding, payload) in [(7, tight), (16, zrle)] {
        let wire = update(4, 4, encoding, &payload);
        for end in 0..wire.len() {
            let mut frame = Framebuffer::new(4, 4).unwrap();
            assert!(
                frame
                    .update_with(&mut wire[..end].as_ref(), &mut Decoder::default())
                    .await
                    .is_err(),
                "Accepted truncated encoding {encoding} prefix {end}"
            );
        }
    }
    let bytes = zip(&mut compressor(), &[1, 2, 3].repeat(16));
    for end in 0..bytes.len() {
        assert!(inflate(&mut Decompress::new(true), &bytes[..end], 48).is_err());
    }
    for data in [
        vec![0, 1, 2],
        vec![2, 1, 2, 3],
        vec![128, 1, 2, 3],
        vec![130, 1, 2, 3, 4, 5, 6, 128],
    ] {
        assert!(decode(4, 4, 16, &zrle_payload(&data)).await.is_err());
    }
}
fn zrle_payload(data: &[u8]) -> Vec<u8> {
    zrle(&mut compressor(), data)
}

#[tokio::test]
async fn compressed_lengths_output_bombs_palette_indexes_and_run_overflows_fail_closed() {
    let oversize = ((MAX_COMPRESSED + 1) as u32).to_be_bytes();
    assert!(
        decode(4, 4, 16, &oversize)
            .await
            .unwrap_err()
            .to_string()
            .contains("input exceeds")
    );
    let bomb = zip(&mut compressor(), &vec![0; 65536]);
    assert!(inflate(&mut Decompress::new(true), &bomb, 48).is_err());
    let mut payload = vec![0];
    payload.extend(compact_length(bomb.len()));
    payload.extend(&bomb);
    assert!(decode(4, 4, 7, &payload).await.is_err());
    for data in [
        vec![128, 1, 2, 3, 255],
        vec![130, 1, 2, 3, 4, 5, 6, 255, 0],
        vec![3, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0xff, 0xff, 0xff, 0xff],
    ] {
        assert!(decode(4, 4, 16, &zrle_payload(&data)).await.is_err());
    }
    for control in [0x90, 0xa0, 0xe0, 0xff] {
        assert!(decode(4, 4, 7, &[control]).await.is_err());
    }
    assert!(decode(2049, 1, 7, &[0x80, 0, 0, 0]).await.is_err());
    let bad_palette = tight(
        &mut compressor(),
        0x40,
        &[1, 2, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        &[255; 16],
    );
    assert!(decode(4, 4, 7, &bad_palette).await.is_err());
    let mut decoder = Decoder {
        wire_bytes: UPDATE_BUDGET,
        ..Default::default()
    };
    assert!(decoder.blob(&mut [0u8].as_slice(), 1).await.is_err());
}

#[tokio::test]
async fn deterministic_mutation_corpus_is_bounded_and_never_panics() {
    for seed in 0..256u32 {
        let mut state = seed + 1;
        let mut payload = Vec::new();
        for _ in 0..48 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            payload.push((state >> 24) as u8);
        }
        let _ = decode(8, 8, 7, &payload).await;
        let _ = decode(8, 8, 16, &zrle_payload(&payload)).await;
    }
}
