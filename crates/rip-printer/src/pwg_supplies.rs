//! Bounded PWG 5100.13 supply records; independent of CUPS marker arrays.
use crate::supplies::Messages;
use rip_ipp::{Attribute, Request};
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterSupplies {
    pub profile: &'static str,
    pub source: &'static str,
    pub status: &'static str,
    pub attribute_state: &'static str,
    pub description_attribute_state: &'static str,
    pub reason_code: Option<&'static str>,
    pub supplies: Vec<Supply>,
    pub advisory_only: bool,
    pub messages: Messages,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Supply {
    pub position: usize,
    pub raw: String,
    pub fields: BTreeMap<String, String>,
    pub kind: String,
    pub colorant_name: String,
    pub supply_class: Option<String>,
    pub unit: Option<String>,
    pub raw_level: i32,
    pub level: Option<i32>,
    pub level_state: &'static str,
    pub level_meaning: &'static str,
    pub level_percent: Option<i32>,
    pub raw_max_capacity: i32,
    pub max_capacity: Option<i32>,
    pub max_capacity_state: &'static str,
    pub description: Option<Description>,
}
#[derive(Debug, Serialize)]
pub struct Description {
    pub language: Option<String>,
    pub text: String,
}
fn values<'a>(message: &'a Request<'_>, name: &str) -> Result<Vec<&'a Attribute>, &'static str> {
    let values: Vec<_> = message
        .attributes
        .iter()
        .filter(|a| a.group == 4 && a.name == name)
        .collect();
    if values.len() > 64 {
        return Err("SUPPLY_COUNT_LIMIT");
    }
    Ok(values)
}
fn attribute_state(values: &[&Attribute]) -> &'static str {
    match values {
        [] => "UNREPORTED",
        [a] if a.value.is_empty() => match a.tag {
            0x10 => "UNSUPPORTED",
            0x12 => "UNKNOWN",
            0x13 => "NO_VALUE",
            _ => "REPORTED",
        },
        _ => "REPORTED",
    }
}
fn number(fields: &BTreeMap<String, String>, key: &str, minimum: i32) -> Result<i32, &'static str> {
    let value = fields.get(key).ok_or("MISSING_SUPPLY_KEY")?;
    if !value.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return Err("INVALID_SUPPLY_NUMBER");
    }
    let value = value.parse::<i32>().map_err(|_| "INVALID_SUPPLY_NUMBER")?;
    if value < minimum {
        return Err("INVALID_SUPPLY_NUMBER");
    }
    Ok(value)
}
fn record(raw: &[u8], position: usize) -> Result<Supply, &'static str> {
    if raw.is_empty() || raw.len() > 4096 || !raw.iter().all(|b| (0x20..=0x7e).contains(b)) {
        return Err("INVALID_SUPPLY_RECORD");
    }
    let raw = std::str::from_utf8(raw).map_err(|_| "INVALID_SUPPLY_RECORD")?;
    let body = raw.strip_suffix(';').ok_or("INVALID_SUPPLY_RECORD")?;
    let mut fields = BTreeMap::new();
    for pair in body.split(';') {
        let (key, value) = pair.split_once('=').ok_or("INVALID_SUPPLY_RECORD")?;
        if key.is_empty()
            || key.len() > 128
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || value.is_empty()
            || value.len() > 1024
            || fields.len() >= 32
        {
            return Err("INVALID_SUPPLY_FIELD");
        }
        let key = key.to_ascii_lowercase();
        let valid = match key.as_str() {
            "type" | "class" | "unit" | "colorantrole" => {
                value.bytes().all(|b| b.is_ascii_alphabetic())
            }
            "maxcapacity" | "level" => value.bytes().all(|b| b.is_ascii_digit() || b == b'-'),
            "index" | "markerindex" | "colorantindex" | "coloranttonality" => {
                value.bytes().all(|b| b.is_ascii_digit())
            }
            // MSN2 color names include hyphens, vendor domains and an sRGBA suffix.
            "colorantname" => value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_')),
            _ => value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b',')),
        };
        if !valid || fields.insert(key, value.to_string()).is_some() {
            return Err("INVALID_SUPPLY_FIELD");
        }
    }
    let kind = fields.get("type").ok_or("MISSING_SUPPLY_KEY")?.clone();
    let colorant_name = fields
        .get("colorantname")
        .ok_or("MISSING_SUPPLY_KEY")?
        .clone();
    let level = number(&fields, "level", -3)?;
    let capacity = number(&fields, "maxcapacity", -2)?;
    for (key, min, max) in [
        ("index", 1, 65535),
        ("markerindex", 0, 65535),
        ("colorantindex", 0, 65535),
        ("coloranttonality", 0, i32::MAX),
    ] {
        if fields.contains_key(key) && number(&fields, key, min)? > max {
            return Err("INVALID_SUPPLY_NUMBER");
        }
    }
    let unit = fields.get("unit").cloned();
    let percent = unit
        .as_deref()
        .is_some_and(|u| u.eq_ignore_ascii_case("percent"));
    if percent && (level > 100 || capacity > 100) {
        return Err("INVALID_SUPPLY_PERCENTAGE");
    }
    let level_meaning = match fields.get("class").map(String::as_str) {
        Some(c) if c.eq_ignore_ascii_case("supplyThatIsConsumed") => "CURRENT_SUPPLY",
        Some(c) if c.eq_ignore_ascii_case("receptacleThatIsFilled") => "REMAINING_SPACE",
        _ => "UNSPECIFIED",
    };
    Ok(Supply {
        position,
        raw: raw.to_string(),
        kind,
        colorant_name,
        supply_class: fields.get("class").cloned(),
        unit,
        raw_level: level,
        level_meaning,
        level: (level >= 0).then_some(level),
        level_state: match level {
            -1 => "OTHER",
            -2 => "UNKNOWN",
            -3 => "SOME_AVAILABLE",
            _ => "NUMERIC",
        },
        level_percent: (percent && level >= 0).then_some(level),
        raw_max_capacity: capacity,
        max_capacity: (capacity >= 0).then_some(capacity),
        max_capacity_state: match capacity {
            -1 => "OTHER",
            -2 => "UNKNOWN",
            _ => "NUMERIC",
        },
        description: None,
        fields,
    })
}
fn description(a: &Attribute) -> Result<Description, &'static str> {
    let (language, text) = match a.tag {
        0x41 => (None, a.value.as_slice()),
        0x35 => {
            if a.value.len() < 4 {
                return Err("INVALID_SUPPLY_DESCRIPTION");
            }
            let n = u16::from_be_bytes([a.value[0], a.value[1]]) as usize;
            if n == 0 || n > 63 || a.value.len() < n + 4 {
                return Err("INVALID_SUPPLY_DESCRIPTION");
            }
            let lang = &a.value[2..n + 2];
            if !lang.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-') {
                return Err("INVALID_SUPPLY_DESCRIPTION");
            }
            let len = u16::from_be_bytes([a.value[n + 2], a.value[n + 3]]) as usize;
            if a.value.len() != n + 4 + len {
                return Err("INVALID_SUPPLY_DESCRIPTION");
            }
            (
                Some(
                    std::str::from_utf8(lang)
                        .map_err(|_| "INVALID_SUPPLY_DESCRIPTION")?
                        .to_string(),
                ),
                &a.value[n + 4..],
            )
        }
        _ => return Err("INVALID_SUPPLY_DESCRIPTION"),
    };
    let text = std::str::from_utf8(text).map_err(|_| "INVALID_SUPPLY_DESCRIPTION")?;
    if text.len() > 1024 || text.chars().any(char::is_control) {
        return Err("INVALID_SUPPLY_DESCRIPTION");
    }
    Ok(Description {
        language,
        text: text.to_string(),
    })
}
fn parse(values: &[&Attribute], descriptions: &[&Attribute]) -> Result<Vec<Supply>, &'static str> {
    let supply_present = attribute_state(values) == "REPORTED";
    let descriptions_present = attribute_state(descriptions) == "REPORTED";
    if !supply_present {
        if descriptions_present {
            return Err("SUPPLY_DESCRIPTION_WITHOUT_RECORDS");
        }
        return Ok(vec![]);
    }
    if descriptions_present && descriptions.len() != values.len() {
        return Err("SUPPLY_DESCRIPTION_COUNT_MISMATCH");
    }
    values
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if a.tag != 0x30 {
                return Err("INVALID_SUPPLY_TAG");
            }
            let mut supply = record(&a.value, i + 1)?;
            if descriptions_present {
                supply.description = Some(description(descriptions[i])?);
            }
            Ok(supply)
        })
        .collect()
}
pub(crate) fn describe(message: &Request<'_>) -> PrinterSupplies {
    let records = values(message, "printer-supply");
    let descriptions = values(message, "printer-supply-description");
    let attribute = records
        .as_ref()
        .map(|v| attribute_state(v))
        .unwrap_or("INVALID_RESPONSE");
    let description_attribute = descriptions
        .as_ref()
        .map(|v| attribute_state(v))
        .unwrap_or("INVALID_RESPONSE");
    let result = records.and_then(|v| descriptions.and_then(|d| parse(&v, &d)));
    let (supplies, reason_code) = match result {
        Ok(v) => (v, None),
        Err(code) => (vec![], Some(code)),
    };
    let status = if reason_code.is_some() {
        "INVALID_RESPONSE"
    } else if supplies.is_empty() {
        "UNREPORTED"
    } else {
        "REPORTED"
    };
    PrinterSupplies {
        profile: "org.jrip.pwg-printer-supplies-v1",
        source: "printer-supply",
        status,
        attribute_state: attribute,
        description_attribute_state: description_attribute,
        reason_code,
        supplies,
        advisory_only: true,
        messages: if status == "INVALID_RESPONSE" {
            Messages {
                en: "Printer supply telemetry is invalid; inspect the device directly",
                ja: "機器の消耗品情報が不正です。機器側で確認してください",
                zh_cn: "设备耗材信息无效，请直接检查设备",
            }
        } else {
            Messages {
                en: "Supply quantities retain their reported units and class; they are not page estimates",
                ja: "消耗品の値は報告された単位・分類のままです。印刷可能枚数の推測ではありません",
                zh_cn: "耗材值保留报告的单位及分类，不推测可打印页数",
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASIC: &str = "type=toner;maxcapacity=100;level=0;colorantname=black;";
    fn attr(name: &str, tag: u8, value: Vec<u8>) -> Attribute {
        Attribute {
            group: 4,
            name: name.into(),
            tag,
            value,
        }
    }
    fn supply(s: &str) -> Attribute {
        attr("printer-supply", 0x30, s.as_bytes().to_vec())
    }
    fn report(attributes: Vec<Attribute>) -> PrinterSupplies {
        describe(&Request {
            version: [1, 1],
            operation: 0,
            request_id: 1,
            attributes,
            document: &[],
        })
    }
    fn localized(language: &str, text: &str) -> Vec<u8> {
        let mut bytes = (language.len() as u16).to_be_bytes().to_vec();
        bytes.extend_from_slice(language.as_bytes());
        bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }
    #[test]
    fn preserves_units_classes_zero_and_mib_sentinels_without_scaling() {
        for (level, state) in [
            (-3, "SOME_AVAILABLE"),
            (-2, "UNKNOWN"),
            (-1, "OTHER"),
            (0, "NUMERIC"),
            (100, "NUMERIC"),
        ] {
            let raw = format!("type=ink;maxcapacity=-2;level={level};colorantname=cyan;");
            let r = report(vec![supply(&raw)]);
            assert_eq!(r.status, "REPORTED");
            assert_eq!(r.supplies[0].raw_level, level);
            assert_eq!(r.supplies[0].level_state, state);
            assert_eq!(r.supplies[0].level_percent, None);
            assert_eq!(r.supplies[0].max_capacity, None);
            assert_eq!(r.supplies[0].max_capacity_state, "UNKNOWN");
        }
        let percent = report(vec![supply(
            &(BASIC.to_string() + "unit=percent;class=supplyThatIsConsumed;"),
        )]);
        assert_eq!(percent.supplies[0].level_percent, Some(0));
        assert_eq!(percent.supplies[0].level_meaning, "CURRENT_SUPPLY");
        let waste = report(vec![supply(
            "type=wasteToner;maxcapacity=100;level=90;colorantname=no-color;class=receptacleThatIsFilled;unit=percent;",
        )]);
        assert_eq!(waste.supplies[0].level_meaning, "REMAINING_SPACE");
        assert_eq!(waste.supplies[0].level_percent, Some(90));
        let measured = report(vec![supply(
            "type=fuser;maxcapacity=5000;level=2000;colorantname=no-color;unit=impressions;class=other;",
        )]);
        assert_eq!(measured.supplies[0].level, Some(2000));
        assert_eq!(measured.supplies[0].level_percent, None);
        assert_eq!(measured.supplies[0].level_meaning, "UNSPECIFIED");
    }
    #[test]
    fn unordered_extensions_case_insensitive_keys_and_color_extensions_are_preserved() {
        let raw = "LEVEL=5;type=ink;maxcapacity=100;colorantname=com.example-light-magenta_ff7fffff;site-number=part.1,part-2;index=1;markerindex=0;";
        let r = report(vec![supply(raw)]);
        assert_eq!(r.status, "REPORTED");
        assert_eq!(r.supplies[0].raw, raw);
        assert_eq!(r.supplies[0].fields["level"], "5");
        assert_eq!(r.supplies[0].fields["site-number"], "part.1,part-2");
        assert!(r.supplies[0].description.is_none());
    }
    #[test]
    fn invalid_records_are_withheld_as_optional_telemetry_errors() {
        for bad in [
            BASIC.replace("level=0;", ""),
            BASIC.replace("level=0;", "level=--1;"),
            BASIC.replace("level=0;", "level=2147483648;"),
            BASIC.replace("level=0;", "level=-4;"),
            BASIC.replace("maxcapacity=100;", "maxcapacity=-3;"),
            format!("{BASIC}LEVEL=7;"),
            format!("{BASIC}unit:percent;"),
            format!("{BASIC};"),
            BASIC.trim_end_matches(';').to_string(),
            format!("{BASIC}bad=value value;"),
            format!("{BASIC}\n"),
            format!("{BASIC}bad=é;"),
            format!("{BASIC}index=0;"),
            format!("{BASIC}markerindex=65536;"),
            BASIC.replace("level=0;", "level=101;unit=percent;"),
            "x".repeat(4097),
        ] {
            let r = report(vec![supply(&bad)]);
            assert_eq!(r.status, "INVALID_RESPONSE", "{bad}");
            assert!(r.supplies.is_empty());
            assert!(r.reason_code.is_some());
        }
        let mut wrong = supply(BASIC);
        wrong.tag = 0x41;
        assert_eq!(report(vec![wrong]).reason_code, Some("INVALID_SUPPLY_TAG"));
    }
    #[test]
    fn descriptions_are_language_aware_bounded_and_position_bound() {
        let r = report(vec![
            supply(BASIC),
            attr(
                "printer-supply-description",
                0x35,
                localized("ja", "黒トナー"),
            ),
        ]);
        assert_eq!(r.status, "REPORTED");
        let d = r.supplies[0].description.as_ref().unwrap();
        assert_eq!(d.language.as_deref(), Some("ja"));
        assert_eq!(d.text, "黒トナー");
        let r = report(vec![
            supply(BASIC),
            attr("printer-supply-description", 0x41, b"Black toner".to_vec()),
        ]);
        assert_eq!(r.supplies[0].description.as_ref().unwrap().language, None);
        let mismatch = report(vec![
            supply(BASIC),
            supply(BASIC),
            attr("printer-supply-description", 0x41, b"Only one".to_vec()),
        ]);
        assert_eq!(
            mismatch.reason_code,
            Some("SUPPLY_DESCRIPTION_COUNT_MISMATCH")
        );
        assert!(mismatch.supplies.is_empty());
        for data in [
            vec![],
            vec![0, 255, 0, 0],
            localized("en", "bad\ntext"),
            localized("en", &"x".repeat(1025)),
            {
                let mut b = localized("en", "text");
                b.push(0);
                b
            },
        ] {
            assert_eq!(
                report(vec![
                    supply(BASIC),
                    attr("printer-supply-description", 0x35, data)
                ])
                .reason_code,
                Some("INVALID_SUPPLY_DESCRIPTION")
            );
        }
    }
    #[test]
    fn missing_and_out_of_band_telemetry_are_not_fabricated_as_empty_supplies() {
        assert_eq!(report(vec![]).status, "UNREPORTED");
        for (tag, state) in [(0x10, "UNSUPPORTED"), (0x12, "UNKNOWN"), (0x13, "NO_VALUE")] {
            let r = report(vec![attr("printer-supply", tag, vec![])]);
            assert_eq!(r.status, "UNREPORTED");
            assert_eq!(r.attribute_state, state);
        }
        assert_eq!(
            report((0..64).map(|_| supply(BASIC)).collect())
                .supplies
                .len(),
            64
        );
        assert_eq!(
            report((0..65).map(|_| supply(BASIC)).collect()).reason_code,
            Some("SUPPLY_COUNT_LIMIT")
        );
        assert_eq!(
            report(vec![attr(
                "printer-supply-description",
                0x41,
                b"No record".to_vec()
            )])
            .status,
            "INVALID_RESPONSE"
        );
    }
}
