//! Independent loopback witness for the RDP certificate pre-read deadline.
//! Exit 2 means the nominal timeout was exceeded by more than scheduling slack.
#[path = "../src/rdp_cert.rs"]
pub mod rdp_cert;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

fn main() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 19];
        stream.read_exact(&mut request).unwrap();
        // Complete TPKT body, with a refusal instead of TLS. Every byte arrives
        // inside the per-read timeout, but the complete operation exceeds it.
        let mut response = [0u8; 19];
        response[..4].copy_from_slice(&[3, 0, 0, 19]);
        response[11] = 3;
        for byte in response {
            std::thread::sleep(Duration::from_millis(20));
            if stream.write_all(&[byte]).is_err() {
                break;
            }
        }
    });
    let budget = Duration::from_millis(80);
    let started = Instant::now();
    let refused = rdp_cert::fingerprint("127.0.0.1", port, budget).is_err();
    let elapsed = started.elapsed();
    peer.join().unwrap();
    let violated = elapsed > budget * 2;
    println!(
        "{}",
        serde_json::json!({"fixture":"loopback trickled RDP negotiation refusal; no TLS or credentials", "timeout_ms":budget.as_millis(), "elapsed_ms":elapsed.as_millis(), "refused":refused, "whole_operation_deadline_violated":violated})
    );
    if violated {
        std::process::exit(2);
    }
}
