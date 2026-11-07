//! Optional CUPS marker telemetry. Malformed telemetry does not change print settings.
use rip_ipp::Request as Message;
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct Messages {
    pub en: &'static str,
    pub ja: &'static str,
    #[serde(rename = "zh-CN")]
    pub zh_cn: &'static str,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerSupplies {
    pub profile: &'static str,
    pub source: &'static str,
    pub status: &'static str,
    pub levels_attribute_state: &'static str,
    pub reason_code: Option<&'static str>,
    pub markers: Vec<Marker>,
    pub numeric_levels_reported: usize,
    pub all_levels_numeric: bool,
    pub advisory_only: bool,
    pub messages: Messages,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub index: usize,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub color: Option<String>,
    pub raw_level: Option<i32>,
    pub level_state: &'static str,
    pub level_percent: Option<i32>,
    pub low_level_percent: Option<i32>,
    pub high_level_percent: Option<i32>,
}
fn attributes<'a>(
    message: &'a Message<'_>,
    name: &str,
) -> Result<Option<Vec<&'a rip_ipp::Attribute>>, &'static str> {
    let attrs: Vec<_> = message
        .attributes
        .iter()
        .filter(|a| a.group == 4 && a.name == name)
        .collect();
    if attrs.is_empty()
        || (attrs.len() == 1
            && matches!(attrs[0].tag, 0x10 | 0x12 | 0x13)
            && attrs[0].value.is_empty())
    {
        return Ok(None);
    }
    if attrs.len() > 64 {
        return Err("MARKER_COUNT_LIMIT");
    }
    Ok(Some(attrs))
}
fn levels_attribute_state(message: &Message<'_>) -> &'static str {
    let attrs: Vec<_> = message
        .attributes
        .iter()
        .filter(|a| a.group == 4 && a.name == "marker-levels")
        .collect();
    match attrs.as_slice() {
        [] => "UNREPORTED",
        [a] if a.tag == 0x10 && a.value.is_empty() => "UNSUPPORTED",
        [a] if a.tag == 0x12 && a.value.is_empty() => "UNKNOWN",
        [a] if a.tag == 0x13 && a.value.is_empty() => "NO_VALUE",
        _ => "REPORTED",
    }
}
fn integers(
    message: &Message<'_>,
    name: &str,
    minimum: i32,
) -> Result<Option<Vec<i32>>, &'static str> {
    attributes(message, name)?
        .map(|attrs| {
            attrs
                .into_iter()
                .map(|a| {
                    if a.tag != 0x21 || a.value.len() != 4 {
                        return Err("INVALID_MARKER_INTEGER");
                    }
                    let value = i32::from_be_bytes(a.value.as_slice().try_into().unwrap());
                    if !(minimum..=100).contains(&value) {
                        return Err("INVALID_MARKER_LEVEL");
                    }
                    Ok(value)
                })
                .collect()
        })
        .transpose()
}
fn strings(
    message: &Message<'_>,
    name: &str,
    tag: u8,
) -> Result<Option<Vec<String>>, &'static str> {
    attributes(message, name)?
        .map(|attrs| {
            attrs
                .into_iter()
                .map(|a| {
                    if a.tag != tag || a.value.is_empty() || a.value.len() > 1024 {
                        return Err("INVALID_MARKER_TEXT");
                    }
                    let text = std::str::from_utf8(&a.value).map_err(|_| "INVALID_MARKER_TEXT")?;
                    if text.chars().any(char::is_control) {
                        return Err("INVALID_MARKER_TEXT");
                    }
                    Ok(text.to_string())
                })
                .collect()
        })
        .transpose()
}
fn parse(message: &Message<'_>) -> Result<Vec<Marker>, &'static str> {
    let levels = integers(message, "marker-levels", -3)?;
    let low = integers(message, "marker-low-levels", 0)?;
    let high = integers(message, "marker-high-levels", 0)?;
    let names = strings(message, "marker-names", 0x42)?;
    let kinds = strings(message, "marker-types", 0x44)?;
    let colors = strings(message, "marker-colors", 0x42)?;
    let counts = [
        levels.as_ref().map(Vec::len),
        low.as_ref().map(Vec::len),
        high.as_ref().map(Vec::len),
        names.as_ref().map(Vec::len),
        kinds.as_ref().map(Vec::len),
        colors.as_ref().map(Vec::len),
    ];
    let count = counts.iter().flatten().next().copied().unwrap_or(0);
    if counts.iter().flatten().any(|n| *n != count) {
        return Err("MARKER_ARRAY_LENGTH_MISMATCH");
    }
    Ok((0..count)
        .map(|index| {
            let raw = levels.as_ref().map(|v| v[index]);
            Marker {
                index: index + 1,
                name: names.as_ref().map(|v| v[index].clone()),
                kind: kinds.as_ref().map(|v| v[index].clone()),
                color: colors.as_ref().map(|v| v[index].clone()),
                raw_level: raw,
                level_state: match raw {
                    Some(-1) => "UNAVAILABLE",
                    Some(-2) => "UNKNOWN",
                    Some(-3) => "NOT_AT_CAPACITY",
                    Some(_) => "PERCENTAGE",
                    None => levels_attribute_state(message),
                },
                level_percent: raw.filter(|v| *v >= 0),
                low_level_percent: low.as_ref().map(|v| v[index]),
                high_level_percent: high.as_ref().map(|v| v[index]),
            }
        })
        .collect())
}
pub(crate) fn describe(message: &Message<'_>) -> MarkerSupplies {
    let (markers, reason_code) = match parse(message) {
        Ok(markers) => (markers, None),
        Err(code) => (vec![], Some(code)),
    };
    let count = markers.iter().filter(|m| m.level_percent.is_some()).count();
    let status = if reason_code.is_some() {
        "INVALID_RESPONSE"
    } else if markers.is_empty() {
        "UNREPORTED"
    } else {
        "REPORTED"
    };
    MarkerSupplies {
        profile: "org.jrip.cups-marker-supplies-v1",
        source: "cups-marker-attributes",
        status,
        levels_attribute_state: if reason_code.is_some() {
            "INVALID_RESPONSE"
        } else {
            levels_attribute_state(message)
        },
        reason_code,
        all_levels_numeric: !markers.is_empty() && count == markers.len(),
        numeric_levels_reported: count,
        markers,
        advisory_only: true,
        messages: if status == "INVALID_RESPONSE" {
            Messages {
                en: "Supply telemetry is invalid; inspect the device directly",
                ja: "消耗品情報が不正です。機器側で確認してください",
                zh_cn: "耗材信息无效，请直接检查设备",
            }
        } else {
            Messages {
                en: "Reported supply percentages are not remaining-page estimates; waste supplies may fill up",
                ja: "報告された百分率は残り印刷枚数ではありません。廃棄容器は使用で増加します",
                zh_cn: "报告百分比不是剩余可打印页数；废料容器可能随使用增加",
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rip_ipp::Attribute;
    fn attr(name: &str, tag: u8, value: Vec<u8>) -> Attribute {
        Attribute {
            group: 4,
            name: name.into(),
            tag,
            value,
        }
    }
    fn level(name: &str, value: i32) -> Attribute {
        attr(name, 0x21, value.to_be_bytes().to_vec())
    }
    fn report(attrs: Vec<Attribute>) -> MarkerSupplies {
        describe(&Message {
            version: [1, 1],
            operation: 0,
            request_id: 1,
            attributes: attrs,
            document: &[],
        })
    }
    #[test]
    fn percentages_and_unknown_sentinels_are_distinct_without_remaining_page_guesses() {
        for (value, state, percent) in [
            (0, "PERCENTAGE", Some(0)),
            (100, "PERCENTAGE", Some(100)),
            (-1, "UNAVAILABLE", None),
            (-2, "UNKNOWN", None),
            (-3, "NOT_AT_CAPACITY", None),
        ] {
            let r = report(vec![level("marker-levels", value)]);
            assert_eq!(r.status, "REPORTED");
            assert_eq!(r.markers[0].raw_level, Some(value));
            assert_eq!(r.markers[0].level_state, state);
            assert_eq!(r.markers[0].level_percent, percent);
            assert_eq!(r.all_levels_numeric, percent.is_some());
            assert!(r.advisory_only);
            assert!(!r.messages.ja.is_empty());
            assert!(!r.messages.zh_cn.is_empty());
        }
        for (tag, state) in [(0x10, "UNSUPPORTED"), (0x12, "UNKNOWN"), (0x13, "NO_VALUE")] {
            let r = report(vec![
                attr("marker-levels", tag, vec![]),
                attr("marker-names", 0x42, b"Supply".to_vec()),
            ]);
            assert_eq!(r.levels_attribute_state, state);
            assert_eq!(r.markers[0].level_state, state);
            assert_eq!(r.markers[0].raw_level, None);
        }
        assert_eq!(report(vec![]).status, "UNREPORTED");
    }
    #[test]
    fn corresponding_arrays_keep_waste_threshold_direction_and_partial_metadata() {
        let r = report(vec![
            level("marker-levels", 90),
            level("marker-levels", 0),
            level("marker-low-levels", 95),
            level("marker-low-levels", 5),
            level("marker-high-levels", 0),
            level("marker-high-levels", 100),
            attr("marker-types", 0x44, b"waste-ink".to_vec()),
            attr("marker-types", 0x44, b"toner".to_vec()),
        ]);
        assert_eq!(r.markers.len(), 2);
        assert_eq!(r.markers[0].kind.as_deref(), Some("waste-ink"));
        assert_eq!(r.markers[0].high_level_percent, Some(0));
        assert_eq!(r.markers[0].low_level_percent, Some(95));
        assert_eq!(r.markers[1].level_percent, Some(0));
        assert_eq!(r.markers[1].name, None);
        assert_eq!(r.numeric_levels_reported, 2);
        let partial = report(vec![attr("marker-names", 0x42, b"Supply".to_vec())]);
        assert_eq!(partial.status, "REPORTED");
        assert_eq!(partial.markers[0].level_state, "UNREPORTED");
        assert!(!partial.all_levels_numeric);
    }
    #[test]
    fn malformed_telemetry_is_withheld_instead_of_zipping_or_truncating() {
        for attrs in [
            vec![level("marker-levels", 101)],
            vec![level("marker-levels", -4)],
            vec![level("marker-low-levels", -1)],
            vec![attr("marker-levels", 0x21, vec![0])],
            vec![attr("marker-levels", 0x44, b"50".to_vec())],
            vec![
                level("marker-levels", 5),
                level("marker-levels", 6),
                attr("marker-names", 0x42, b"Only one".to_vec()),
            ],
            vec![attr("marker-names", 0x42, b"bad\nname".to_vec())],
            vec![attr("marker-names", 0x42, vec![255])],
            vec![attr("marker-names", 0x42, vec![b'a'; 1025])],
            vec![attr("marker-levels", 0x12, vec![0])],
            (0..65).map(|_| level("marker-levels", 50)).collect(),
        ] {
            let r = report(attrs);
            assert_eq!(r.status, "INVALID_RESPONSE");
            assert!(r.reason_code.is_some());
            assert!(r.markers.is_empty());
            assert_eq!(r.numeric_levels_reported, 0);
        }
        assert_eq!(
            report((0..64).map(|_| level("marker-levels", 50)).collect())
                .markers
                .len(),
            64
        );
    }
}
