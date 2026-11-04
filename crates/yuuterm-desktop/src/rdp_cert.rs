//! Remembered RDP server certificates ("Don't ask me again", like mstsc).
//!
//! Ticking "Don't ask me again" in the control's certificate prompt stores the
//! certificate's SHA-256 as CertHash under HKCU\Software\Microsoft\Terminal
//! Server Client\Servers\<host> (measured; the same value mstsc uses), but the
//! embedded control itself ignores that value on the next connect and never
//! fires OnReceivedTSPublicKey. So YuuTerm reads the server certificate itself
//! before connecting: X.224 negotiation (TLS/CredSSP requested) and a TLS
//! handshake, then close. No credentials are sent. When the certificate equals
//! the CertHash Windows stored (or one remembered by older YuuTerm versions in
//! rdp-certificates.json), the control connects without its prompt through the
//! loopback relay (`rdp_relay`), which checks the certificate again on the very
//! connection that authenticates (issue #16). Otherwise it asks as usual.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_rustls::rustls::{
    self, ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};

/// Accepts any certificate: this handshake only reads the certificate and
/// carries no data. Trust is decided by comparing the fingerprint afterwards.
#[derive(Debug)]
struct ReadOnly(Arc<rustls::crypto::CryptoProvider>);
impl ServerCertVerifier for ReadOnly {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// TCP stream whose every read and write is bounded by one overall deadline,
/// so a server trickling bytes cannot stretch the whole read.
pub struct Deadline {
    pub stream: TcpStream,
    pub until: Instant,
}
impl Deadline {
    fn remaining(&self) -> std::io::Result<Duration> {
        self.until
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "Certificate read timed out")
            })
    }
}
impl Read for Deadline {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.remaining()?;
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buf)
    }
}
impl Write for Deadline {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.remaining()?;
        self.stream.set_write_timeout(Some(left))?;
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

/// Connects within `timeout`, resolving on a helper thread so a slow resolver
/// cannot outlast the deadline; a late answer is dropped with the channel.
pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, String> {
    let until = Instant::now() + timeout;
    let (sender, receiver) = std::sync::mpsc::channel();
    let name = (host.to_owned(), port);
    std::thread::spawn(move || {
        let _ = sender.send(
            name.to_socket_addrs()
                .map(|mut a| a.next())
                .map_err(|e| e.to_string()),
        );
    });
    let address = receiver
        .recv_timeout(timeout)
        .map_err(|_| "Connection timed out")??
        .ok_or("Host has no address")?;
    let left = until
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or("Connection timed out")?;
    TcpStream::connect_timeout(&address, left).map_err(|e| e.to_string())
}

/// Lowercase hex SHA-256 of the server's TLS leaf certificate. Offers TLS 1.2
/// only, like the control (measured on mstscax 10.0.26100: its ClientHello has
/// no supported_versions; Windows 11 25H2 answers rustls' TLS 1.3 offer with
/// TLS 1.3). Blocking; call on a worker thread. The whole operation is bounded
/// by `timeout`.
pub fn fingerprint(host: &str, port: u16, timeout: Duration) -> Result<String, String> {
    let until = Instant::now() + timeout;
    let stream = connect(host, port, timeout)?;
    let mut stream = Deadline { stream, until };
    // TPKT + X.224 Connection Request + RDP_NEG_REQ (PROTOCOL_SSL | PROTOCOL_HYBRID).
    stream
        .write_all(&[
            0x03, 0x00, 0x00, 0x13, 0x0E, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08,
            0x00, 0x03, 0x00, 0x00, 0x00,
        ])
        .map_err(|e| e.to_string())?;
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).map_err(|e| e.to_string())?;
    let length = u16::from_be_bytes([header[2], header[3]]) as usize;
    if header[0] != 3 || !(11..=64).contains(&length) {
        return Err("Unexpected RDP negotiation response".into());
    }
    let mut body = vec![0u8; length - 4];
    stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    // RDP_NEG_RSP type 2 = response; type 3 = failure; absent = standard RDP security.
    if body.len() < 15 || body[7] != 2 {
        return Err("Server offers no TLS negotiation".into());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ReadOnly(provider)))
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned())
        .unwrap_or_else(|_| ServerName::try_from("rdp.invalid".to_owned()).expect("static name"));
    let mut tls = ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    while tls.is_handshaking() {
        tls.complete_io(&mut stream).map_err(|e| e.to_string())?;
    }
    let leaf = tls
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or("Server presented no certificate")?;
    let _ = stream.stream.shutdown(std::net::Shutdown::Both);
    Ok(yuuterm_ssh::hex::lower(Sha256::digest(leaf.as_ref())))
}

/// Largest store read or written. 1000 entries with 253-byte host names fit.
const STORE_LIMIT: u64 = 1024 * 1024;

/// host:port → certificate SHA-256 the user chose to remember. A missing store
/// is empty; an unreadable, oversized or invalid one is an error, so a later
/// write never replaces it with fewer pins.
fn load(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(STORE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > STORE_LIMIT {
        return Err("Remembered RDP certificate store is too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Remembered RDP certificate store is invalid".into())
}

pub fn store_path() -> Result<PathBuf, String> {
    yuuterm_core::config::path()
        .map(|p| p.with_file_name("rdp-certificates.json"))
        .map_err(|e| e.to_string())
}

fn key(host: &str, port: u16) -> String {
    format!("{}:{port}", host.to_ascii_lowercase())
}

pub fn remembered(path: &Path, host: &str, port: u16, fingerprint: &str) -> bool {
    load(path)
        .unwrap_or_default()
        .get(&key(host, port))
        .is_some_and(|f| f == fingerprint)
}

/// Writes a pin. Only older versions wrote pins in production; kept for the
/// tests and the review_rdp_store witness, which check the store stays bounded.
#[allow(dead_code)]
pub fn remember(path: &Path, host: &str, port: u16, fingerprint: &str) -> Result<(), String> {
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid certificate fingerprint".into());
    }
    let mut map = load(path)?;
    if map.len() >= 1000 && !map.contains_key(&key(host, port)) {
        return Err("Too many remembered RDP certificates".into());
    }
    map.insert(key(host, port), fingerprint.to_ascii_lowercase());
    write(path, &map)
}

pub fn forget(path: &Path, host: &str, port: u16) -> Result<(), String> {
    let mut map = load(path)?;
    if map.remove(&key(host, port)).is_some() {
        write(path, &map)?;
    }
    Ok(())
}

fn write(path: &Path, map: &BTreeMap<String, String>) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(map).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > STORE_LIMIT {
        return Err("Too many remembered RDP certificates".into());
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

/// Registry keys the control uses for a server: the host name, and host:port
/// for a non-default port.
fn windows_keys(host: &str, port: u16) -> Vec<String> {
    let base = r"Software\Microsoft\Terminal Server Client\Servers\";
    let mut keys = vec![format!("{base}{host}")];
    if port != 3389 {
        keys.push(format!("{base}{host}:{port}"));
    }
    keys
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The CertHash Windows stored for the server, as lowercase hex.
fn windows_hashes(host: &str, port: u16) -> Vec<String> {
    use windows::{
        Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW},
        core::PCWSTR,
    };
    let name = wide("CertHash");
    windows_keys(host, port)
        .into_iter()
        .filter_map(|key| {
            let key = wide(&key);
            let mut bytes = [0u8; 64];
            let mut size = bytes.len() as u32;
            unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    PCWSTR(key.as_ptr()),
                    PCWSTR(name.as_ptr()),
                    RRF_RT_REG_BINARY,
                    None,
                    Some(bytes.as_mut_ptr().cast()),
                    Some(&mut size),
                )
            }
            .ok()
            .ok()?;
            let hash = &bytes[..size as usize];
            Some(hash.iter().map(|b| format!("{b:02x}")).collect())
        })
        .collect()
}

/// The user ticked "Don't ask me again" for this exact certificate (Windows'
/// CertHash), or remembered it with an older YuuTerm version.
pub fn trusted(path: &Path, host: &str, port: u16, fingerprint: &str) -> bool {
    remembered(path, host, port, fingerprint)
        || windows_hashes(host, port)
            .iter()
            .any(|hash| hash.eq_ignore_ascii_case(fingerprint))
}

/// Whether any certificate is remembered for the server (no network access).
pub fn any_remembered(path: &Path, host: &str, port: u16) -> bool {
    load(path)
        .unwrap_or_default()
        .contains_key(&key(host, port))
        || !windows_hashes(host, port).is_empty()
}

/// Forgets the server's certificate in YuuTerm and in Windows ("Don't ask me
/// again" is undone for this server, also for mstsc).
pub fn forget_everywhere(path: &Path, host: &str, port: u16) -> Result<(), String> {
    use windows::{
        Win32::{
            Foundation::ERROR_FILE_NOT_FOUND,
            System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyValueW},
        },
        core::PCWSTR,
    };
    forget(path, host, port)?;
    let name = wide("CertHash");
    for key in windows_keys(host, port) {
        let key = wide(&key);
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        };
        if status.is_err() && status != ERROR_FILE_NOT_FOUND {
            return Err(format!(
                "Could not forget the Windows certificate entry ({})",
                status.0
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remembered_certificates_are_per_host_port_and_replaceable() {
        let dir = std::env::temp_dir().join(format!("yuuterm-rdp-cert-{}", std::process::id()));
        let path = dir.join("rdp-certificates.json");
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        assert!(!remembered(&path, "Host.Test", 3389, &a));
        remember(&path, "Host.Test", 3389, &a).unwrap();
        assert!(remembered(&path, "host.test", 3389, &a));
        assert!(
            !remembered(&path, "host.test", 3390, &a),
            "port is part of the key"
        );
        assert!(
            !remembered(&path, "host.test", 3389, &b),
            "a changed certificate asks again"
        );
        remember(&path, "host.test", 3389, &b).unwrap();
        assert!(remembered(&path, "host.test", 3389, &b));
        assert!(remember(&path, "host.test", 3389, "not-hex").is_err());
        forget(&path, "HOST.test", 3389).unwrap();
        assert!(!remembered(&path, "host.test", 3389, &b));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn full_store_keeps_every_pin_and_invalid_store_is_not_replaced() {
        let dir = std::env::temp_dir().join(format!("yuuterm-rdp-full-{}", std::process::id()));
        let path = dir.join("rdp-certificates.json");
        let pin = "c".repeat(64);
        let host = |i: usize| format!("{i:04}.{}", "h".repeat(248));
        for i in 0..1000 {
            remember(&path, &host(i), 65535, &pin).unwrap();
        }
        assert!(remember(&path, "one-more.test", 3389, &pin).is_err());
        for i in 0..1000 {
            assert!(remembered(&path, &host(i), 65535, &pin), "pin {i} lost");
        }
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(remember(&path, "new.test", 3389, &pin).is_err());
        assert!(forget(&path, "new.test", 3389).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn fingerprint_refuses_a_listener_without_rdp_negotiation() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 19];
            let _ = s.read_exact(&mut buf);
            // TPKT with an X.224 confirm but no RDP_NEG_RSP (standard RDP security).
            let _ = s.write_all(&[3, 0, 0, 11, 6, 0xD0, 0, 0, 0, 0, 0]);
        });
        assert!(fingerprint("127.0.0.1", port, Duration::from_secs(2)).is_err());
        worker.join().unwrap();
    }
    #[test]
    fn fingerprint_deadline_covers_a_trickling_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 19];
            let _ = s.read_exact(&mut buf);
            // A valid-length TPKT header, then one byte every 40 ms: each read
            // finishes well inside the timeout, the whole response does not.
            for byte in [3u8, 0, 0, 19]
                .into_iter()
                .chain(std::iter::repeat_n(0, 15))
            {
                if s.write_all(&[byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let start = Instant::now();
        assert!(fingerprint("127.0.0.1", port, Duration::from_millis(150)).is_err());
        assert!(
            start.elapsed() < Duration::from_millis(400),
            "took {:?}",
            start.elapsed()
        );
        worker.join().unwrap();
    }
}
