use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::Path,
    sync::OnceLock,
};

pub fn installed() -> &'static BTreeMap<u16, String> {
    static ICONS: OnceLock<BTreeMap<u16, String>> = OnceLock::new();
    ICONS.get_or_init(|| {
        for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(root) = std::env::var_os(variable)
                && let Some(icons) =
                    read_known_executable(&Path::new(&root).join("Mobatek/MobaXterm/MobaXterm.exe"))
            {
                return icons;
            }
        }
        BTreeMap::new()
    })
}

fn read_known_executable(path: &Path) -> Option<BTreeMap<u16, String>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    // This exact publisher build's FLICO32 archive and index mapping were checked
    // offline. Other versions fall back, rather than assigning unrelated icons.
    if bytes.len() > 64 * 1024 * 1024
        || format!("{:x}", Sha256::digest(&bytes))
            != "f0e77836182fe52e66c68f88fbe13801f935f100623103df1d8be2181294e8e5"
    {
        return None;
    }
    decode_archive(bytes.get(8_428_352..8_428_352 + 79_371)?).ok()
}

fn decode_archive(compressed: &[u8]) -> Result<BTreeMap<u16, String>, ()> {
    let declared = u64::from_le_bytes(
        compressed
            .get(5..13)
            .ok_or(())?
            .try_into()
            .map_err(|_| ())?,
    );
    if declared > 4 * 1024 * 1024 {
        return Err(());
    }
    let reader =
        lzma_rust2::LzmaReader::new_mem_limit(compressed, 16 * 1024, None).map_err(|_| ())?;
    let mut archive = Vec::new();
    reader
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut archive)
        .map_err(|_| ())?;
    if archive.len() as u64 != declared {
        return Err(());
    }
    stored_icons(&archive)
}

// Only stored local members from the checksum-pinned archive are supported.
// Nothing is extracted to disk, and member names are numeric lookup keys only.
fn stored_icons(zip: &[u8]) -> Result<BTreeMap<u16, String>, ()> {
    let mut icons = BTreeMap::new();
    let mut at = 0;
    for _ in 0..512 {
        let signature = zip.get(at..at + 4).ok_or(())?;
        if signature == b"PK\x01\x02" {
            return Ok(icons);
        }
        let header = zip.get(at..at + 30).ok_or(())?;
        let u16_at = |n| u16::from_le_bytes([header[n], header[n + 1]]) as usize;
        let u32_at = |n| u32::from_le_bytes(header[n..n + 4].try_into().unwrap()) as usize;
        let size = u32_at(18);
        let name_len = u16_at(26);
        let extra_len = u16_at(28);
        if signature != b"PK\x03\x04"
            || u16_at(6) & 9 != 0
            || u16_at(8) != 0
            || size != u32_at(22)
            || size > 16384
            || name_len > 64
        {
            return Err(());
        }
        let name =
            std::str::from_utf8(zip.get(at + 30..at + 30 + name_len).ok_or(())?).map_err(|_| ())?;
        at += 30 + name_len + extra_len;
        let data = zip.get(at..at + size).ok_or(())?;
        at += size;
        if name == "Empty.bmp" {
            continue;
        }
        let number = name.strip_suffix(".ico").ok_or(())?;
        let index: u16 = number.parse().map_err(|_| ())?;
        if index > 4095 || index.to_string() != number || icons.contains_key(&index) {
            return Err(());
        }
        icons.insert(index, ico_png(data)?);
    }
    Err(())
}

fn ico_png(data: &[u8]) -> Result<String, ()> {
    if data.len() < 22 || data[..6] != [0, 0, 1, 0, 1, 0] || data[6..8] != [32, 32] {
        return Err(());
    }
    let image =
        image::load_from_memory_with_format(data, image::ImageFormat::Ico).map_err(|_| ())?;
    if image.width() != 32 || image.height() != 32 {
        return Err(());
    }
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| ())?;
    yuuterm_core::session_icon::from_png(png.get_ref()).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires the checksum-pinned local MobaXterm installation"]
    fn installed_catalog_contains_original_indexed_pngs() {
        let icons = installed();
        assert_eq!(icons.len(), 301);
        for value in icons.values() {
            yuuterm_core::session_icon::validate(value).unwrap();
        }
        let source = b"[Bookmarks]\nOriginal=#117#0%ssh.test%22%user\n";
        let parsed = yuuterm_core::mobaxterm::parse_with_icons(source, icons).unwrap();
        assert_eq!(parsed.profiles[0].icon, icons[&117]);
    }
    #[test]
    #[ignore = "Read-only diagnostic; requires YUUTERM_IMPORT_TEST_FILE"]
    fn selected_export_import_summary() {
        let path = std::env::var_os("YUUTERM_IMPORT_TEST_FILE").expect("Select a source export");
        let bytes = std::fs::read(path).unwrap();
        let parsed =
            yuuterm_core::session_transfer::parse_import_with_icons(&bytes, installed()).unwrap();
        let first = yuuterm_core::session_transfer::merge(
            &yuuterm_core::config::Preferences::fresh(),
            &parsed.profiles,
        )
        .unwrap();
        let second = yuuterm_core::session_transfer::merge(&first, &parsed.profiles).unwrap();
        assert_eq!(second.profiles.len(), first.profiles.len());
        println!(
            "accepted={} rejected={} icons={} repeated_additions={}",
            parsed.profiles.len(),
            parsed.skipped.len(),
            first.profiles.iter().filter(|p| !p.icon.is_empty()).count(),
            second.profiles.len() - first.profiles.len()
        );
    }
    #[test]
    fn unknown_executables_and_malformed_archives_fall_back() {
        assert!(read_known_executable(Path::new("missing-moba.exe")).is_none());
        for data in [b"".as_slice(), b"not an archive", &[0; 32]] {
            assert!(decode_archive(data).is_err());
            assert!(stored_icons(data).is_err());
            assert!(ico_png(data).is_err());
        }
    }
}
