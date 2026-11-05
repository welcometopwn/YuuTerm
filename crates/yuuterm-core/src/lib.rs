//! Protocol-independent metadata. Credentials and live transports do not belong here.
pub mod broadcast;
pub mod config;
mod gateway;
pub use gateway::*;
pub mod highlight_import;
pub mod instance;
pub mod mobaxterm;
pub mod session_actions;
pub mod session_icon;
pub mod session_transfer;
pub mod vnc;
pub mod wol;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    #[default]
    Ssh,
    Telnet,
    Rdp,
    Vnc,
    Serial,
    /// A local Windows shell in a pseudo console; no network.
    Local,
}

impl Protocol {
    /// The remote protocols (Local is a separate, local session type).
    pub const ALL: [Self; 5] = [Self::Ssh, Self::Telnet, Self::Rdp, Self::Vnc, Self::Serial];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ssh => "SSH",
            Self::Telnet => "TELNET",
            Self::Rdp => "RDP",
            Self::Vnc => "VNC",
            Self::Serial => "SERIAL",
            Self::Local => "LOCAL",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Ssh => 22,
            Self::Telnet => 23,
            Self::Rdp => 3389,
            Self::Vnc => 5900,
            Self::Serial | Self::Local => 0,
        }
    }
}

/// A bookmark ID must never be reused as a live connection incarnation ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    /// Sidebar group (folder); empty for ungrouped sessions.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Optional Ethernet MAC for explicit Wake-on-LAN requests.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mac_address: String,
    /// Quick connect: lives only in memory for this run; never written to the
    /// settings file or exported, and never stores a password.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub temporary: bool,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub font_size: Option<f32>,
    #[serde(default)]
    pub statistics_enabled: Option<bool>,
    #[serde(default)]
    pub logging: LoggingSettings,
    #[serde(default)]
    pub ssh: SshSettings,
    #[serde(default)]
    pub telnet: TelnetSettings,
    #[serde(default)]
    pub serial: SerialSettings,
    #[serde(default)]
    pub local: LocalSettings,
    #[serde(default)]
    pub rdp: RdpSettings,
    #[serde(default)]
    pub vnc: vnc::VncSettings,
    /// Local, non-recycled identity of this bookmark's saved password in the
    /// Windows Credential Manager. Never copied, exported or imported.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub credential_id: String,
    /// SSH gateways (jump hosts) in connection order; empty is direct.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateways: Vec<Gateway>,
}

/// Where remote audio plays. Values match the Microsoft control's
/// AudioRedirectionMode (0 local, 1 on the remote computer, 2 off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpAudio {
    Local,
    Remote,
    #[default]
    Off,
}

/// Where Windows key combinations (Win, Alt+Tab, ...) go while the remote
/// desktop has focus. Values match the control's KeyboardHookMode (1 remote,
/// 0 this computer).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpKeyboard {
    #[default]
    Remote,
    Local,
}

/// Deliberate per-session RDP channels; everything else stays off. The
/// clipboard (text and files, both directions) is on by default at the user's
/// request (18 October 2026), like mstsc.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RdpSettings {
    pub clipboard: bool,
    pub audio: RdpAudio,
    pub keyboard: RdpKeyboard,
}
impl Default for RdpSettings {
    fn default() -> Self {
        Self {
            clipboard: true,
            audio: RdpAudio::default(),
            keyboard: RdpKeyboard::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingSettings {
    pub enabled: bool,
    /// Empty selects the Windows Documents/YuuTerm/session directory.
    pub file: String,
    /// Explicit opt-in; None preserves timestamped defaults and the stop cap.
    #[serde(default)]
    pub rotation: Option<LogRotation>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRotation {
    pub max_mib: u32,
    /// Includes the active file.
    pub keep_files: u8,
}
impl LoggingSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .rotation
            .is_some_and(|r| !(1..=64).contains(&r.max_mib) || !(2..=10).contains(&r.keep_files))
        {
            return Err("Log rotation requires 1–64 MiB and 2–10 files including the active log.");
        }
        let file = &self.file;
        if file.is_empty() {
            return Ok(());
        }
        if file.len() > 1024
            || file.chars().any(char::is_control)
            || !std::path::Path::new(file).is_absolute()
            || file.starts_with(r"\\?\")
            || file.starts_with(r"\\.\")
            || file.ends_with(['\\', '/'])
            || file
                .chars()
                .enumerate()
                .any(|(i, c)| "<>\"|?*".contains(c) || (c == ':' && i != 1))
        {
            return Err(
                "Log file must be an absolute Windows file path without device names or control characters.",
            );
        }
        for part in file.split(['\\', '/']).filter(|p| !p.is_empty()).skip(1) {
            let base = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            if part.ends_with([' ', '.'])
                || matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ["COM", "LPT"].iter().any(|prefix| {
                    base.strip_prefix(prefix).is_some_and(|suffix| {
                        suffix.chars().count() == 1 && "123456789¹²³".contains(suffix)
                    })
                })
            {
                return Err("Choose a regular log filename, not a reserved Windows device name.");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialParity {
    #[default]
    None,
    Odd,
    Even,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialFlow {
    #[default]
    None,
    Software,
    Hardware,
}

/// Shell of a local terminal session. Only these fixed system shells run;
/// bookmarks never carry a command line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalShell {
    #[default]
    PowerShell,
    WindowsPowerShell,
    Cmd,
}

impl LocalShell {
    pub fn label(self) -> &'static str {
        match self {
            Self::PowerShell => "PowerShell 7",
            Self::WindowsPowerShell => "Windows PowerShell",
            Self::Cmd => "Command Prompt",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalSettings {
    pub shell: LocalShell,
    /// Absolute start directory; empty starts in the user profile folder.
    pub directory: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SerialSettings {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: SerialParity,
    pub stop_bits: u8,
    pub flow_control: SerialFlow,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TelnetSettings {
    /// Browser-local draft; only explicit submission enters the terminal engine.
    pub local_line_editing: bool,
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            baud_rate: 115200,
            data_bits: 8,
            parity: SerialParity::None,
            stop_bits: 1,
            flow_control: SerialFlow::None,
            dtr: false,
            rts: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshAuth {
    #[default]
    WindowsAgent,
    KeyFile,
    /// Secret is supplied separately at connection time, never in profile metadata.
    Password,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshSettings {
    /// Explicit opt-in; local X server credentials remain runtime-only.
    pub x11_forward: bool,
    pub auth: SshAuth,
    pub agent_pipe: String,
    pub identity_fingerprint: String,
    pub config_host: String,
    pub config_file: String,
    pub key_file: String,
}

impl Default for SshSettings {
    fn default() -> Self {
        Self {
            x11_forward: false,
            auth: SshAuth::WindowsAgent,
            agent_pipe: r"\\.\pipe\openssh-ssh-agent".into(),
            identity_fingerprint: String::new(),
            config_host: String::new(),
            config_file: String::new(),
            key_file: String::new(),
        }
    }
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            icon: String::new(),
            group: String::new(),
            mac_address: String::new(),
            temporary: false,
            protocol: Protocol::Ssh,
            host: String::new(),
            port: 22,
            username: String::new(),
            font_size: None,
            statistics_enabled: None,
            logging: LoggingSettings::default(),
            ssh: SshSettings::default(),
            telnet: TelnetSettings::default(),
            serial: SerialSettings::default(),
            local: LocalSettings::default(),
            rdp: RdpSettings::default(),
            vnc: vnc::VncSettings::default(),
            credential_id: String::new(),
            gateways: Vec::new(),
        }
    }
}

impl Profile {
    pub fn validate(&self) -> Result<(), &'static str> {
        session_icon::validate(&self.icon)?;
        if !self.mac_address.is_empty() {
            wol::parse_mac(&self.mac_address)?;
        }
        if !self.credential_id.is_empty() && !valid_credential_id(&self.credential_id) {
            return Err("Invalid saved-password identity.");
        }
        self.logging.validate()?;
        for value in [
            &self.ssh.agent_pipe,
            &self.ssh.key_file,
            &self.ssh.identity_fingerprint,
            &self.ssh.config_host,
            &self.ssh.config_file,
        ] {
            if value.len() > 4096 || value.chars().any(char::is_control) {
                return Err("SSH identity settings are too long or contain control characters.");
            }
        }
        if self.protocol == Protocol::Ssh
            && self.ssh.auth == SshAuth::Password
            && self.username.trim().is_empty()
        {
            return Err("SSH password authentication requires an explicit username.");
        }
        if self.ssh.config_host.len() > 253 || self.ssh.config_host.chars().any(char::is_whitespace)
        {
            return Err("OpenSSH identity host must be a single host entry of up to 253 bytes.");
        }
        if self.ssh.auth == SshAuth::WindowsAgent && !self.ssh.agent_pipe.starts_with(r"\\.\pipe\")
        {
            return Err(r"Use a local Windows agent pipe, for example \\.\pipe\openssh-ssh-agent.");
        }
        if self.group.len() > 64
            || self.group.chars().any(char::is_control)
            || self.group.trim() != self.group
        {
            return Err("Group names have up to 64 bytes, no control characters or outer spaces.");
        }
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err("Enter a session name (up to 128 bytes, no control characters).");
        }
        if self.protocol == Protocol::Local {
            let dir = &self.local.directory;
            let absolute = dir.len() >= 3
                && dir.as_bytes()[0].is_ascii_alphabetic()
                && dir[1..].starts_with(":\\");
            if dir.len() > 1024
                || dir.chars().any(char::is_control)
                || (!dir.is_empty() && !absolute)
            {
                return Err(r"Start directory must be empty or an absolute path such as C:\Work.");
            }
            if !self.host.is_empty() || self.ssh.x11_forward {
                return Err("A local terminal has no host or forwarding.");
            }
        } else {
            validate_host(&self.host)?;
        }
        if self.protocol == Protocol::Serial {
            let suffix = self
                .host
                .strip_prefix("COM")
                .ok_or("Use a COM device such as COM3.")?;
            if !suffix.bytes().all(|b| b.is_ascii_digit())
                || suffix.starts_with('0')
                || suffix.parse::<u16>().ok().filter(|n| *n > 0).is_none()
            {
                return Err("Use a COM device such as COM3.");
            }
        } else if self.port == 0 && self.protocol != Protocol::Local {
            return Err("Port must be between 1 and 65535.");
        }
        if !(50..=4_000_000).contains(&self.serial.baud_rate)
            || !(5..=8).contains(&self.serial.data_bits)
            || !(1..=2).contains(&self.serial.stop_bits)
        {
            return Err(
                "Serial settings require 50–4000000 baud, 5–8 data bits and 1 or 2 stop bits.",
            );
        }
        if self.username.len() > 256 || self.username.chars().any(char::is_control) {
            return Err("Username is too long or contains control characters.");
        }
        if self
            .font_size
            .is_some_and(|s| !s.is_finite() || !(8.0..=32.0).contains(&s))
        {
            return Err("Font size must be between 8 and 32.");
        }
        validate_gateways(self)
    }

    pub fn effective_font_size(&self, global: f32) -> f32 {
        self.font_size.unwrap_or(global)
    }
}

pub(crate) fn validate_host(host: &str) -> Result<(), &'static str> {
    if host.is_empty()
        || host.len() > 253
        || host.starts_with('-')
        || host.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("Enter a host, IP address or COM device without spaces or options.");
    }
    Ok(())
}

/// User name and identity fields of an SSH endpoint other than the target.
pub(crate) fn validate_ssh_identity(
    username: &str,
    auth: SshAuth,
    agent_pipe: &str,
    others: &[&str],
) -> Result<(), &'static str> {
    if username.len() > 256 || username.chars().any(char::is_control) {
        return Err("Username is too long or contains control characters.");
    }
    if std::iter::once(agent_pipe)
        .chain(others.iter().copied())
        .any(|v| v.len() > 4096 || v.chars().any(char::is_control))
    {
        return Err("SSH identity settings are too long or contain control characters.");
    }
    if auth == SshAuth::Password && username.trim().is_empty() {
        return Err("SSH password authentication requires an explicit username.");
    }
    if auth == SshAuth::WindowsAgent && !agent_pipe.starts_with(r"\\.\pipe\") {
        return Err(r"Use a local Windows agent pipe, for example \\.\pipe\openssh-ssh-agent.");
    }
    Ok(())
}

/// Lowercase UUID, not all zeros.
pub fn valid_credential_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
            }
        })
        && value.bytes().any(|b| b != b'-' && b != b'0')
}

/// Deny by default. Call `remove` on disconnect, before allocating a new live ID.
#[derive(Default, Debug)]
pub struct BroadcastTargets(std::collections::BTreeSet<u64>);

impl BroadcastTargets {
    pub fn select(&mut self, live_id: u64) {
        self.0.insert(live_id);
    }
    pub fn remove(&mut self, live_id: u64) {
        self.0.remove(&live_id);
    }
    pub fn stop(&mut self) {
        self.0.clear();
    }
    pub fn recipients(&self, connected_terminal_ids: &[u64]) -> Vec<u64> {
        self.0
            .iter()
            .copied()
            .filter(|id| connected_terminal_ids.contains(id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            name: "Server".into(),
            host: "server.example".into(),
            ..Default::default()
        }
    }

    #[test]
    fn group_names_are_bounded_and_trimmed() {
        let mut p = Profile {
            name: "Grouped".into(),
            host: "host.test".into(),
            ..Default::default()
        };
        for good in ["", "Development", "Kunden / Wien"] {
            p.group = good.into();
            assert!(p.validate().is_ok(), "{good:?}");
        }
        for bad in [" padded", "tab	name", &"g".repeat(65)] {
            p.group = bad.into();
            assert!(p.validate().is_err(), "{bad:?}");
        }
        let old: Profile = serde_json::from_str(
            r#"{"id":1,"name":"Old","protocol":"Ssh","host":"h","port":22,"username":"","font_size":null}"#,
        )
        .unwrap();
        assert_eq!(old.group, "", "settings without groups still load");
    }
    #[test]
    fn validate_metadata_and_overrides() {
        let mut p = profile();
        assert!(p.validate().is_ok());
        assert_eq!(p.effective_font_size(14.0), 14.0);
        p.font_size = Some(16.0);
        assert_eq!(p.effective_font_size(14.0), 16.0);
        for host in ["-oProxyCommand=cmd", "host\ncommand", "", "host name"] {
            p.host = host.into();
            assert!(p.validate().is_err());
        }
    }

    #[test]
    fn ports_serial_and_nonfinite_font_are_checked() {
        let mut p = profile();
        p.port = 0;
        assert!(p.validate().is_err());
        p.protocol = Protocol::Serial;
        p.host = "COM3".into();
        assert!(p.validate().is_ok());
        p.host = "COM0".into();
        assert!(p.validate().is_err());
        for bad in ["COM+3", "COM03", "COM65536", "COM", r"\\.\COM3"] {
            p.host = bad.into();
            assert!(p.validate().is_err());
        }
        p.host = "COM3".into();
        p.font_size = Some(f32::NAN);
        assert!(p.validate().is_err());
    }

    #[test]
    fn metadata_roundtrip_rejects_secret_fields() {
        let p = profile();
        let mut json = serde_json::to_value(&p).unwrap();
        json.as_object_mut().unwrap().remove("ssh");
        let old: Profile = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.ssh.auth, SshAuth::WindowsAgent);
        assert!(!old.ssh.x11_forward);
        assert!(serde_json::from_value::<Profile>(json.clone()).is_ok());
        json["password"] = "must not be saved here".into();
        assert!(serde_json::from_value::<Profile>(json).is_err());
    }

    #[test]
    fn serial_defaults_preserve_old_profiles_and_reject_invalid_parameters() {
        let mut json = serde_json::to_value(profile()).unwrap();
        json.as_object_mut().unwrap().remove("serial");
        let mut old: Profile = serde_json::from_value(json).unwrap();
        assert_eq!(old.serial.baud_rate, 115200);
        assert!(!old.serial.dtr && !old.serial.rts);
        old.serial.baud_rate = 0;
        assert!(old.validate().is_err());
        old.serial.baud_rate = 9600;
        old.serial.data_bits = 9;
        assert!(old.validate().is_err());
        old.serial.data_bits = 7;
        old.serial.stop_bits = 3;
        assert!(old.validate().is_err());
        old.serial.stop_bits = 2;
        assert!(old.validate().is_ok());
    }

    #[test]
    fn broadcast_never_enrolls_new_or_reconnected_sessions() {
        let mut b = BroadcastTargets::default();
        assert!(b.recipients(&[1, 2]).is_empty());
        b.select(1);
        assert_eq!(b.recipients(&[1, 2, 3]), vec![1]);
        b.remove(1);
        assert!(b.recipients(&[2, 3, 4]).is_empty());
        b.select(2);
        assert!(b.recipients(&[3]).is_empty());
        b.stop();
        assert!(b.recipients(&[2]).is_empty());
    }
}
