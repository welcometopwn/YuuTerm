//! Syntax metadata only. Never execute, connect, or read referenced paths.
use crate::config::{HighlightRule, Highlighting};
use serde::Serialize;
use std::collections::BTreeMap;

pub const MAX_BYTES: usize = 128 * 1024;
#[derive(Serialize)]
pub struct Accepted {
    pub line: usize,
    pub source: String,
    pub rule: HighlightRule,
}
#[derive(Serialize)]
pub struct Rejected {
    pub line: usize,
    pub reason: &'static str,
}
#[derive(Default, Serialize)]
pub struct Preview {
    pub accepted: Vec<Accepted>,
    pub rejected: Vec<Rejected>,
}
pub fn parse(bytes: &[u8]) -> Result<Preview, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Syntax import exceeds 128 KiB".into());
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err("Truncated UTF-16 syntax file".into());
        }
        let little = bytes[0] == 0xff;
        let units = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if little {
                    u16::from_le_bytes(*p)
                } else {
                    u16::from_be_bytes(*p)
                }
            })
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|_| "Invalid UTF-16 syntax file")?
    } else {
        std::str::from_utf8(bytes)
            .map_err(|_| "Use a UTF-8 or BOM-marked UTF-16 syntax export")?
            .trim_start_matches('\u{feff}')
            .to_owned()
    };
    if text.contains('\0') {
        return Err("Syntax file contains NUL bytes".into());
    }
    let mut result = Preview::default();
    let mut properties: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    let mut active = false;
    let mut sections = 0;
    for (index, raw) in text.lines().enumerate() {
        if index >= 512 || raw.len() > 4096 {
            return Err("Syntax file exceeds line bounds".into());
        }
        let line = raw.trim();
        let number = index + 1;
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if line.starts_with('[') {
            active = line.eq_ignore_ascii_case("[CustomSyntax]");
            if active {
                sections += 1;
            } else {
                result.rejected.push(Rejected {
                    line: number,
                    reason: "Unsupported section; only CustomSyntax is imported",
                });
            }
            continue;
        }
        if !active {
            result.rejected.push(Rejected {
                line: number,
                reason: "Entry outside CustomSyntax was not imported",
            });
            continue;
        }
        let Some((key, value)) = raw.trim_start().split_once('=') else {
            result.rejected.push(Rejected {
                line: number,
                reason: "Malformed syntax property",
            });
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if !matches!(
            key.as_str(),
            "name"
                | "useregex"
                | "casesensitive"
                | "red"
                | "green"
                | "yellow"
                | "blue"
                | "magenta"
                | "cyan"
                | "underline"
                | "blinking"
        ) {
            result.rejected.push(Rejected {
                line: number,
                reason: "Unsupported syntax property",
            });
            continue;
        }
        let value = if matches!(
            key.as_str(),
            "red" | "green" | "yellow" | "blue" | "magenta" | "cyan"
        ) {
            value
        } else {
            value.trim()
        };
        properties
            .entry(key)
            .or_default()
            .push((number, value.to_owned()));
    }
    if sections != 1 {
        return Err("Choose a syntax export with exactly one CustomSyntax section".into());
    }
    let flag = |key: &str, expected: &str| {
        properties
            .get(key)
            .is_some_and(|v| v.len() == 1 && v[0].1 == expected)
    };
    // YuuTerm is case-insensitive; list-mode delimiters/vendor substitutions
    // are not guessed. Every affected rule remains visible as a rejection.
    let supported = flag("useregex", "1") && flag("casesensitive", "0");
    for (key, values) in properties {
        for (line, value) in &values {
            let reason = if values.len() > 1 {
                Some("Duplicate property; none of its entries imported")
            } else if key == "name" {
                if value.len() > 80 || value.chars().any(char::is_control) {
                    Some("Invalid syntax name")
                } else {
                    None
                }
            } else if key == "useregex" || key == "casesensitive" {
                if !supported {
                    Some("Only UseRegex=1 and CaseSensitive=0 are supported")
                } else {
                    None
                }
            } else if value.is_empty() {
                None
            } else if key == "underline" || key == "blinking" {
                Some("Underline and blinking effects are unsupported")
            } else if !supported {
                Some("Rule needs UseRegex=1 and CaseSensitive=0")
            } else if value.contains(['¨', '╙', '╘']) {
                Some("Vendor-specific regex substitutions are unsupported")
            } else {
                let color = match key.as_str() {
                    "red" => "#ff0000",
                    "green" => "#00ff00",
                    "yellow" => "#ffff00",
                    "blue" => "#0000ff",
                    "magenta" => "#ff00ff",
                    "cyan" => "#00ffff",
                    _ => unreachable!(),
                };
                let rule = HighlightRule {
                    keywords: value.clone(),
                    color: color.into(),
                    regex: true,
                };
                let settings = Highlighting {
                    rules: vec![rule.clone()],
                    ..Default::default()
                };
                if settings.validate().is_err() {
                    Some("Regex is invalid, unsupported or exceeds YuuTerm bounds")
                } else {
                    result.accepted.push(Accepted {
                        line: *line,
                        source: key.clone(),
                        rule,
                    });
                    None
                }
            };
            if let Some(reason) = reason {
                result.rejected.push(Rejected {
                    line: *line,
                    reason,
                });
            }
        }
    }
    result.accepted.sort_by_key(|a| a.line);
    result.rejected.sort_by_key(|r| r.line);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/highlight-synthetic.msyn");
    #[test]
    fn synthetic_export_preview_preserves_order_and_lists_every_rejection() {
        let p = parse(FIXTURE).unwrap();
        assert_eq!(
            p.accepted
                .iter()
                .map(|a| a.source.as_str())
                .collect::<Vec<_>>(),
            ["red", "green", "blue"]
        );
        assert_eq!(p.accepted[0].rule.keywords, "\\berror\\b");
        assert_eq!(p.rejected.len(), 4);
        assert!(
            p.rejected
                .iter()
                .all(|r| !r.reason.contains("private-fixture-value"))
        );
        assert!(
            !serde_json::to_string(&p)
                .unwrap()
                .contains("private-fixture-value")
        );
        for bom in [[0xff, 0xfe], [0xfe, 0xff]] {
            let mut bytes = bom.to_vec();
            for unit in std::str::from_utf8(FIXTURE).unwrap().encode_utf16() {
                bytes.extend(if bom[0] == 0xff {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            assert_eq!(parse(&bytes).unwrap().accepted.len(), 3);
        }
    }
    #[test]
    fn bounds_duplicates_modes_and_truncation_fail_closed() {
        for bad in [
            vec![b'x'; MAX_BYTES + 1],
            b"[CustomSyntax]\0".to_vec(),
            vec![0xff, 0xfe, 0],
            b"[CustomSyntax]\n[CustomSyntax]".to_vec(),
            b"x\n".repeat(513),
        ] {
            assert!(parse(&bad).is_err());
        }
        for suffix in [
            "Red=error\nRed=failed",
            "Red=(?<=x)y",
            "Red=¨error",
            "Red=error\nUseRegex=0",
            "Red=error\nCaseSensitive=1",
        ] {
            let p =
                parse(format!("[CustomSyntax]\nUseRegex=1\nCaseSensitive=0\n{suffix}").as_bytes())
                    .unwrap();
            assert!(p.accepted.is_empty());
            assert!(!p.rejected.is_empty());
        }
        for prefix in 0..FIXTURE.len() {
            let _ = parse(&FIXTURE[..prefix]);
        }
    }
    #[test]
    fn pattern_whitespace_and_equals_are_preserved_but_unrelated_sections_are_not_disclosed() {
        let p=parse(b"[Other]\nPassword=unrelated-synthetic-secret\n[CustomSyntax]\nUseRegex=1\nCaseSensitive=0\nRed=  error=1  \n").unwrap();
        assert_eq!(p.accepted[0].rule.keywords, "  error=1  ");
        assert_eq!(p.rejected.len(), 2);
        assert!(
            !serde_json::to_string(&p)
                .unwrap()
                .contains("unrelated-synthetic-secret")
        );
    }
}
