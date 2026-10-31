//! Windows COM I/O runs on a blocking worker, never a Tokio/render thread.
use crate::{Command, Snapshot, Wake, terminal::Terminal};
use anyhow::{Context, Result};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use yuuterm_core::{Profile, SerialFlow, SerialParity};

pub async fn run(
    profile: Profile,
    commands: mpsc::Receiver<Command>,
    state: watch::Sender<Snapshot>,
    wake: Wake,
    output: Option<mpsc::Sender<Vec<u8>>>,
    cancel: CancellationToken,
) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let s = &profile.serial;
        let mut port = serialport::new(&profile.host, s.baud_rate)
            .data_bits(match s.data_bits {
                5 => serialport::DataBits::Five,
                6 => serialport::DataBits::Six,
                7 => serialport::DataBits::Seven,
                _ => serialport::DataBits::Eight,
            })
            .parity(match s.parity {
                SerialParity::None => serialport::Parity::None,
                SerialParity::Odd => serialport::Parity::Odd,
                SerialParity::Even => serialport::Parity::Even,
            })
            .stop_bits(if s.stop_bits == 2 {
                serialport::StopBits::Two
            } else {
                serialport::StopBits::One
            })
            .flow_control(match s.flow_control {
                SerialFlow::None => serialport::FlowControl::None,
                SerialFlow::Software => serialport::FlowControl::Software,
                SerialFlow::Hardware => serialport::FlowControl::Hardware,
            })
            .timeout(Duration::from_millis(100))
            .dtr_on_open(s.dtr)
            .open()
            .context("Could not open serial device")?;
        if cancel.is_cancelled() {
            return Ok(());
        }
        // Hardware flow control owns RTS. Never override the driver's handshake.
        // The builder sets DTR only best-effort; require success explicitly.
        port.write_data_terminal_ready(s.dtr)
            .context("Could not set DTR")?;
        if s.flow_control != SerialFlow::Hardware {
            port.write_request_to_send(s.rts)
                .context("Could not set RTS")?;
        }
        state.send_modify(|v| {
            // A cancelled async owner can finish while a driver call returns.
            // Never resurrect its snapshot from this blocking worker.
            if !v.finished && !cancel.is_cancelled() {
                v.connected = true;
                v.status = format!("SERIAL · {} baud", s.baud_rate);
            }
        });
        wake();
        pump(port.as_mut(), commands, state, wake, output, cancel)
    })
    .await
    .context("Serial worker failed")?
}

fn pump<P: Read + Write + ?Sized>(
    port: &mut P,
    mut commands: mpsc::Receiver<Command>,
    state: watch::Sender<Snapshot>,
    wake: Wake,
    output: Option<mpsc::Sender<Vec<u8>>>,
    cancel: CancellationToken,
) -> Result<()> {
    let mut terminal = Terminal::new();
    let mut buffer = [0; 16384];
    while !cancel.is_cancelled() {
        // One command per read bounds input/output starvation. Each OS read/write
        // has a 100 ms timeout; cancellation is checked between calls.
        match commands.try_recv() {
            Ok(Command::Input(bytes)) => {
                let end = Instant::now() + Duration::from_secs(2);
                let mut rest = bytes.as_slice();
                while !rest.is_empty() && !cancel.is_cancelled() {
                    anyhow::ensure!(
                        Instant::now() < end,
                        "Serial write timed out; input may have been partially sent"
                    );
                    match port.write(rest) {
                        Ok(0) => anyhow::bail!(
                            "Serial write returned zero; input may have been partially sent"
                        ),
                        Ok(n) => rest = &rest[n..],
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::TimedOut
                                    | std::io::ErrorKind::WouldBlock
                                    | std::io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => {
                            return Err(e).context(
                                "Serial write failed; input may have been partially sent",
                            );
                        }
                    }
                }
            }
            Ok(Command::Resize { rows, cols }) => {
                terminal.resize(rows, cols);
            }
            Ok(Command::Scroll(delta)) if output.is_none() => {
                terminal.scroll(delta);
                state.send_modify(|s| s.terminal = terminal.view());
                wake();
            }
            Ok(_) | Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
        if cancel.is_cancelled() {
            break;
        }
        match port.read(&mut buffer) {
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(n) => {
                if let Some(output) = &output {
                    let mut bytes = buffer[..n].to_vec();
                    loop {
                        if cancel.is_cancelled() {
                            return Ok(());
                        }
                        match output.try_send(bytes) {
                            Ok(()) => break,
                            Err(mpsc::error::TrySendError::Full(b)) => {
                                bytes = b;
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                        }
                    }
                    wake();
                } else {
                    terminal.process(&buffer[..n]);
                    state.send_modify(|s| s.terminal = terminal.view());
                    wake();
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e).context("Serial read failed"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    struct Device {
        input: Vec<u8>,
        reads: usize,
    }
    impl Read for Device {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.reads > 3 {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            bytes[..4].copy_from_slice(&[0, 13, 255, 10]);
            Ok(4)
        }
    }
    impl Write for Device {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let n = bytes.len().min(2);
            self.input.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn state() -> watch::Sender<Snapshot> {
        watch::channel(Snapshot {
            #[cfg(feature = "startup-probe")]
            startup: Default::default(),
            protocol: yuuterm_core::Protocol::Serial,
            endpoint: "COM7".into(),
            status: String::new(),
            connected: true,
            finished: false,
            password_rejected: false,
            trust: None,
            terminal: Terminal::new().view(),
            path: String::new(),
            files: Arc::new(Vec::new()),
            files_status: "Unavailable".into(),
        })
        .0
    }
    #[test]
    fn serial_worker_preserves_raw_bytes_partial_writes_and_reports_removal() {
        let (tx, rx) = mpsc::channel(32);
        let (out, mut reader) = mpsc::channel(32);
        tx.try_send(Command::Input(vec![0, 255, 13, 10, 3]))
            .unwrap();
        tx.try_send(Command::Resize {
            rows: 30,
            cols: 120,
        })
        .unwrap();
        let mut device = Device {
            input: vec![],
            reads: 0,
        };
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let signal = wakes.clone();
        assert!(
            pump(
                &mut device,
                rx,
                state(),
                Arc::new(move || {
                    signal.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }),
                Some(out),
                CancellationToken::new()
            )
            .is_err()
        );
        assert_eq!(device.input, [0, 255, 13, 10, 3]);
        let mut actual = Vec::new();
        while let Ok(bytes) = reader.try_recv() {
            actual.extend(bytes);
        }
        assert_eq!(actual, [0, 13, 255, 10].repeat(3));
        assert_eq!(
            wakes.load(std::sync::atomic::Ordering::Relaxed),
            3,
            "Every queued serial read must wake a waiting renderer"
        );
    }
    #[test]
    fn cancellation_releases_worker_when_renderer_is_full() {
        let (tx, rx) = mpsc::channel(32);
        let (out, _reader) = mpsc::channel(1);
        out.try_send(vec![1]).unwrap();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let (finished, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut device = Device {
                input: vec![],
                reads: 0,
            };
            let r = pump(&mut device, rx, state(), Arc::new(|| {}), Some(out), token);
            finished.send((r.is_ok(), device.reads)).unwrap();
        });
        std::thread::sleep(Duration::from_millis(30));
        cancel.cancel();
        let (ok, reads) = result.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(ok);
        assert!(reads <= 1);
        worker.join().unwrap();
        drop(tx);
    }
}
