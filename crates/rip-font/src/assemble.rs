//! SFNT assembly copied and adapted from Fonts font-core (MIT).
use std::collections::BTreeMap;
fn sum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |a, b| {
        let mut v = [0; 4];
        v[..b.len()].copy_from_slice(b);
        a.wrapping_add(u32::from_be_bytes(v))
    })
}
pub(crate) fn assemble(tables: BTreeMap<[u8; 4], Vec<u8>>, cff: bool) -> Result<Vec<u8>, String> {
    if tables.is_empty() || tables.len() > 4095 {
        return Err("color_invalid".into());
    }
    let count = tables.len() as u16;
    let power = 1u16 << count.ilog2();
    let mut out = Vec::new();
    out.extend(if cff { *b"OTTO" } else { [0, 1, 0, 0] });
    for n in [
        count,
        power * 16,
        count.ilog2() as u16,
        count * 16 - power * 16,
    ] {
        out.extend(n.to_be_bytes());
    }
    let mut payload = Vec::new();
    let base = 12 + tables.len() * 16;
    let mut head = None;
    for (tag, mut data) in tables {
        if &tag == b"head" {
            data.get_mut(8..12).ok_or("color_invalid")?.fill(0);
        }
        let at = base + payload.len();
        if &tag == b"head" {
            head = Some(at);
        }
        out.extend(tag);
        out.extend(sum(&data).to_be_bytes());
        out.extend((at as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        payload.extend(data);
        while payload.len() % 4 != 0 {
            payload.push(0);
        }
        if base + payload.len() > crate::MAX_BYTES {
            return Err("font_too_large".into());
        }
    }
    out.extend(payload);
    let adjust = 0xB1B0AFBAu32.wrapping_sub(sum(&out));
    let h = head.ok_or("color_invalid")?;
    out[h + 8..h + 12].copy_from_slice(&adjust.to_be_bytes());
    Ok(out)
}
