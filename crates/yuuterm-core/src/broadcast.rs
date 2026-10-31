//! One reviewed dispatch to explicit live IDs; no persistent enrollment.
pub const MAX_TARGETS: usize = 64;
pub const MAX_BYTES: usize = 16 * 1024;

pub fn prepare(ids: &[u64], text: &str, append_enter: bool) -> Result<Vec<u8>, &'static str> {
    if ids.is_empty() || ids.len() > MAX_TARGETS {
        return Err("Select between 1 and 64 live terminals per dispatch.");
    }
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    if ids.contains(&0) || unique.len() != ids.len() {
        return Err("Broadcast targets must be distinct live connection IDs.");
    }
    if text.trim().is_empty() || text.len() + usize::from(append_enter) > MAX_BYTES {
        return Err("Enter nonempty text within the 16 KiB dispatch limit.");
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(
            "Broadcast text cannot contain terminal control characters other than LF and Tab.",
        );
    }
    let mut bytes = text.as_bytes().to_vec();
    for byte in &mut bytes {
        if *byte == b'\n' {
            *byte = b'\r';
        }
    }
    if append_enter {
        bytes.push(b'\r');
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_targets_and_execution_bytes_are_bounded() {
        assert_eq!(
            prepare(&[2, 9], "echo Grüße\nwhoami", false).unwrap(),
            "echo Grüße\rwhoami".as_bytes()
        );
        assert_eq!(prepare(&[2], "pwd", true).unwrap(), b"pwd\r");
        for ids in [vec![], vec![0], vec![2, 2], (1..=65).collect()] {
            assert!(prepare(&ids, "pwd", false).is_err());
        }
        for text in ["", "\n", "a\r\nb", "\x1b[31m", "\x03", "x\u{85}y"] {
            assert!(prepare(&[1], text, false).is_err());
        }
        assert!(prepare(&[1], &"é".repeat(8192), false).is_ok());
        assert!(prepare(&[1], &"é".repeat(8192), true).is_err());
    }
}
