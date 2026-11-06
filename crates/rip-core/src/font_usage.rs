//! Bounded evidence from MuPDF text callbacks, never a rendered-ink guarantee.
use crate::ValidationError;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Usage {
    schema_version: u32,
    profile: String,
    engine: String,
    phase: String,
    page_count: u32,
    text_operations: u64,
    glyph_callbacks: u64,
    fonts: Vec<Font>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Font {
    name: String,
    glyph_callbacks: u64,
    glyph_instances: u64,
    negative_glyph_ids: u64,
    zero_glyph_ids: u64,
    horizontal_glyphs: u64,
    vertical_glyphs: u64,
    first_page: u32,
    last_page: u32,
    page_count: u32,
    glyph_id_sample: Vec<u32>,
    sample_truncated: bool,
    operations: Operations,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Operations {
    fill: u64,
    stroke: u64,
    clip: u64,
    clip_stroke: u64,
    ignore: u64,
}

pub fn validate(value: &Value, pages: u32) -> Result<(), ValidationError> {
    let invalid = || ValidationError("invalid_font_usage");
    if serde_json::to_vec(value).map_err(|_| invalid())?.len() > 32768 {
        return Err(invalid());
    }
    let usage: Usage = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    if usage.schema_version != 1
        || usage.profile != "org.jrip.mupdf-font-usage-v1"
        || usage.engine != "mupdf"
        || usage.phase != "pdf-inspection"
        || usage.page_count != pages
        || !(1..=100).contains(&pages)
        || usage.text_operations > 100000
        || usage.glyph_callbacks > 1000000
        || (usage.glyph_callbacks > 0 && usage.text_operations == 0)
        || usage.fonts.len() > 64
    {
        return Err(invalid());
    }
    let mut total = 0;
    let mut names = std::collections::HashSet::new();
    for font in usage.fonts {
        let operations = [
            font.operations.fill,
            font.operations.stroke,
            font.operations.clip,
            font.operations.clip_stroke,
            font.operations.ignore,
        ];
        if font.name.chars().count() > 128
            || !names.insert(font.name)
            || !(1..=1000000).contains(&font.glyph_callbacks)
            || font.glyph_instances.checked_add(font.negative_glyph_ids)
                != Some(font.glyph_callbacks)
            || font.horizontal_glyphs.checked_add(font.vertical_glyphs)
                != Some(font.glyph_callbacks)
            || font.zero_glyph_ids > font.glyph_instances
            || operations.into_iter().try_fold(0u64, u64::checked_add) != Some(font.glyph_callbacks)
            || font.first_page == 0
            || font.first_page > font.last_page
            || font.last_page > pages
            || font.page_count == 0
            || font.page_count > font.last_page - font.first_page + 1
            || font.glyph_id_sample.len() > 16
            || font.glyph_id_sample.len() as u64 > font.glyph_instances
            || font.glyph_id_sample.iter().any(|&id| id > i32::MAX as u32)
            || font.glyph_id_sample.windows(2).any(|ids| ids[0] >= ids[1])
            || (font.sample_truncated && font.glyph_id_sample.len() != 16)
        {
            return Err(invalid());
        }
        total += font.glyph_callbacks;
    }
    if total != usage.glyph_callbacks {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn checks_totals_scope_bounds_and_does_not_accept_source_text() {
        let report = json!({"schemaVersion":1,"profile":"org.jrip.mupdf-font-usage-v1","engine":"mupdf","phase":"pdf-inspection","pageCount":2,"textOperations":2,"glyphCallbacks":2,"fonts":[{"name":"Owned","glyphCallbacks":2,"glyphInstances":2,"negativeGlyphIds":0,"zeroGlyphIds":0,"horizontalGlyphs":1,"verticalGlyphs":1,"firstPage":1,"lastPage":2,"pageCount":2,"glyphIdSample":[1,3],"sampleTruncated":false,"operations":{"fill":2,"stroke":0,"clip":0,"clipStroke":0,"ignore":0}}]});
        validate(&report, 2).unwrap();
        for (field, value) in [
            ("engine", json!("ghostscript")),
            ("phase", json!("raster-output")),
            ("glyphCallbacks", json!(3)),
            ("textOperations", json!(0)),
            ("text", json!("private source")),
        ] {
            let mut bad = report.clone();
            bad[field] = value;
            assert!(validate(&bad, 2).is_err());
        }
        for (field, value) in [
            ("glyphCallbacks", json!(3)),
            ("horizontalGlyphs", json!(u64::MAX)),
            ("pageCount", json!(3)),
            ("glyphIdSample", json!([3, 1])),
            ("sampleTruncated", json!(true)),
            ("unicode", json!([65])),
        ] {
            let mut bad = report.clone();
            bad["fonts"][0][field] = value;
            assert!(validate(&bad, 2).is_err());
        }
        let mut empty = report;
        empty["fonts"] = json!([]);
        empty["glyphCallbacks"] = json!(0);
        empty["textOperations"] = json!(0);
        validate(&empty, 2).unwrap();
        assert!(validate(&empty, 1).is_err());
    }
}
