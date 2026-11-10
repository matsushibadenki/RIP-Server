//! Immutable installed font snapshots and table-preserving font conversion.
mod assemble;
pub mod store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, String>;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Font {
    pub name: String,
    pub family: String,
    pub style: String,
    pub file: String,
    pub sha256: String,
    pub source_sha256: String,
    pub bytes: u64,
    pub face_index: u32,
    pub glyphs: u16,
    pub format: String,
    pub cid: bool,
    pub bbox: [i32; 4],
    pub ascent: i32,
    pub descent: i32,
    pub cap_height: i32,
    pub italic_angle: f32,
    pub flags: u32,
}
pub struct Converted {
    pub font: Font,
    pub bytes: Vec<u8>,
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
}
pub fn convert(bytes: &[u8]) -> Result<Vec<Converted>> {
    if bytes.len() > MAX_BYTES || bytes.len() < 12 {
        return Err("font_size_invalid".into());
    }
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    if count == 0 || count > 32 {
        return Err("font_collection_limit".into());
    }
    let source_sha256 = hash(bytes);
    let mut result = Vec::new();
    let mut total = 0;
    for index in 0..count {
        let face = ttf_parser::Face::parse(bytes, index).map_err(|_| "unsupported_font_format")?;
        if face.is_variable() || face.tables().cff2.is_some() {
            return Err("variable_font_requires_static_instance".into());
        }
        let raw = face.raw_face();
        if [b"COLR", b"CBDT", b"sbix", b"SVG "]
            .iter()
            .any(|tag| raw.table(ttf_parser::Tag::from_bytes(tag)).is_some())
        {
            return Err("color_font_unsupported".into());
        }
        let cff = face.tables().cff.is_some();
        if !cff && face.tables().glyf.is_none() {
            return Err("font_outlines_required".into());
        }
        if let Some(os2) = raw.table(ttf_parser::Tag::from_bytes(b"OS/2")) {
            let f = os2.get(8..10).ok_or("invalid_font_permissions")?;
            let flags = u16::from_be_bytes([f[0], f[1]]);
            if flags & 0x202 != 0 {
                return Err("font_embedding_restricted".into());
            }
        }
        let getname = |id| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == id)
                .find_map(|n| n.to_string())
                .unwrap_or_default()
        };
        let name = getname(6);
        if !safe_name(&name) || result.iter().any(|c: &Converted| c.font.name == name) {
            return Err("invalid_or_duplicate_font_name".into());
        }
        let mut tables = BTreeMap::new();
        let mut table_bytes = 0usize;
        for record in raw.table_records {
            let tag = record.tag.to_bytes();
            if &tag == b"DSIG" {
                continue;
            }
            let data = raw.table(record.tag).ok_or("font_table_invalid")?;
            table_bytes = table_bytes
                .checked_add(data.len())
                .ok_or("font_table_limit")?;
            if table_bytes > MAX_BYTES {
                return Err("font_table_limit".into());
            }
            let data = data.to_vec();
            if tables.insert(tag, data).is_some() {
                return Err("font_table_invalid".into());
            }
        }
        let output = assemble::assemble(tables, cff)?;
        total += output.len();
        if total > 64 * 1024 * 1024 {
            return Err("font_collection_limit".into());
        }
        let checked = ttf_parser::Face::parse(&output, 0).map_err(|_| "font_conversion_failed")?;
        if checked.number_of_glyphs() != face.number_of_glyphs()
            || checked.units_per_em() != face.units_per_em()
        {
            return Err("font_conversion_failed".into());
        }
        let scale = |n: i16| (f64::from(n) * 1000. / f64::from(face.units_per_em())).round() as i32;
        let b = face.global_bounding_box();
        let sha256 = hash(&output);
        let extension = if cff { "otf" } else { "ttf" };
        let font = Font {
            name,
            family: getname(1).chars().take(256).collect(),
            style: getname(2).chars().take(256).collect(),
            file: format!("{sha256}.{extension}"),
            sha256,
            source_sha256: source_sha256.clone(),
            bytes: output.len() as u64,
            face_index: index,
            glyphs: face.number_of_glyphs(),
            format: extension.into(),
            cid: face
                .tables()
                .cff
                .is_some_and(|c| c.glyph_cid(ttf_parser::GlyphId(0)).is_some()),
            bbox: [
                scale(b.x_min),
                scale(b.y_min),
                scale(b.x_max),
                scale(b.y_max),
            ],
            ascent: scale(face.ascender()),
            descent: scale(face.descender()),
            cap_height: scale(face.capital_height().unwrap_or(face.ascender())),
            italic_angle: face.italic_angle(),
            flags: if face.is_monospaced() { 33 } else { 32 },
        };
        result.push(Converted {
            font,
            bytes: output,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tables(bytes: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
        let face = ttf_parser::Face::parse(bytes, 0).unwrap();
        face.raw_face()
            .table_records
            .into_iter()
            .map(|r| {
                (
                    r.tag.to_bytes(),
                    face.raw_face().table(r.tag).unwrap().to_vec(),
                )
            })
            .collect()
    }
    #[test]
    fn unsupported_formats_and_embedding_permissions_fail_closed() {
        assert_eq!(
            convert(include_bytes!("../tests/data/color.otf"))
                .err()
                .unwrap(),
            "color_font_unsupported"
        );
        assert_eq!(
            convert(include_bytes!("../tests/data/variable.ttf"))
                .err()
                .unwrap(),
            "variable_font_requires_static_instance"
        );
        let mut t = tables(include_bytes!("../tests/data/shaping.ttf"));
        t.get_mut(b"OS/2").unwrap()[8..10].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(
            convert(&assemble::assemble(t, false).unwrap())
                .err()
                .unwrap(),
            "font_embedding_restricted"
        );
    }
    #[test]
    fn collection_faces_are_extracted_without_rebuilding_outlines() {
        let ttf = include_bytes!("../tests/data/shaping.ttf").to_vec();
        let mut t = tables(include_bytes!("../tests/data/color.otf"));
        t.remove(b"COLR");
        t.remove(b"CPAL");
        let otf = assemble::assemble(t, true).unwrap();
        assert_eq!(convert(&otf).unwrap()[0].font.format, "otf");
        let mut collection = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        let first = 20u32;
        let second = (20 + ttf.len() + 3) & !3;
        collection.extend(first.to_be_bytes());
        collection.extend((second as u32).to_be_bytes());
        for (mut bytes, offset) in [(ttf, 20usize), (otf, second)] {
            let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
            for i in 0..count {
                let p = 12 + i * 16 + 8;
                let value = u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap()) + offset as u32;
                bytes[p..p + 4].copy_from_slice(&value.to_be_bytes());
            }
            collection.resize(offset, 0);
            collection.extend(bytes);
        }
        let faces = convert(&collection).unwrap();
        assert_eq!(faces.len(), 2);
        assert_eq!(faces[0].font.face_index, 0);
        assert_eq!(faces[1].font.face_index, 1);
        assert_eq!(faces[0].font.format, "ttf");
        assert_eq!(faces[1].font.format, "otf");
        assert_ne!(faces[0].font.name, faces[1].font.name);
    }
}
