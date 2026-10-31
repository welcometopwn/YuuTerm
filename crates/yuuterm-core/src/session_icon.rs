//! Raster-only, bounded inline icons. Never allow SVG, remote URLs or file paths.
use base64::{Engine, engine::general_purpose::STANDARD};
pub fn from_png(bytes: &[u8]) -> Result<String, &'static str> {
    if bytes.len() > 8192 {
        return Err("Session icon is too large");
    }
    let uri = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
    validate(&uri)?;
    Ok(uri)
}
pub fn validate(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Ok(());
    }
    if value.len() > 11000 {
        return Err("Session icon is too large");
    }
    let data = value
        .strip_prefix("data:image/png;base64,")
        .ok_or("Choose a PNG session icon")?;
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| "Invalid PNG session icon")?;
    if bytes.len() < 33
        || bytes.len() > 8192
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes[8..12] != 13u32.to_be_bytes()
        || &bytes[12..16] != b"IHDR"
    {
        return Err("Invalid or oversized PNG session icon");
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if !(1..=64).contains(&width) || !(1..=64).contains(&height) || bytes[24] != 8 {
        return Err("Session icon must be a small PNG (up to 64×64 pixels)");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_bounded_png_data_is_accepted_and_older_profiles_keep_no_icon() {
        let png=STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==").unwrap();
        let uri = format!("data:image/png;base64,{}", STANDARD.encode(&png));
        assert!(validate(&uri).is_ok());
        assert!(validate("").is_ok());
        for bad in [
            "https://example.test/icon.png",
            "file:///icon.png",
            "data:image/svg+xml,<svg/>",
            "data:image/png;base64,PHN2Zy8+",
        ] {
            assert!(validate(bad).is_err());
        }
        let mut huge = png.clone();
        huge[16..20].copy_from_slice(&65535u32.to_be_bytes());
        assert!(validate(&format!("data:image/png;base64,{}", STANDARD.encode(huge))).is_err());
        assert!(validate(&"x".repeat(11001)).is_err());
        let p:crate::Profile=serde_json::from_str(r#"{"id":1,"name":"old","protocol":"Ssh","host":"example.test","port":22,"username":"fixture","font_size":null}"#).unwrap();
        assert!(p.icon.is_empty());
        let mut prefs = crate::config::Preferences::fresh();
        let mut custom = p;
        custom.icon = uri.clone();
        prefs.profiles.push(custom);
        let duplicated = prefs.with_duplicate(1).unwrap();
        assert_eq!(duplicated.profiles[1].icon, uri);
        let mut imported =
            crate::session_transfer::parse(&crate::session_transfer::export(&prefs).unwrap())
                .unwrap();
        imported[0].host = "another.test".into();
        let merged = crate::session_transfer::merge(&prefs, &imported).unwrap();
        assert_eq!(merged.profiles[1].icon, uri);
        assert_ne!(merged.profiles[1].id, prefs.profiles[0].id);
    }
}
