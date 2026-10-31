//! Bounded character-mode TELNET transport; no encryption, authentication or SFTP.
use crate::{Command, Snapshot, Wake, terminal::Terminal};
use anyhow::{Context, Result, ensure};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, watch},
};
use yuuterm_core::Profile;

const IAC: u8 = 255;
const WILL: u8 = 251;
const WONT: u8 = 252;
const DO: u8 = 253;
const DONT: u8 = 254;
const SB: u8 = 250;
const SE: u8 = 240;
const BINARY: u8 = 0;
const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;
#[derive(Clone, Copy)]
enum State {
    Data,
    Iac,
    Option(u8),
    SubOption,
    SubData,
    SubIac,
}
pub(crate) struct Codec {
    state: State,
    remote: [bool; 256],
    local: [bool; 256],
    sub_option: u8,
    sub: Vec<u8>,
    pending_cr: bool,
    sent_cr: bool,
    echo_cr: bool,
    rows: u16,
    cols: u16,
    window: Instant,
    controls: usize,
}
impl Default for Codec {
    fn default() -> Self {
        Self {
            state: State::Data,
            remote: [false; 256],
            local: [false; 256],
            sub_option: 0,
            sub: Vec::new(),
            pending_cr: false,
            sent_cr: false,
            echo_cr: false,
            rows: 24,
            cols: 80,
            window: Instant::now(),
            controls: 0,
        }
    }
}
fn escaped(bytes: &[u8], out: &mut Vec<u8>) {
    for &byte in bytes {
        out.push(byte);
        if byte == IAC {
            out.push(IAC);
        }
    }
}
impl Codec {
    fn control(&mut self) -> Result<()> {
        if self.window.elapsed() >= Duration::from_secs(1) {
            self.window = Instant::now();
            self.controls = 0;
        }
        self.controls += 1;
        ensure!(
            self.controls <= 512,
            "TELNET negotiation rate exceeds the 512/s limit"
        );
        Ok(())
    }
    fn data(&mut self, byte: u8, out: &mut Vec<u8>) {
        if self.pending_cr {
            self.pending_cr = false;
            out.push(b'\r');
            if byte == 0 && !self.remote[BINARY as usize] {
                return;
            }
        }
        if byte == b'\r' && !self.remote[BINARY as usize] {
            self.pending_cr = true;
        } else {
            out.push(byte);
        }
    }
    pub fn finish(&mut self) -> Vec<u8> {
        if std::mem::take(&mut self.pending_cr) {
            vec![b'\r']
        } else {
            vec![]
        }
    }
    fn window_size(&self) -> Vec<u8> {
        let mut out = vec![IAC, SB, NAWS];
        escaped(&self.cols.to_be_bytes(), &mut out);
        escaped(&self.rows.to_be_bytes(), &mut out);
        out.extend([IAC, SE]);
        out
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Vec<u8> {
        self.rows = rows;
        self.cols = cols;
        if self.local[NAWS as usize] {
            self.window_size()
        } else {
            vec![]
        }
    }
    pub fn local_echo(&self) -> bool {
        !self.remote[ECHO as usize]
    }
    fn echo(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len() * 2);
        for &byte in bytes {
            if byte == b'\n' && self.echo_cr {
                self.echo_cr = false;
                continue;
            }
            self.echo_cr = byte == b'\r';
            if byte == b'\r' || byte == b'\n' {
                out.extend(b"\r\n");
            } else {
                out.push(byte);
            }
        }
        out
    }
    pub fn encode(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len() * 2);
        for &byte in bytes {
            if self.local[BINARY as usize] {
                self.sent_cr = false;
                escaped(&[byte], &mut out);
                continue;
            }
            if byte == b'\n' && self.sent_cr {
                self.sent_cr = false;
                continue;
            }
            self.sent_cr = byte == b'\r';
            if byte == b'\r' || byte == b'\n' {
                out.extend(b"\r\n");
            } else {
                escaped(&[byte], &mut out);
            }
        }
        out
    }
    pub fn decode(&mut self, bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut out = Vec::with_capacity(bytes.len());
        let mut reply = Vec::new();
        for &byte in bytes {
            match self.state {
                State::Data => {
                    if byte == IAC {
                        self.state = State::Iac;
                    } else {
                        self.data(byte, &mut out);
                    }
                }
                State::Iac => {
                    self.state = match byte {
                        IAC => {
                            self.data(IAC, &mut out);
                            State::Data
                        }
                        WILL | WONT | DO | DONT => State::Option(byte),
                        SB => State::SubOption,
                        _ => {
                            self.control()?;
                            State::Data
                        }
                    };
                }
                State::Option(command) => {
                    self.control()?;
                    let index = usize::from(byte);
                    match command {
                        WILL => {
                            if matches!(byte, BINARY | ECHO | SGA) {
                                if !self.remote[index] {
                                    if byte == BINARY {
                                        out.extend(self.finish());
                                    }
                                    self.remote[index] = true;
                                    if byte == ECHO {
                                        self.echo_cr = false;
                                    }
                                    reply.extend([IAC, DO, byte]);
                                }
                            } else {
                                reply.extend([IAC, DONT, byte]);
                            }
                        }
                        WONT => {
                            if self.remote[index] {
                                self.remote[index] = false;
                                if byte == ECHO {
                                    self.echo_cr = false;
                                }
                                reply.extend([IAC, DONT, byte]);
                            }
                        }
                        DO => {
                            if matches!(byte, BINARY | SGA | TTYPE | NAWS) {
                                if !self.local[index] {
                                    self.local[index] = true;
                                    if byte == BINARY {
                                        self.sent_cr = false;
                                    }
                                    reply.extend([IAC, WILL, byte]);
                                    if byte == NAWS {
                                        reply.extend(self.window_size());
                                    }
                                }
                            } else {
                                reply.extend([IAC, WONT, byte]);
                            }
                        }
                        DONT => {
                            if self.local[index] {
                                self.local[index] = false;
                                if byte == BINARY {
                                    self.sent_cr = false;
                                }
                                reply.extend([IAC, WONT, byte]);
                            }
                        }
                        _ => unreachable!(),
                    }
                    self.state = State::Data;
                }
                State::SubOption => {
                    self.sub_option = byte;
                    self.sub.clear();
                    self.state = State::SubData;
                }
                State::SubData => {
                    if byte == IAC {
                        self.state = State::SubIac;
                    } else {
                        ensure!(
                            self.sub.len() < 512,
                            "TELNET subnegotiation exceeds 512 bytes"
                        );
                        self.sub.push(byte);
                    }
                }
                State::SubIac => match byte {
                    SE => {
                        self.control()?;
                        if self.sub_option == TTYPE && self.local[TTYPE as usize] && self.sub == [1]
                        {
                            reply.extend([IAC, SB, TTYPE, 0]);
                            reply.extend(b"XTERM-256COLOR");
                            reply.extend([IAC, SE]);
                        }
                        self.sub.clear();
                        self.state = State::Data;
                    }
                    IAC => {
                        ensure!(
                            self.sub.len() < 512,
                            "TELNET subnegotiation exceeds 512 bytes"
                        );
                        self.sub.push(IAC);
                        self.state = State::SubData;
                    }
                    _ => anyhow::bail!("Malformed TELNET subnegotiation"),
                },
            }
        }
        Ok((out, reply))
    }
}

pub(crate) async fn run(
    profile: Profile,
    mut commands: mpsc::Receiver<Command>,
    state: &watch::Sender<Snapshot>,
    wake: Wake,
    output: Option<mpsc::Sender<Vec<u8>>>,
) -> Result<()> {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(10),
        TcpStream::connect((profile.host.as_str(), profile.port)),
    )
    .await
    .context("TELNET connection timed out")??;
    stream.set_nodelay(true)?;
    let mut codec = Codec::default();
    let mut terminal = Terminal::new();
    let mut data = [0u8; 16384];
    state.send_modify(|s| {
        s.connected = true;
        s.status = "TELNET · Unencrypted connection".into();
        s.files_status = "File browsing is unavailable for TELNET".into();
    });
    wake();
    loop {
        tokio::select! {
            count=stream.read(&mut data)=>{
                let count=count?;
                if count==0 {
                    publish(codec.finish(),&output,&mut terminal,state,&wake).await?;
                    break;
                }
                let (text,reply)=codec.decode(&data[..count])?;
                if !reply.is_empty() { tokio::time::timeout(Duration::from_secs(10),stream.write_all(&reply)).await.context("TELNET negotiation write timed out")??; }
                publish(text,&output,&mut terminal,state,&wake).await?;
            },
            command=commands.recv()=>match command {
                Some(Command::Input(bytes))=>{
                    let encoded=codec.encode(&bytes);
                    tokio::time::timeout(Duration::from_secs(10),stream.write_all(&encoded)).await.context("TELNET input write timed out")??;
                    if codec.local_echo() { publish(codec.echo(&bytes),&output,&mut terminal,state,&wake).await?; }
                },
                Some(Command::Resize{rows,cols})=>{
                    let (rows,cols)=crate::terminal::dimensions(rows,cols);
                    terminal.resize(rows,cols);
                    let reply=codec.resize(rows,cols);
                    if !reply.is_empty() { tokio::time::timeout(Duration::from_secs(10),stream.write_all(&reply)).await.context("TELNET resize write timed out")??; }
                },
                Some(Command::Scroll(delta))=>{if output.is_none(){terminal.scroll(delta);state.send_modify(|s|s.terminal=terminal.view());wake();}},
                Some(Command::Browse(_))=>{state.send_modify(|s|s.files_status="File browsing is unavailable for TELNET".into());wake();},
                Some(Command::Processes(reply))=>{let _=reply.send(Err("Host processes require SSH".into()));},
                Some(Command::Statistics(reply))=>{let _=reply.send(Err("Host statistics require SSH".into()));},
                Some(Command::ToolsProbe(reply))=>{let _=reply.send(Err("Remote tools require SSH".into()));},
                Some(Command::FolderSizes{reply,..})=>{let _=reply.send(Err("Folder sizes require SSH".into()));},
                Some(Command::CancelFolderSizes)=>{},
                None=>break,
            }
        }
    }
    Ok(())
}
async fn publish(
    bytes: Vec<u8>,
    output: &Option<mpsc::Sender<Vec<u8>>>,
    terminal: &mut Terminal,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    if let Some(output) = output {
        output
            .send(bytes)
            .await
            .context("Terminal renderer closed")?;
        wake();
    } else {
        terminal.process(&bytes);
        state.send_modify(|s| s.terminal = terminal.view());
        wake();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_options_nvt_binary_and_literal_iac() {
        let mut c = Codec::default();
        assert_eq!(c.echo(b"local\r"), b"local\r\n");
        assert!(c.echo(b"\n").is_empty());
        let mut out = Vec::new();
        let mut replies = Vec::new();
        for b in b"one\r\0two\r\n\xff\xfb\x01\xff\xfd\x18\xff\xfa\x18\x01\xff\xf0\xff\xff" {
            let (text, reply) = c.decode(&[*b]).unwrap();
            out.extend(text);
            replies.extend(reply);
        }
        assert_eq!(out, b"one\rtwo\r\n\xff");
        assert!(!c.local_echo());
        assert_eq!(
            replies,
            [
                vec![IAC, DO, ECHO, IAC, WILL, TTYPE, IAC, SB, TTYPE, 0],
                b"XTERM-256COLOR".to_vec(),
                vec![IAC, SE]
            ]
            .concat()
        );
        assert!(c.decode(&[IAC, WILL, ECHO]).unwrap().1.is_empty());
        assert_eq!(c.encode(b"a\rb\n"), b"a\r\nb\r\n");
        assert_eq!(c.encode(b"\r"), b"\r\n");
        assert!(c.encode(b"\n").is_empty());
        c.decode(&[IAC, DO, BINARY, IAC, WILL, BINARY]).unwrap();
        assert_eq!(c.encode(&[13, 0, 255]), [13, 0, 255, 255]);
        assert_eq!(c.decode(&[13, 0, 255, 255]).unwrap().0, [13, 0, 255]);
    }
    #[test]
    fn naws_refusals_subnegotiation_and_control_flood_are_bounded() {
        let mut c = Codec::default();
        assert!(c.resize(255, 511).is_empty());
        assert_eq!(
            c.decode(&[IAC, DO, NAWS]).unwrap().1,
            [
                IAC, WILL, NAWS, IAC, SB, NAWS, 1, 255, 255, 0, 255, 255, IAC, SE
            ]
        );
        assert_eq!(
            c.decode(&[IAC, WILL, 99, IAC, DO, ECHO]).unwrap().1,
            [IAC, DONT, 99, IAC, WONT, ECHO]
        );
        c.decode(&[IAC, SB, 99]).unwrap();
        assert!(c.decode(&vec![1; 513]).is_err());
        let mut c = Codec::default();
        let flood = [IAC, DO, 99].repeat(513);
        assert!(c.decode(&flood).is_err());
    }
}
