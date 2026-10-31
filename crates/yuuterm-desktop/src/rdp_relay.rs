//! Binds a remembered RDP certificate to the connection that authenticates
//! (issue #16, review F1). The embedded control cannot pin a certificate, and
//! without its own check (AuthenticationLevel 0) it would trust whatever the
//! network presents. So a remembered connection goes through this loopback
//! relay: the control connects to 127.0.0.1, the relay connects to the server
//! and holds back everything the server sends until it has seen the X.224
//! negotiation select TLS and the TLS 1.2 handshake present the remembered
//! certificate. Only then are the held bytes released and the relay becomes a
//! plain pipe. Anything else closes both sides before the control can answer,
//! so no credential leaves this machine. The control's TLS stack still checks
//! that the peer owns the certificate's key (ServerKeyExchange signature or RSA
//! key exchange), and CredSSP binds to that key, so a forwarded handshake gives
//! an attacker nothing. The control offers TLS 1.2 only (measured, see
//! `rdp_cert::fingerprint`); should a later one get TLS 1.3, which encrypts the
//! certificate, the relay stops before sign-in and says so.
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// How long the control may take to connect after Connect.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound for reaching the server and for its handshake up to the certificate.
const SERVER_TIMEOUT: Duration = Duration::from_secs(15);
/// Most server bytes held back before the certificate (chains are a few KiB).
const HOLD_LIMIT: usize = 64 * 1024;

#[derive(Default)]
struct Shared {
    /// Why the relay closed the connection before the certificate was confirmed.
    refusal: Option<String>,
    /// Sockets to shut down when the session goes away.
    streams: Vec<TcpStream>,
    closed: bool,
}

/// One relayed connection. Dropping it closes the relay and its sockets.
pub struct Relay {
    port: u16,
    shared: Arc<Mutex<Shared>>,
}

impl Relay {
    /// Listens on an ephemeral loopback port for one connection to `host:port`
    /// whose leaf certificate must have SHA-256 `pin`.
    pub fn start(host: String, port: u16, pin: String) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let local = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("rdp-relay".into())
            .spawn(move || {
                if let Err(reason) = run(listener, &host, port, &pin, &worker)
                    && let Ok(mut state) = worker.lock()
                {
                    state.refusal.get_or_insert(reason);
                    for stream in state.streams.drain(..) {
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            port: local,
            shared,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Set when the relay stopped the connection before sign-in.
    pub fn refusal(&self) -> Option<String> {
        self.shared.lock().ok()?.refusal.clone()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.lock() {
            state.closed = true;
            for stream in state.streams.drain(..) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

/// Registers a socket for shutdown; false once the relay was dropped.
fn track(shared: &Mutex<Shared>, stream: &TcpStream) -> Result<(), String> {
    let mut state = shared.lock().map_err(|_| "RDP relay state unavailable")?;
    if state.closed {
        return Err("Closed".into());
    }
    state
        .streams
        .push(stream.try_clone().map_err(|e| e.to_string())?);
    Ok(())
}

fn run(
    listener: TcpListener,
    host: &str,
    port: u16,
    pin: &str,
    shared: &Mutex<Shared>,
) -> Result<(), String> {
    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    let client = loop {
        if shared.lock().map_or(true, |s| s.closed) {
            return Ok(());
        }
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("The remote desktop client did not connect".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    // Exactly one connection: a retry by the control (for example without TLS)
    // finds nothing listening.
    drop(listener);
    client.set_nonblocking(false).map_err(|e| e.to_string())?;
    track(shared, &client)?;
    let server = crate::rdp_cert::connect(host, port, SERVER_TIMEOUT)
        .map_err(|e| format!("Could not reach {host}: {e}"))?;
    track(shared, &server)?;
    let _ = client.set_nodelay(true);
    let _ = server.set_nodelay(true);

    // The control's requests carry nothing secret before it has the server's
    // certificate, so they flow at once.
    let mut upstream = server.try_clone().map_err(|e| e.to_string())?;
    let mut from_client = client.try_clone().map_err(|e| e.to_string())?;
    std::thread::Builder::new()
        .name("rdp-relay-up".into())
        .spawn(move || {
            let _ = std::io::copy(&mut from_client, &mut upstream);
            let _ = upstream.shutdown(Shutdown::Write);
        })
        .map_err(|e| e.to_string())?;

    let mut from_server = server;
    let mut to_client = client;
    let held = {
        let mut bounded = crate::rdp_cert::Deadline {
            stream: from_server.try_clone().map_err(|e| e.to_string())?,
            until: Instant::now() + SERVER_TIMEOUT,
        };
        let mut reader = Holding {
            inner: &mut bounded,
            held: Vec::new(),
        };
        check(&mut reader, &mut to_client, pin)?;
        reader.held
    };
    // The deadline set a timeout on the shared socket.
    from_server
        .set_read_timeout(None)
        .map_err(|e| e.to_string())?;
    to_client.write_all(&held).map_err(|e| e.to_string())?;
    let _ = std::io::copy(&mut from_server, &mut to_client);
    let _ = to_client.shutdown(Shutdown::Both);
    let _ = from_server.shutdown(Shutdown::Both);
    Ok(())
}

/// Records every byte read so it can be released once the check passed.
struct Holding<'a, R: Read> {
    inner: &'a mut R,
    held: Vec<u8>,
}
impl<R: Read> Holding<'_, R> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        if self.held.len() + count > HOLD_LIMIT {
            return Err("The server's TLS handshake is too large".into());
        }
        let start = self.held.len();
        self.held.resize(start + count, 0);
        self.inner
            .read_exact(&mut self.held[start..])
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                    "The server did not present its certificate in time"
                }
                _ => "The server closed the connection before its certificate",
            })?;
        Ok(&self.held[start..])
    }
}

/// Reads the X.224 Connection Confirm and the TLS handshake up to the server
/// certificate. Ok only when TLS was negotiated and the leaf certificate's
/// SHA-256 is `pin`. The confirm goes to the client as soon as it selected TLS,
/// because the client sends its ClientHello only after it; the handshake stays
/// held.
fn check<R: Read>(
    server: &mut Holding<'_, R>,
    client: &mut impl Write,
    pin: &str,
) -> Result<(), String> {
    let header = server.take(4)?;
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    if header[0] != 3 || !(11..=64).contains(&length) {
        return Err("Unexpected RDP negotiation response".into());
    }
    let body = server.take(length - 4)?;
    // RDP_NEG_RSP (type 2) selecting PROTOCOL_SSL, _HYBRID or _HYBRID_EX: all
    // start with TLS. Standard RDP security or a failure has no certificate.
    let selected = (body.len() >= 15 && body[7] == 2)
        .then(|| u32::from_le_bytes([body[11], body[12], body[13], body[14]]));
    if !matches!(selected, Some(1 | 2 | 8)) {
        return Err("The server did not negotiate TLS".into());
    }
    client
        .write_all(&server.held)
        .map_err(|_| "The remote desktop client closed the connection")?;
    server.held.clear();
    let mut handshake = Vec::new();
    let mut hello = false;
    loop {
        let record = server.take(5)?;
        let (kind, size) = (
            record[0],
            usize::from(u16::from_be_bytes([record[3], record[4]])),
        );
        // Plain handshake records only: an alert, ChangeCipherSpec (session
        // resumption, no certificate) or data ends the check.
        if kind != 22 || size == 0 || size > 16384 {
            return Err("The server did not present its certificate".into());
        }
        handshake.extend_from_slice(server.take(size)?);
        while handshake.len() >= 4 {
            let size = usize::from(handshake[1]) << 16
                | usize::from(handshake[2]) << 8
                | usize::from(handshake[3]);
            if handshake.len() < 4 + size {
                break;
            }
            let message: Vec<u8> = handshake.drain(..4 + size).collect();
            match message[0] {
                2 if !hello => {
                    server_hello(&message[4..])?;
                    hello = true;
                }
                11 if hello => return certificate(&message[4..], pin),
                _ => return Err("The server did not present its certificate".into()),
            }
        }
    }
}

/// Accepts TLS 1.0–1.2; TLS 1.3 (also a HelloRetryRequest) hides the certificate.
fn server_hello(body: &[u8]) -> Result<(), String> {
    let malformed = || "Malformed TLS ServerHello".to_owned();
    if body.len() < 35 {
        return Err(malformed());
    }
    let version = u16::from_be_bytes([body[0], body[1]]);
    if !(0x0301..=0x0303).contains(&version) {
        return Err("The server chose an unsupported TLS version".into());
    }
    let session = usize::from(body[34]);
    let mut at = 35 + session + 3;
    if body.len() < at {
        return Err(malformed());
    }
    if body.len() == at {
        return Ok(());
    }
    let length = usize::from(u16::from_be_bytes([
        *body.get(at).ok_or_else(malformed)?,
        *body.get(at + 1).ok_or_else(malformed)?,
    ]));
    at += 2;
    let end = at + length;
    if body.len() != end {
        return Err(malformed());
    }
    while at + 4 <= end {
        let kind = u16::from_be_bytes([body[at], body[at + 1]]);
        let size = usize::from(u16::from_be_bytes([body[at + 2], body[at + 3]]));
        at += 4;
        if at + size > end {
            return Err(malformed());
        }
        // supported_versions: TLS 1.3 encrypts the certificate.
        if kind == 43 {
            return Err("The server chose TLS 1.3, so its certificate cannot be checked".into());
        }
        at += size;
    }
    if at != end {
        return Err(malformed());
    }
    Ok(())
}

/// TLS 1.0–1.2 Certificate message: the first entry is the leaf.
fn certificate(body: &[u8], pin: &str) -> Result<(), String> {
    let u24 = |b: &[u8]| usize::from(b[0]) << 16 | usize::from(b[1]) << 8 | usize::from(b[2]);
    if body.len() < 6 || u24(body) != body.len() - 3 {
        return Err("Malformed TLS Certificate".into());
    }
    let leaf = u24(&body[3..]);
    let der = body
        .get(6..6 + leaf)
        .filter(|der| !der.is_empty())
        .ok_or("Malformed TLS Certificate")?;
    if format!("{:x}", Sha256::digest(der)).eq_ignore_ascii_case(pin) {
        Ok(())
    } else {
        Err("The server's certificate is not the remembered one".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIRM_HYBRID: [u8; 19] = [
        3, 0, 0, 19, 14, 0xD0, 0, 0, 0x12, 0x34, 0, 2, 0x1F, 8, 0, 2, 0, 0, 0,
    ];

    fn record(messages: &[Vec<u8>]) -> Vec<u8> {
        let payload: Vec<u8> = messages.concat();
        let mut out = vec![22, 3, 3];
        out.extend((payload.len() as u16).to_be_bytes());
        out.extend(payload);
        out
    }
    fn message(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend(body);
        out
    }
    fn hello(extensions: &[u8]) -> Vec<u8> {
        let mut body = vec![3, 3];
        body.extend([7; 32]);
        body.push(0);
        body.extend([0xC0, 0x30, 0]);
        if !extensions.is_empty() {
            body.extend((extensions.len() as u16).to_be_bytes());
            body.extend(extensions);
        }
        message(2, &body)
    }
    fn certificates(leaf: &[u8]) -> Vec<u8> {
        let mut entry = (leaf.len() as u32).to_be_bytes()[1..].to_vec();
        entry.extend(leaf);
        let mut body = (entry.len() as u32).to_be_bytes()[1..].to_vec();
        body.extend(entry);
        message(11, &body)
    }
    fn pin(leaf: &[u8]) -> String {
        format!("{:x}", Sha256::digest(leaf))
    }
    fn run_check(stream: &[u8], pin: &str) -> (Result<(), String>, usize) {
        let mut source = stream;
        let mut reader = Holding {
            inner: &mut source,
            held: Vec::new(),
        };
        let result = check(&mut reader, &mut Vec::new(), pin);
        (result, reader.held.len())
    }

    #[test]
    fn remembered_certificate_passes_and_only_the_handshake_is_held() {
        let leaf = b"leaf certificate".to_vec();
        let mut stream = CONFIRM_HYBRID.to_vec();
        // Renegotiation info extension, as Windows sends it.
        stream.extend(record(&[hello(&[0xFF, 1, 0, 1, 0])]));
        stream.extend(record(&[certificates(&leaf), message(14, &[])]));
        let after = stream.len();
        stream.extend(b"later data");
        let (result, held) = run_check(&stream, &pin(&leaf).to_uppercase());
        assert_eq!(result, Ok(()));
        assert_eq!(
            held,
            after - CONFIRM_HYBRID.len(),
            "holds the handshake up to the certificate record, no further"
        );
    }

    #[test]
    fn certificate_split_across_records_is_reassembled() {
        let leaf = vec![0xAB; 3000];
        let mut flight = hello(&[]);
        flight.extend(certificates(&leaf));
        let mut stream = CONFIRM_HYBRID.to_vec();
        for part in flight.chunks(700) {
            stream.extend(record(&[part.to_vec()]));
        }
        assert_eq!(run_check(&stream, &pin(&leaf)).0, Ok(()));
    }

    #[test]
    fn another_certificate_is_refused() {
        let mut stream = CONFIRM_HYBRID.to_vec();
        stream.extend(record(&[hello(&[]), certificates(b"attacker")]));
        let (result, _) = run_check(&stream, &pin(b"remembered"));
        assert_eq!(
            result.unwrap_err(),
            "The server's certificate is not the remembered one"
        );
    }

    #[test]
    fn negotiation_without_tls_is_refused() {
        let leaf = b"leaf";
        // RDP_NEG_RSP selecting PROTOCOL_RDP (0): standard RDP security.
        let mut standard = CONFIRM_HYBRID;
        standard[15] = 0;
        // No RDP_NEG_RSP at all.
        let legacy = [3u8, 0, 0, 11, 6, 0xD0, 0, 0, 0, 0, 0];
        // RDP_NEG_FAILURE.
        let mut failure = CONFIRM_HYBRID;
        failure[11] = 3;
        for confirm in [&standard[..], &legacy[..], &failure[..]] {
            let mut stream = confirm.to_vec();
            stream.extend(record(&[hello(&[]), certificates(leaf)]));
            assert_eq!(
                run_check(&stream, &pin(leaf)).0.unwrap_err(),
                "The server did not negotiate TLS"
            );
        }
    }

    #[test]
    fn tls13_resumption_and_unexpected_messages_are_refused() {
        let leaf = b"leaf";
        let tls13 = hello(&[0, 43, 0, 2, 3, 4]);
        let mut stream = CONFIRM_HYBRID.to_vec();
        stream.extend(record(&[tls13]));
        assert!(
            run_check(&stream, &pin(leaf))
                .0
                .unwrap_err()
                .contains("TLS 1.3")
        );

        // Abbreviated handshake: ServerHello, then ChangeCipherSpec.
        let mut stream = CONFIRM_HYBRID.to_vec();
        stream.extend(record(&[hello(&[])]));
        stream.extend([20, 3, 3, 0, 1, 1]);
        assert!(run_check(&stream, &pin(leaf)).0.is_err());

        // Certificate before ServerHello, or a key exchange instead of it.
        for flight in [
            vec![certificates(leaf), hello(&[])],
            vec![hello(&[]), message(12, &[1, 2, 3])],
        ] {
            let mut stream = CONFIRM_HYBRID.to_vec();
            stream.extend(record(&flight));
            assert!(run_check(&stream, &pin(leaf)).0.is_err());
        }

        // An alert record.
        let mut stream = CONFIRM_HYBRID.to_vec();
        stream.extend([21, 3, 3, 0, 2, 2, 40]);
        assert!(run_check(&stream, &pin(leaf)).0.is_err());
    }

    #[test]
    fn oversized_or_truncated_handshakes_are_refused() {
        let mut stream = CONFIRM_HYBRID.to_vec();
        // Endless handshake fragments that never complete a message.
        stream.extend(record(&[vec![2, 0xFF, 0xFF, 0xFF]]));
        for _ in 0..8 {
            stream.extend(record(&[vec![0; 16000]]));
        }
        assert!(
            run_check(&stream, "00")
                .0
                .unwrap_err()
                .contains("too large")
        );
        let mut stream = CONFIRM_HYBRID.to_vec();
        stream.extend(&record(&[hello(&[]), certificates(b"leaf")])[..30]);
        assert!(run_check(&stream, &pin(b"leaf")).0.is_err());
    }

    /// End to end over loopback: the client receives nothing from a server
    /// presenting another certificate, but everything once it matches.
    #[test]
    fn relay_holds_back_an_unexpected_server_and_passes_the_remembered_one() {
        fn serve(leaf: &'static [u8]) -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut request = [0u8; 19];
                s.read_exact(&mut request).unwrap();
                s.write_all(&CONFIRM_HYBRID).unwrap();
                // Like a real server: the handshake follows the ClientHello.
                let mut client_hello = [0u8; 5];
                if s.read_exact(&mut client_hello).is_err() {
                    return;
                }
                let mut flight = record(&[hello(&[]), certificates(leaf)]);
                flight.extend(b"after");
                let _ = s.write_all(&flight);
                let mut rest = Vec::new();
                let _ = s.read_to_end(&mut rest);
            });
            port
        }
        fn client(relay: &Relay) -> Vec<u8> {
            let mut c = TcpStream::connect(("127.0.0.1", relay.port())).unwrap();
            c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            c.write_all(&[0u8; 19]).unwrap();
            // Like the control: the ClientHello follows the confirm.
            let mut confirm = [0u8; 19];
            c.read_exact(&mut confirm).unwrap();
            assert_eq!(confirm, CONFIRM_HYBRID);
            c.write_all(&[22, 3, 1, 0, 0]).unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n @ 1..) = c.read(&mut buf) {
                seen.extend(&buf[..n]);
                if seen.ends_with(b"after") {
                    break;
                }
            }
            seen
        }
        let wrong = Relay::start("127.0.0.1".into(), serve(b"attacker"), pin(b"good")).unwrap();
        assert!(
            client(&wrong).is_empty(),
            "nothing of the handshake reaches the control"
        );
        assert_eq!(
            wrong.refusal().as_deref(),
            Some("The server's certificate is not the remembered one")
        );
        let right = Relay::start("127.0.0.1".into(), serve(b"good"), pin(b"good")).unwrap();
        assert!(client(&right).ends_with(b"after"));
        assert_eq!(right.refusal(), None);
        // A second connection finds nothing listening.
        assert!(TcpStream::connect(("127.0.0.1", right.port())).is_err());
    }
}
