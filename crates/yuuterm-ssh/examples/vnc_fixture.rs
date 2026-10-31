//! Synthetic desktop over real loopback VeNCrypt/TLS, never an external server.
use anyhow::{Result, ensure};
use std::{path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{TlsAcceptor, rustls};
#[tokio::main]
async fn main() -> Result<()> {
    let folder = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Fixture folder required"))?,
    );
    let key = rcgen::KeyPair::generate()?;
    let params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()])?;
    let cert = params.self_signed(&key)?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut profile = yuuterm_core::Profile {
        id: 1,
        name: "VNC fixture".into(),
        host: "127.0.0.1".into(),
        port: listener.local_addr()?.port(),
        username: "fixture".into(),
        protocol: yuuterm_core::Protocol::Vnc,
        ..Default::default()
    };
    profile.vnc.clipboard = true; // Explicit fixture opt-in; production default is off.
    if std::env::var("YUUTERM_VNC_FIXTURE_AUTH").as_deref() == Ok("vnc") {
        profile.vnc.auth = yuuterm_core::vnc::VncAuth::Vnc;
    }
    tokio::fs::write(folder.join("profile.json"), serde_json::to_vec(&profile)?).await?;
    let config = Arc::new(config);
    loop {
        let (s, _) = listener.accept().await?;
        let config = config.clone();
        let folder = folder.clone();
        tokio::spawn(async move {
            let _ = peer(s, config, folder).await;
        });
    }
}
async fn peer(mut s: TcpStream, config: Arc<rustls::ServerConfig>, folder: PathBuf) -> Result<()> {
    s.write_all(b"RFB 003.008\n").await?;
    let mut banner = [0; 12];
    s.read_exact(&mut banner).await?;
    s.write_all(&[1, 19]).await?;
    ensure!(s.read_u8().await? == 19, "security");
    s.write_all(&[0, 2]).await?;
    ensure!(s.read_u16().await? == 2, "version");
    let classic = std::env::var("YUUTERM_VNC_FIXTURE_AUTH").as_deref() == Ok("vnc");
    let subtype = if classic { 261 } else { 262 };
    s.write_all(&[0, 1]).await?;
    s.write_u32(subtype).await?;
    ensure!(s.read_u32().await? == subtype, "subtype");
    s.write_u8(1).await?;
    s.flush().await?;
    let mut s = TlsAcceptor::from(config).accept(s).await?;
    if classic {
        use des::cipher::{BlockCipherEncrypt, KeyInit};
        let key = b"password".map(u8::reverse_bits);
        let cipher = des::Des::new_from_slice(&key)?;
        let challenge = [0x42; 16];
        s.write_all(&challenge).await?;
        s.flush().await?;
        let mut expected = challenge;
        for chunk in expected.as_chunks_mut::<8>().0 {
            let mut block = des::cipher::Block::<des::Des>::default();
            block.copy_from_slice(chunk);
            cipher.encrypt_block(&mut block);
            chunk.copy_from_slice(&block);
        }
        let mut response = [0; 16];
        s.read_exact(&mut response).await?;
        ensure!(response == expected, "fixture VNC auth");
    } else {
        let n = s.read_u32().await? as usize;
        let p = s.read_u32().await? as usize;
        ensure!(n <= 256 && p <= 256, "auth size");
        let mut auth = vec![0; n + p];
        s.read_exact(&mut auth).await?;
        ensure!(auth == b"fixturepassword", "fixture auth");
    }
    s.write_u32(0).await?;
    s.flush().await?;
    ensure!(s.read_u8().await? == 1, "shared");
    s.write_u16(640).await?;
    s.write_u16(400).await?;
    s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0])
        .await?;
    s.write_u32(7).await?;
    s.write_all(b"fixture").await?;
    s.flush().await?;
    let mut format = [0; 20];
    s.read_exact(&mut format).await?;
    ensure!(s.read_u8().await? == 2, "encodings");
    s.read_u8().await?;
    let n = s.read_u16().await?;
    ensure!(n <= 16, "encodings size");
    for _ in 0..n {
        s.read_i32().await?;
    }
    let mut first = true;
    let encoding = match std::env::var("YUUTERM_VNC_FIXTURE_ENCODING").as_deref() {
        Ok("tight") => 7,
        Ok("zrle") => 16,
        _ => 0,
    };
    let mut compressor = flate2::Compress::new(flate2::Compression::fast(), true);
    loop {
        match s.read_u8().await? {
            3 => {
                let mut request = [0; 9];
                s.read_exact(&mut request).await?;
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                s.write_all(&[0, 0, 0, 1]).await?;
                let (x, y, w, h) = if first {
                    (0, 0, 640, 400)
                } else {
                    (20, 20, 80, 40)
                };
                for v in [x, y, w, h] {
                    s.write_u16(v).await?;
                }
                s.write_i32(encoding).await?;
                if encoding == 0 {
                    let mut row = Vec::with_capacity(w as usize * 4);
                    for y in y..y + h {
                        row.clear();
                        for x in x..x + w {
                            let tile = (x / 80 + y / 80) % 2;
                            row.extend_from_slice(if tile == 0 {
                                &[55, 40, 30, 0]
                            } else {
                                &[85, 65, 45, 0]
                            });
                        }
                        s.write_all(&row).await?;
                    }
                } else {
                    let mut data = Vec::new();
                    if encoding == 7 {
                        for dy in y..y + h {
                            for dx in x..x + w {
                                let bgr = color(dx, dy);
                                data.extend_from_slice(&[bgr[2], bgr[1], bgr[0]]);
                            }
                        }
                    } else {
                        for ty in (0..h).step_by(64) {
                            for tx in (0..w).step_by(64) {
                                data.push(0);
                                for dy in ty..(ty + 64).min(h) {
                                    for dx in tx..(tx + 64).min(w) {
                                        data.extend_from_slice(&color(x + dx, y + dy));
                                    }
                                }
                            }
                        }
                    }
                    let before = compressor.total_out();
                    let mut compressed = vec![0; data.len() * 2 + 128];
                    compressor.compress(&data, &mut compressed, flate2::FlushCompress::Sync)?;
                    compressed.truncate((compressor.total_out() - before) as usize);
                    if encoding == 7 {
                        s.write_u8(0).await?;
                        let len = compressed.len();
                        let mut length = vec![(len & 127) as u8];
                        if len > 127 {
                            length[0] |= 128;
                            length.push(((len >> 7) & 127) as u8);
                            if len > 16383 {
                                length[1] |= 128;
                                length.push((len >> 14) as u8);
                            }
                        }
                        s.write_all(&length).await?;
                    } else {
                        s.write_u32(compressed.len() as u32).await?;
                    }
                    s.write_all(&compressed).await?;
                }
                s.flush().await?;
                first = false;
            }
            4 => {
                let mut key = [0; 7];
                s.read_exact(&mut key).await?;
            }
            5 => {
                let mut pointer = [0; 5];
                s.read_exact(&mut pointer).await?;
            }
            6 => {
                let mut padding = [0; 3];
                s.read_exact(&mut padding).await?;
                let n = s.read_u32().await? as usize;
                ensure!(n <= 16384, "clipboard bound");
                let mut bytes = vec![0; n];
                s.read_exact(&mut bytes).await?;
                ensure!(
                    bytes == b"native clipboard fixture",
                    "reviewed clipboard payload"
                );
                tokio::fs::write(folder.join("clipboard-result.json"), b"{\"passed\":true}")
                    .await?;
            }
            _ => anyhow::bail!("unsupported fixture message"),
        }
    }
}
fn color(x: u16, y: u16) -> [u8; 3] {
    if (x / 80 + y / 80).is_multiple_of(2) {
        [55, 40, 30]
    } else {
        [85, 65, 45]
    }
}
