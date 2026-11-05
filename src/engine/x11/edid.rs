//! A monitor's own name from its EDID, the block every display sends describing itself:
//! maker, model and serial, so settings can follow it from one connector to another.

const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
/// Display descriptor tags for the monitor's name and its serial as text.
const NAME: u8 = 0xfc;
const SERIAL: u8 = 0xff;

/// Maker, then name or product code, then serial, as far as the block says.
pub(super) fn identity(edid: &[u8]) -> Option<String> {
    if edid.len() < 128 || edid[..8] != HEADER {
        return None;
    }
    let packed = u16::from_be_bytes([edid[8], edid[9]]);
    let letter = |shift: u16| char::from(b'@' + ((packed >> shift) & 0x1f) as u8);
    let maker: String = [letter(10), letter(5), letter(0)].into_iter().collect();
    if !maker.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let product = u16::from_le_bytes([edid[10], edid[11]]);
    let number = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);
    let mut name = None;
    let mut serial = None;
    for descriptor in edid[54..126].as_chunks::<18>().0 {
        if descriptor[..3] != [0, 0, 0] {
            continue;
        }
        let text = String::from_utf8_lossy(&descriptor[5..])
            .split('\n')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        match descriptor[3] {
            NAME if !text.is_empty() => name = Some(text),
            SERIAL if !text.is_empty() => serial = Some(text),
            _ => {}
        }
    }
    let model = name.unwrap_or_else(|| format!("{product:04X}"));
    let serial = serial.or_else(|| (number != 0).then(|| number.to_string()));
    Some(match serial {
        Some(serial) => format!("{maker} {model} {serial}"),
        None => format!("{maker} {model}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edid(name: Option<&str>, serial_text: Option<&str>, serial: u32) -> Vec<u8> {
        let mut block = vec![0u8; 128];
        block[..8].copy_from_slice(&HEADER);
        // "DEL": D=4, E=5, L=12.
        let maker: u16 = (4 << 10) | (5 << 5) | 12;
        block[8..10].copy_from_slice(&maker.to_be_bytes());
        block[10..12].copy_from_slice(&0xa0b1u16.to_le_bytes());
        block[12..16].copy_from_slice(&serial.to_le_bytes());
        for (at, (tag, text)) in [(NAME, name), (SERIAL, serial_text)]
            .into_iter()
            .enumerate()
        {
            if let Some(text) = text {
                let start = 54 + at * 18;
                block[start + 3] = tag;
                let mut bytes = text.as_bytes().to_vec();
                bytes.push(b'\n');
                bytes.resize(13, b' ');
                block[start + 5..start + 18].copy_from_slice(&bytes);
            }
        }
        block
    }

    #[test]
    fn the_name_and_serial_come_from_the_descriptors() {
        assert_eq!(
            identity(&edid(Some("DELL U2720Q"), Some("7XYZ123"), 0)).as_deref(),
            Some("DEL DELL U2720Q 7XYZ123")
        );
    }

    #[test]
    fn without_descriptors_the_product_code_and_number_stand_in() {
        assert_eq!(
            identity(&edid(None, None, 4242)).as_deref(),
            Some("DEL A0B1 4242")
        );
        assert_eq!(identity(&edid(None, None, 0)).as_deref(), Some("DEL A0B1"));
    }

    #[test]
    fn anything_but_an_edid_says_nothing() {
        assert_eq!(identity(&[0; 128]), None);
        assert_eq!(identity(&edid(None, None, 1)[..100]), None);
    }
}
