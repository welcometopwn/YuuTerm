//! Wake-on-LAN packet construction; transport belongs to the desktop worker.
pub fn parse_mac(value: &str) -> Result<[u8; 6], &'static str> {
    let error = "Enter a unicast MAC address such as 02:11:22:33:44:55";
    if value.len() != 17 {
        return Err(error);
    }
    let separator = if value.contains(':') { ':' } else { '-' };
    let parts: Vec<_> = value.split(separator).collect();
    if parts.len() != 6 {
        return Err(error);
    }
    let mut mac = [0; 6];
    for (out, part) in mac.iter_mut().zip(parts) {
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(error);
        }
        *out = u8::from_str_radix(part, 16).map_err(|_| error)?;
    }
    if mac == [0; 6] || mac[0] & 1 != 0 {
        return Err(error);
    }
    Ok(mac)
}
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [255; 102];
    for target in packet[6..].as_chunks_mut::<6>().0 {
        target.copy_from_slice(&mac);
    }
    packet
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_ethernet_addresses_and_builds_the_wire_packet() {
        let mac = parse_mac("02:ab:CD:33:44:55").unwrap();
        assert_eq!(mac, parse_mac("02-AB-cd-33-44-55").unwrap());
        let packet = magic_packet(mac);
        assert_eq!(&packet[..6], &[255; 6]);
        assert_eq!(packet.len(), 102);
        assert!(packet[6..].as_chunks::<6>().0.iter().all(|p| *p == mac));
        for invalid in [
            "",
            "00:00:00:00:00:00",
            "ff:ff:ff:ff:ff:ff",
            "01:11:22:33:44:55",
            "2:11:22:33:44:55",
            "02:11-22:33:44:55",
            "02:11:22:33:44:gg",
            " 02:11:22:33:44:55",
        ] {
            assert!(parse_mac(invalid).is_err());
        }
    }
}
