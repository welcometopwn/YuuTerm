use crate::{Profile, Protocol, SerialFlow, SerialParity, session_transfer::MAX_BYTES};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Skipped {
    pub line: usize,
    pub reason: &'static str,
}
#[derive(Debug, Serialize)]
pub struct Import {
    pub format: &'static str,
    pub profiles: Vec<Profile>,
    pub skipped: Vec<Skipped>,
}

pub fn parse(bytes: &[u8]) -> Result<Import, String> {
    parse_with_icons(bytes, &std::collections::BTreeMap::new())
}

pub fn parse_with_icons(
    bytes: &[u8],
    icons: &std::collections::BTreeMap<u16, String>,
) -> Result<Import, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Session import exceeds 1 MiB".into());
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) {
        if !bytes.len().is_multiple_of(2) {
            return Err("Truncated UTF-16 session file".into());
        }
        String::from_utf16(
            &bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| "Invalid UTF-16 session file")?
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => s.trim_start_matches('\u{feff}').to_owned(),
            Err(_) => {
                let (s, errors) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
                if errors {
                    return Err("Invalid session text encoding".into());
                }
                s.into_owned()
            }
        }
    };
    if text.contains('\0') {
        return Err("Session file contains NUL bytes".into());
    }
    let mut result = Import {
        format: "MobaXterm",
        profiles: Vec::new(),
        skipped: Vec::new(),
    };
    // A section's SubRep may follow its bookmarks. Stage at most 1000 parsed
    // entries until all section-local metadata has been validated.
    let mut folders: Vec<Option<Result<String, &'static str>>> = Vec::new();
    let mut section = None;
    let mut entries = Vec::new();
    let mut count = 0;
    for (index, raw) in text.lines().enumerate() {
        if index >= 20_000 || raw.len() > 65_536 {
            return Err("Session file exceeds line limits".into());
        }
        let line = raw.trim();
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return Err("Malformed INI section header".into());
            }
            let bookmarks = line
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .is_some_and(|s| {
                    s == "Bookmarks"
                        || s.strip_prefix("Bookmarks_")
                            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                });
            section = bookmarks.then(|| {
                folders.push(None);
                folders.len() - 1
            });
            continue;
        }
        let Some(section) = section else { continue };
        if line.is_empty() || (line.starts_with([';', '#']) && !line.contains('=')) {
            continue;
        }
        let pair = line.split_once('=');
        if let Some((key, value)) = pair {
            match key.trim() {
                "SubRep" => {
                    folders[section] = Some(if folders[section].is_some() {
                        Err("Duplicate bookmark folder metadata")
                    } else {
                        folder(value.trim())
                    });
                    continue;
                }
                "ImgNum" => continue,
                _ => {}
            }
        }
        count += 1;
        if count > 1000 {
            return Err("MobaXterm import exceeds 1000 entries".into());
        }
        let parsed = pair
            .ok_or("Malformed bookmark entry")
            .and_then(|(name, value)| bookmark(name.trim(), value.trim(), icons));
        entries.push((index + 1, count, section, parsed));
    }
    if folders.is_empty() || count == 0 {
        return Err("No MobaXterm bookmark entries found".into());
    }
    for (line, id, section, parsed) in entries {
        let group = folders[section]
            .clone()
            .unwrap_or_else(|| Ok(String::new()));
        match parsed.and_then(|mut p| {
            p.group = group?;
            Ok(p)
        }) {
            Ok(mut p) => {
                p.id = id;
                result.profiles.push(p);
            }
            Err(reason) => result.skipped.push(Skipped { line, reason }),
        }
    }
    Ok(result)
}

fn folder(value: &str) -> Result<String, &'static str> {
    if value.chars().any(char::is_control) {
        return Err("Invalid bookmark folder control character");
    }
    let mut group = String::new();
    for component in value.split('\\').map(str::trim).filter(|s| !s.is_empty()) {
        let separator = if group.is_empty() { "" } else { " / " };
        if group.len() + separator.len() + component.len() > 64 {
            return Err("Bookmark folder exceeds 64 UTF-8 bytes after joining");
        }
        group.push_str(separator);
        group.push_str(component);
    }
    Ok(group)
}

fn bookmark(
    name: &str,
    value: &str,
    icons: &std::collections::BTreeMap<u16, String>,
) -> Result<Profile, &'static str> {
    let mut groups = value.split('#');
    groups.next().ok_or("Missing bookmark fields")?; // logout/script text: deliberately discarded
    // Icons are optional metadata, never the protocol discriminator. Retain
    // bounded embedded PNGs; numeric IDs and unusable images use the default.
    // Still require the slot so protocol fields cannot shift into its place.
    let icon = groups.next().ok_or("Missing bookmark icon")?;
    let fields: Vec<_> = groups
        .next()
        .ok_or("Missing protocol fields")?
        .split('%')
        .collect();
    let get = |n: usize| fields.get(n).copied().ok_or("Truncated protocol fields");
    let protocol = match get(0)? {
        "0" => Protocol::Ssh,
        "1" => Protocol::Telnet,
        "4" => Protocol::Rdp,
        "5" => Protocol::Vnc,
        "8" => Protocol::Serial,
        _ => return Err("Unsupported MobaXterm session type"),
    };
    let mut p = Profile {
        name: name.into(),
        protocol,
        ..Default::default()
    };
    if let Some(png) = icon
        .strip_prefix("b64:")
        .filter(|s| s.len() <= 11000 - "data:image/png;base64,".len())
    {
        let uri = format!("data:image/png;base64,{png}");
        if crate::session_icon::validate(&uri).is_ok() {
            p.icon = uri;
        }
    } else if let Some(uri) = icon.parse::<u16>().ok().and_then(|n| icons.get(&n))
        && crate::session_icon::validate(uri).is_ok()
    {
        p.icon = uri.clone();
    }
    if protocol == Protocol::Serial {
        // Field 1 is a device-list index, not a COM number. The explicit device
        // label in field 8 is the only portable identifier. Do not guess it.
        let label = get(8)?;
        let device = label.split_once(' ').map_or(label, |(s, _)| s);
        p.host = device.to_ascii_uppercase();
        p.port = 0;
        p.serial.baud_rate = get(2)?.parse().map_err(|_| "Invalid serial baud rate")?;
        p.serial.data_bits = match get(3)? {
            "0" => 5,
            "1" => 6,
            "2" => 7,
            "3" => 8,
            _ => return Err("Unsupported serial data-bit index"),
        };
        p.serial.stop_bits = match get(4)? {
            "0" => 1,
            "1" => 2,
            _ => return Err("Unsupported serial stop-bit index"),
        };
        p.serial.parity = match get(5)? {
            "0" => SerialParity::None,
            "1" => SerialParity::Odd,
            "2" => SerialParity::Even,
            _ => return Err("Unsupported serial parity (Mark/Space are not supported)"),
        };
        p.serial.flow_control = match get(6)? {
            "0" => SerialFlow::None,
            "1" => SerialFlow::Software,
            "2" => SerialFlow::Hardware,
            _ => return Err("Unsupported serial flow control (Dsr/Dtr is not supported)"),
        };
    } else {
        p.host = get(1)?.into();
        if p.host
            .chars()
            .any(|c| !c.is_alphanumeric() && !".-_:[]".contains(c))
        {
            return Err("Invalid network host");
        }
        p.port = get(2)?.parse().map_err(|_| "Invalid network port")?;
        if protocol != Protocol::Vnc {
            p.username = match get(3)? {
                "<default>" => String::new(),
                s => s.into(),
            };
        }
    }
    p.gateways = import_gateways(protocol, &fields)?;
    p.validate()
        .map_err(|_| "Invalid session name, endpoint or settings")?;
    Ok(p)
}

/// Protocol-specific metadata slots, not commands or credential sections.
fn import_gateways(
    protocol: Protocol,
    fields: &[&str],
) -> Result<Vec<crate::Gateway>, &'static str> {
    let (host, key) = match protocol {
        Protocol::Ssh => (8, 15),
        Protocol::Telnet => (6, 9),
        Protocol::Rdp => (13, 18),
        Protocol::Vnc => (5, 8),
        _ => return Ok(Vec::new()),
    };
    let hosts = fields.get(host).copied().unwrap_or("");
    if hosts.is_empty() {
        if [host + 1, host + 2, key]
            .iter()
            .any(|i| fields.get(*i).is_some_and(|s| !s.is_empty()))
        {
            return Err("Gateway fields have no gateway host");
        }
        return Ok(Vec::new());
    }
    let hosts: Vec<_> = hosts.split("__PIPE__").collect();
    if hosts.len() > crate::MAX_GATEWAYS {
        return Err("Use up to 4 SSH gateways");
    }
    let list = |index: usize| -> Result<Vec<&str>, &'static str> {
        let value = fields.get(index).ok_or("Truncated SSH gateway fields")?;
        let values: Vec<_> = value.split("__PIPE__").collect();
        if values.len() != hosts.len() {
            return Err("Inconsistent SSH gateway lists");
        }
        Ok(values)
    };
    let ports = list(host + 1)?;
    let users = list(host + 2)?;
    let keys = list(key)?;
    hosts
        .iter()
        .enumerate()
        .map(|(i, host)| {
            // A drive-relative placeholder cannot be resolved portably on import.
            if keys[i].contains("_CurrentDrive_") {
                return Err("Gateway key path needs a manual drive selection");
            }
            let hop = crate::ManualGateway {
                host: (*host).into(),
                port: ports[i].parse().map_err(|_| "Invalid SSH gateway port")?,
                username: if users[i] == "<default>" {
                    String::new()
                } else {
                    users[i].into()
                },
                auth: if keys[i].is_empty() {
                    crate::SshAuth::WindowsAgent
                } else {
                    crate::SshAuth::KeyFile
                },
                key_file: keys[i].into(),
                ..Default::default()
            };
            hop.validate()
                .map_err(|_| "Invalid SSH gateway endpoint or settings")?;
            Ok(crate::Gateway::Manual(hop))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discarded_icons_never_reject_valid_bookmarks() {
        for icon in [
            "",
            "-1",
            "65536",
            "999999999999999999999",
            "custom-icon.ico",
        ] {
            for (fields, protocol) in [
                ("0%ssh.test%22%user", Protocol::Ssh),
                ("1%telnet.test%23%user", Protocol::Telnet),
                ("4%rdp.test%3389%user", Protocol::Rdp),
                ("5%vnc.test%5900", Protocol::Vnc),
                ("8%1%9600%3%0%0%0%2%COM7", Protocol::Serial),
            ] {
                let source = format!("[Bookmarks]\nSubRep=Team\\Test\nSession=#{icon}#{fields}\n");
                let parsed = parse(source.as_bytes()).unwrap();
                assert!(
                    parsed.skipped.is_empty(),
                    "icon {icon:?}: {:?}",
                    parsed.skipped
                );
                assert_eq!(parsed.profiles.len(), 1);
                assert_eq!(parsed.profiles[0].protocol, protocol);
                assert_eq!(parsed.profiles[0].group, "Team / Test");
                assert!(parsed.profiles[0].icon.is_empty());
            }
        }
    }

    #[test]
    fn ignoring_icons_keeps_protocol_boundaries_and_validation() {
        let source = b"[Bookmarks]\nMissing delimiter=#0%ssh.test%22%user\nTruncated=#custom#0%ssh.test\nUnknown=##99%ssh.test%22%user\nInvalid endpoint=#-1#0%bad host%22%user\n";
        let parsed = parse(source).unwrap();
        assert!(parsed.profiles.is_empty());
        assert_eq!(parsed.skipped.len(), 4);
    }

    #[test]
    fn embedded_png_icons_survive_preview_and_merge_with_safe_fallback() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==";
        let source = format!(
            "[Bookmarks]\nPNG=#b64:{png}#0%ssh.test%22%user\nInvalid=#b64:invalid#0%ssh.test%22%user\nRemote=#https://example.test/icon.png#0%ssh.test%22%user\n"
        );
        let parsed = parse(source.as_bytes()).unwrap();
        assert!(parsed.skipped.is_empty());
        assert_eq!(parsed.profiles.len(), 3);
        let expected = format!("data:image/png;base64,{png}");
        assert_eq!(parsed.profiles[0].icon, expected);
        assert!(parsed.profiles[1..].iter().all(|p| p.icon.is_empty()));
        let merged =
            crate::session_transfer::merge(&crate::config::Preferences::fresh(), &parsed.profiles)
                .unwrap();
        assert_eq!(merged.profiles[0].icon, expected);
        let oversized = format!(
            "[Bookmarks]\nLarge=#b64:{}#0%ssh.test%22%user\n",
            "A".repeat(12000)
        );
        let parsed = parse(oversized.as_bytes()).unwrap();
        assert_eq!(parsed.profiles.len(), 1);
        assert!(parsed.profiles[0].icon.is_empty());
    }

    #[test]
    fn section_folders_are_applied_before_and_after_entries_and_survive_merge() {
        let parsed = parse(include_bytes!(
            "../tests/fixtures/bookmark-groups.mxtsessions"
        ))
        .unwrap();
        assert_eq!(
            parsed
                .profiles
                .iter()
                .map(|p| p.group.as_str())
                .collect::<Vec<_>>(),
            [
                "",
                "Team / Production",
                "Team / Production",
                "Team / Lab",
                "Operations",
                ""
            ]
        );
        assert_eq!(parsed.skipped.len(), 2);
        assert!(parsed.skipped[0].reason.contains("64 UTF-8"));
        assert!(parsed.skipped[1].reason.contains("Duplicate"));
        let mut prefs = crate::config::Preferences::fresh();
        let mut existing = parsed.profiles[0].clone();
        existing.id = 100;
        existing.group = "Existing".into();
        prefs.profiles.push(existing);
        let merged = crate::session_transfer::merge(&prefs, &parsed.profiles).unwrap();
        assert_eq!(merged.profiles[0].group, "Existing");
        for (imported, source) in merged.profiles[1..].iter().zip(&parsed.profiles) {
            assert!(imported.id > 100);
            assert_eq!(imported.group, source.group);
            assert!(!imported.logging.enabled && imported.credential_id.is_empty());
            imported.validate().unwrap();
        }
        assert!(
            !serde_json::to_string(&parsed)
                .unwrap()
                .contains("fixture-secret")
        );
    }
    #[test]
    fn folder_bounds_count_joined_utf8_and_report_each_affected_bookmark() {
        assert_eq!(folder(" \\ Team \\ Lab \\ ").unwrap(), "Team / Lab");
        assert_eq!(folder(&"界".repeat(21)).unwrap().len(), 63);
        assert_eq!(folder(&format!("{}\\b", "a".repeat(60))).unwrap().len(), 64);
        for value in [
            "界".repeat(22),
            format!("{}\\b", "a".repeat(61)),
            "x\u{1b}y".into(),
        ] {
            let source = format!(
                "[Bookmarks]\na=#1#1%a.test%23%fixture\nSubRep={value}\nb=#1#1%b.test%23%fixture\n"
            );
            let parsed = parse(source.as_bytes()).unwrap();
            assert!(parsed.profiles.is_empty());
            assert_eq!(
                parsed.skipped.iter().map(|p| p.line).collect::<Vec<_>>(),
                [2, 4]
            );
            assert!(!serde_json::to_string(&parsed).unwrap().contains(&value));
        }
    }
    #[test]
    fn synthetic_fixtures_import_five_protocols_and_report_every_rejection() {
        for data in [
            include_bytes!("../tests/fixtures/sessions.mxtsessions").as_slice(),
            include_bytes!("../tests/fixtures/MobaXterm.ini").as_slice(),
        ] {
            let r = parse(data).unwrap();
            assert_eq!(
                r.profiles.iter().map(|p| p.protocol).collect::<Vec<_>>(),
                Protocol::ALL
            );
            assert_eq!(r.profiles[4].host, "COM7");
            assert_eq!(r.profiles[4].serial.baud_rate, 9600);
            assert_eq!(r.skipped.len(), 4);
            let mut prefs = crate::config::Preferences::fresh();
            prefs.profiles.push(r.profiles[0].clone());
            let merged = crate::session_transfer::merge(&prefs, &r.profiles).unwrap();
            assert_eq!(merged.profiles.len(), 5);
            assert!(merged.profiles[1].id > prefs.profiles[0].id);
            for p in &merged.profiles[1..] {
                assert!(!p.logging.enabled && p.logging.file.is_empty());
                assert!(p.ssh.key_file.is_empty() && p.ssh.config_file.is_empty());
                assert!(!serde_json::to_string(p).unwrap().contains("fixture-secret"));
            }
        }
    }
    #[test]
    fn encoding_limits_truncation_and_unknowns_are_explicit() {
        let cp = b"[Bookmarks]\nEuro\x80=#1#0%ssh.test%22%user\n";
        assert_eq!(parse(cp).unwrap().profiles[0].name, "Euro€");
        let text = "[Bookmarks]\n界=#999#5%vnc.test%5900\n";
        let mut utf16 = vec![255, 254];
        utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(parse(&utf16).unwrap().profiles[0].name, "界");
        utf16.pop();
        assert!(parse(&utf16).is_err());
        assert!(parse(&vec![b'x'; MAX_BYTES + 1]).is_err());
        assert!(parse(b"[Passwords]\nx=fixture-secret").is_err());
        assert!(parse(format!("[Bookmarks]\n{}", "x=#1#99\n".repeat(1001)).as_bytes()).is_err());
        let source = "[Bookmarks]\nx=#1#0%ssh.test%22%user";
        for end in 12..source.len() {
            if let Ok(r) = parse(&source.as_bytes()[..end]) {
                assert_eq!(r.profiles.len() + r.skipped.len(), 1);
            }
        }
        let r = parse(b"[Bookmarks]\nx=#1#99%fixture-secret\ny=#1#0%bad host%22%user\nz=#1#8%1%9600%3%0%0%3%2%COM7\n").unwrap();
        assert!(r.profiles.is_empty());
        assert_eq!(r.skipped.len(), 3);
        assert!(
            !serde_json::to_string(&r)
                .unwrap()
                .contains("fixture-secret")
        );
    }
    #[test]
    fn serial_fixture_maps_framing_and_flow_without_importing_actions() {
        let r = parse(include_bytes!(
            "../tests/fixtures/serial-variants.mxtsessions"
        ))
        .unwrap();
        assert_eq!(r.profiles.len(), 4);
        assert_eq!(r.skipped.len(), 4);
        for (p, (data, stop, parity, flow)) in r.profiles.iter().zip([
            (5, 1, SerialParity::None, SerialFlow::None),
            (6, 2, SerialParity::Odd, SerialFlow::Software),
            (7, 1, SerialParity::Even, SerialFlow::Hardware),
            (8, 2, SerialParity::None, SerialFlow::Hardware),
        ]) {
            assert_eq!(p.serial.data_bits, data);
            assert_eq!(p.serial.stop_bits, stop);
            assert_eq!(p.serial.parity, parity);
            assert_eq!(p.serial.flow_control, flow);
            assert!(!p.serial.dtr && !p.serial.rts && !p.logging.enabled);
            assert!(p.credential_id.is_empty());
            assert_eq!(p.host, "COM7");
        }
        let prefs = crate::config::Preferences::fresh();
        let merged = crate::session_transfer::merge(&prefs, &r.profiles).unwrap();
        assert_eq!(merged.profiles.len(), 4);
        assert_eq!(merged.profiles[1].serial.parity, SerialParity::Odd);
    }
    #[test]
    fn all_supported_serial_combinations_and_bad_indices_are_checked() {
        for data in 0..4 {
            for stop in 0..2 {
                for parity in 0..3 {
                    for flow in 0..3 {
                        let input = format!(
                            "[Bookmarks]\nSynthetic=#131#8%99%19200%{data}%{stop}%{parity}%{flow}%2%COM42 (Synthetic)\n"
                        );
                        let r = parse(input.as_bytes()).unwrap();
                        assert!(r.skipped.is_empty());
                        assert_eq!(r.profiles[0].serial.data_bits, data + 5);
                        assert_eq!(r.profiles[0].serial.stop_bits, stop + 1);
                        assert_eq!(r.profiles[0].host, "COM42");
                    }
                }
            }
        }
        for field in 3..=6 {
            for value in ["", "-1", "99", "01", "fixture-secret"] {
                let mut fields = vec!["8", "1", "9600", "3", "0", "0", "0", "2", "COM7"];
                fields[field] = value;
                let r =
                    parse(format!("[Bookmarks]\nSynthetic=#131#{}", fields.join("%")).as_bytes())
                        .unwrap();
                assert!(r.profiles.is_empty());
                assert_eq!(r.skipped.len(), 1);
                assert!(
                    !serde_json::to_string(&r)
                        .unwrap()
                        .contains("fixture-secret")
                );
            }
        }
    }
}

#[cfg(test)]
mod gateway_tests {
    use super::*;
    #[test]
    fn gateway_lists_are_manual_ordered_and_credential_free() {
        for (protocol, start, key) in [("0", 8, 15), ("4", 13, 18), ("5", 5, 8)] {
            let mut fields = vec![""; key + 1];
            fields[0] = protocol;
            fields[1] = "target.test";
            fields[2] = "5900";
            fields[3] = "user";
            fields[start] = "first.test__PIPE__second.test";
            fields[start + 1] = "22__PIPE__2222";
            fields[start + 2] = "jump1__PIPE__jump2";
            fields[key] = "__PIPE__C:\\keys\\test";
            let input = format!(
                "[Bookmarks]\nTarget=#0#{}#ignored-secret\n",
                fields.join("%")
            );
            let preview = parse(input.as_bytes()).unwrap();
            assert!(preview.skipped.is_empty(), "{:?}", preview.skipped);
            assert_eq!(preview.profiles[0].gateways.len(), 2);
            let crate::Gateway::Manual(hop) = &preview.profiles[0].gateways[1] else {
                panic!()
            };
            assert_eq!(hop.host, "second.test");
            assert_eq!(hop.port, 2222);
            assert_eq!(hop.username, "jump2");
            assert!(hop.credential_id.is_empty());
            assert!(
                !serde_json::to_string(&preview)
                    .unwrap()
                    .contains("ignored-secret")
            );
        }
    }
    #[test]
    fn malformed_gateway_entries_are_listed_without_discarding_siblings() {
        for (hosts, ports, users, keys) in [
            ("a__PIPE__b", "22", "u__PIPE__v", "__PIPE__"),
            ("a", "0", "u", ""),
            ("-oProxyCommand=secret", "22", "u", ""),
            (
                "a__PIPE__b__PIPE__c__PIPE__d__PIPE__e",
                "22__PIPE__22__PIPE__22__PIPE__22__PIPE__22",
                "u__PIPE__u__PIPE__u__PIPE__u__PIPE__u",
                "__PIPE____PIPE____PIPE____PIPE__",
            ),
            ("a", "22", "u", "_CurrentDrive_:\\key"),
        ] {
            let mut f = vec![""; 16];
            f[0] = "0";
            f[1] = "target.test";
            f[2] = "22";
            f[3] = "u";
            f[8] = hosts;
            f[9] = ports;
            f[10] = users;
            f[15] = keys;
            let preview = parse(
                format!(
                    "[Bookmarks]\nBad=#0#{}\nGood=#0#0%good.test%22%user\n",
                    f.join("%")
                )
                .as_bytes(),
            )
            .unwrap();
            assert_eq!(preview.profiles.len(), 1, "{hosts}");
            assert_eq!(preview.profiles[0].name, "Good");
            assert_eq!(preview.skipped.len(), 1);
            assert_eq!(preview.skipped[0].line, 2);
            assert!(!preview.skipped[0].reason.contains("secret"));
        }
    }
    #[test]
    fn telnet_gateway_uses_its_own_metadata_slots() {
        let source = b"[Bookmarks]\nTelnet=#98#1%target.example%23%login%%1%jump.example%2299%jumpuser%%0%0%%1080%%%0%-1%0\n";
        let preview = parse(source).unwrap();
        assert!(preview.skipped.is_empty());
        let crate::Gateway::Manual(hop) = &preview.profiles[0].gateways[0] else {
            panic!()
        };
        assert_eq!(
            (&*hop.host, hop.port, &*hop.username),
            ("jump.example", 2299, "jumpuser")
        );
        assert!(hop.key_file.is_empty());
    }
}
