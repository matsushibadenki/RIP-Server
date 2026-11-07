//! Canonical, bounded one-based source page selection and artifact mapping.
use crate::ValidationError;
use serde::{Deserialize, Deserializer, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PageRange {
    pub first: u32,
    pub last: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PageRanges(Vec<PageRange>);
impl PageRanges {
    pub fn new(ranges: Vec<PageRange>) -> Result<Self, ValidationError> {
        Self::bounded(ranges, 100)
    }
    fn bounded(ranges: Vec<PageRange>, maximum: u32) -> Result<Self, ValidationError> {
        if ranges.len() > 100 {
            return Err(ValidationError("invalid_page_ranges"));
        }
        let mut canonical: Vec<PageRange> = Vec::new();
        for range in ranges {
            if range.first == 0 || range.last < range.first || range.last > maximum {
                return Err(ValidationError("invalid_page_ranges"));
            }
            if let Some(previous) = canonical.last_mut() {
                if range.first <= previous.last {
                    return Err(ValidationError("invalid_page_ranges"));
                }
                if range.first == previous.last + 1 {
                    previous.last = range.last;
                    continue;
                }
            }
            canonical.push(range);
        }
        Ok(Self(canonical))
    }
    pub fn ranges(&self) -> &[PageRange] {
        &self.0
    }
    pub fn is_all(&self) -> bool {
        self.0.is_empty()
    }
    pub fn selected_count(&self) -> Option<usize> {
        (!self.is_all()).then(|| self.0.iter().map(|r| (r.last - r.first + 1) as usize).sum())
    }
    pub fn indices(&self, total: u32) -> Result<Vec<usize>, ValidationError> {
        if !(1..=100).contains(&total) || self.0.last().is_some_and(|r| r.last > total) {
            return Err(ValidationError("page_range_out_of_bounds"));
        }
        Ok(if self.is_all() {
            (0..total as usize).collect()
        } else {
            self.0
                .iter()
                .flat_map(|r| r.first..=r.last)
                .map(|n| n as usize - 1)
                .collect()
        })
    }
    pub fn label(&self) -> String {
        if self.is_all() {
            return "all".into();
        }
        self.0
            .iter()
            .map(|r| {
                if r.first == r.last {
                    r.first.to_string()
                } else {
                    format!("{}-{}", r.first, r.last)
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}
/// IPP uses input pages and ignores positions outside the source (RFC 8011 §5.2.7).
/// The explicit wrapper preserves strict array semantics for existing REST manifests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum RenderPages {
    Strict(PageRanges),
    Ipp { ipp: IppPageRanges },
}
impl Default for RenderPages {
    fn default() -> Self {
        Self::Strict(PageRanges::default())
    }
}
impl From<PageRanges> for RenderPages {
    fn from(value: PageRanges) -> Self {
        Self::Strict(value)
    }
}
impl RenderPages {
    pub fn is_all(&self) -> bool {
        matches!(self,Self::Strict(ranges) if ranges.is_all())
    }
    pub fn is_ipp(&self) -> bool {
        matches!(self, Self::Ipp { .. })
    }
    pub fn label(&self) -> String {
        match self {
            Self::Strict(r) => r.label(),
            Self::Ipp { ipp } => ipp.0.label(),
        }
    }
    pub fn ranges(&self) -> &[PageRange] {
        match self {
            Self::Strict(r) => r.ranges(),
            Self::Ipp { ipp } => ipp.0.ranges(),
        }
    }
    pub fn indices(&self, total: u32) -> Result<Vec<usize>, ValidationError> {
        match self {
            Self::Strict(r) => r.indices(total),
            Self::Ipp { ipp } => ipp.indices(total),
        }
    }
    pub fn argument(&self) -> String {
        format!(
            "--jrip-{}pages={}",
            if self.is_ipp() { "ipp-" } else { "" },
            self.label()
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct IppPageRanges(PageRanges);
impl IppPageRanges {
    pub fn new(ranges: Vec<PageRange>) -> Result<Self, ValidationError> {
        if ranges.is_empty() {
            return Err(ValidationError("invalid_page_ranges"));
        }
        Ok(Self(PageRanges::bounded(ranges, i32::MAX as u32)?))
    }
    pub fn indices(&self, total: u32) -> Result<Vec<usize>, ValidationError> {
        if !(1..=100).contains(&total) {
            return Err(ValidationError("page_range_out_of_bounds"));
        }
        Ok(self
            .0
            .ranges()
            .iter()
            .flat_map(|r| r.first..=r.last.min(total))
            .map(|n| n as usize - 1)
            .collect())
    }
}
impl<'de> Deserialize<'de> for IppPageRanges {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(Vec::<PageRange>::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
/// Validate the source-to-raster map before selecting pages for delivery.
pub fn source_mapping(
    manifest: &crate::Manifest,
    artifacts: &[crate::Artifact],
) -> Result<(Vec<u32>, u32), ValidationError> {
    if artifacts.is_empty()
        || artifacts.len() > 100
        || artifacts
            .iter()
            .enumerate()
            .any(|(i, a)| a.page != i as u32 + 1)
    {
        return Err(ValidationError("invalid_page_mapping"));
    }
    if manifest.page_ranges.is_all() {
        if artifacts
            .iter()
            .any(|a| a.source_page.is_some() || a.source_page_count.is_some())
        {
            return Err(ValidationError("invalid_page_mapping"));
        }
        return Ok((
            (1..=artifacts.len() as u32).collect(),
            artifacts.len() as u32,
        ));
    }
    let total = artifacts[0]
        .source_page_count
        .ok_or(ValidationError("invalid_page_mapping"))?;
    let expected: Vec<u32> = manifest
        .page_ranges
        .indices(total)?
        .into_iter()
        .map(|i| i as u32 + 1)
        .collect();
    if expected.len() != artifacts.len()
        || artifacts
            .iter()
            .zip(&expected)
            .any(|(a, n)| a.source_page != Some(*n) || a.source_page_count != Some(total))
    {
        return Err(ValidationError("invalid_page_mapping"));
    }
    Ok((expected, total))
}
pub fn delivery_indices(
    request: &PageRanges,
    mapping: &[u32],
    source_total: u32,
) -> Result<Vec<usize>, ValidationError> {
    if request.is_all() {
        return Ok((0..mapping.len()).collect());
    }
    request
        .indices(source_total)?
        .into_iter()
        .map(|i| {
            mapping
                .binary_search(&(i as u32 + 1))
                .map_err(|_| ValidationError("page_not_rendered"))
        })
        .collect()
}
impl<'de> Deserialize<'de> for PageRanges {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(Vec::<PageRange>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn selections_are_bounded_sorted_unique_and_canonical() {
        let selection: PageRanges = serde_json::from_value(
            json!([{"first":1,"last":1},{"first":2,"last":3},{"first":5,"last":5}]),
        )
        .unwrap();
        assert_eq!(selection.label(), "1-3,5");
        assert_eq!(selection.indices(5).unwrap(), vec![0, 1, 2, 4]);
        assert_eq!(selection.selected_count(), Some(4));
        assert_eq!(
            serde_json::to_value(selection).unwrap(),
            json!([{"first":1,"last":3},{"first":5,"last":5}])
        );
        let all = PageRanges::default();
        assert_eq!(all.indices(3).unwrap(), vec![0, 1, 2]);
        assert_eq!(all.label(), "all");
        assert_eq!(all.selected_count(), None);
        for total in [0, 101] {
            assert!(all.indices(total).is_err());
        }
    }
    #[test]
    fn invalid_ranges_never_expand_or_silently_clip() {
        for invalid in [
            json!(null),
            json!("1-3"),
            json!([{"first":0,"last":1}]),
            json!([{"first":2,"last":1}]),
            json!([{"first":1,"last":101}]),
            json!([{"first":2,"last":3},{"first":1,"last":1}]),
            json!([{"first":1,"last":3},{"first":3,"last":4}]),
            json!([{"first":1.5,"last":3}]),
            json!([{"first":true,"last":3}]),
            json!([{"first":1,"last":3,"unknown":1}]),
        ] {
            assert!(serde_json::from_value::<PageRanges>(invalid).is_err());
        }
        let selection: PageRanges = serde_json::from_value(json!([{"first":2,"last":3}])).unwrap();
        assert!(selection.indices(2).is_err());
        assert_eq!(selection.indices(3).unwrap(), vec![1, 2]);
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn selection_mapping_binds_source_positions_and_rejects_unrendered_pages() {
        let mut manifest: crate::Manifest=serde_json::from_value(json!({"name":"test.pdf","format":"application/pdf","dpi":72,"color_mode":"rgb","priority":50,"page_ranges":[{"first":2,"last":2},{"first":5,"last":5}]})).unwrap();
        manifest.validate().unwrap();
        let mut artifacts: Vec<crate::Artifact> = (1..=2)
            .map(|page| crate::Artifact {
                page,
                name: format!("page-{page:06}.tiff"),
                media_type: "image/tiff".into(),
                bytes: 8,
                sha256: "a".repeat(64),
                source_page: Some(if page == 1 { 2 } else { 5 }),
                source_page_count: Some(6),
            })
            .collect();
        let (map, total) = source_mapping(&manifest, &artifacts).unwrap();
        assert_eq!(map, vec![2, 5]);
        assert_eq!(total, 6);
        let selection = PageRanges::new(vec![PageRange { first: 5, last: 5 }]).unwrap();
        assert_eq!(delivery_indices(&selection, &map, total).unwrap(), vec![1]);
        let missing = PageRanges::new(vec![PageRange { first: 3, last: 3 }]).unwrap();
        assert_eq!(
            delivery_indices(&missing, &map, total).unwrap_err().0,
            "page_not_rendered"
        );
        artifacts[1].source_page = Some(3);
        assert!(source_mapping(&manifest, &artifacts).is_err());
        artifacts[1].source_page = Some(5);
        artifacts[1].source_page_count = Some(5);
        assert!(source_mapping(&manifest, &artifacts).is_err());
        manifest.page_ranges = PageRanges::default().into();
        assert!(source_mapping(&manifest, &artifacts).is_err());
        for artifact in &mut artifacts {
            artifact.source_page = None;
            artifact.source_page_count = None;
        }
        assert_eq!(
            source_mapping(&manifest, &artifacts).unwrap(),
            (vec![1, 2], 2)
        );
        let json = serde_json::to_value(&manifest).unwrap();
        assert!(json.get("page_ranges").is_none());
        assert!(
            serde_json::to_value(&artifacts[0])
                .unwrap()
                .get("sourcePage")
                .is_none()
        );
        manifest.page_ranges = selection.into();
        manifest.format = crate::DocumentFormat::Postscript;
        assert!(manifest.validate().is_err());
    }
}

#[cfg(test)]
mod ipp_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn ipp_ranges_clip_without_expanding_large_values_or_changing_strict_defaults() {
        for (input, expected) in [
            (json!({"ipp":[{"first":2,"last":2147483647}]}), vec![1, 2]),
            (json!({"ipp":[{"first":4,"last":2147483647}]}), vec![]),
            (
                json!({"ipp":[{"first":1,"last":1},{"first":3,"last":5}]}),
                vec![0, 2],
            ),
        ] {
            let selection: RenderPages = serde_json::from_value(input.clone()).unwrap();
            assert!(selection.is_ipp());
            assert!(!selection.is_all());
            assert_eq!(selection.indices(3).unwrap(), expected);
            assert_eq!(serde_json::to_value(selection).unwrap(), input);
        }
        for invalid in [
            json!({"ipp":[]}),
            json!({"ipp":[{"first":1,"last":2147483648_u64}]}),
            json!({"ipp":[{"first":1,"last":2}],"extra":true}),
            json!([{"first":1,"last":101}]),
        ] {
            assert!(serde_json::from_value::<RenderPages>(invalid).is_err());
        }
        let strict: RenderPages = serde_json::from_value(json!([{"first":2,"last":3}])).unwrap();
        assert!(strict.indices(2).is_err());
        assert!(!strict.is_ipp());
        assert_eq!(
            serde_json::to_value(RenderPages::default()).unwrap(),
            json!([])
        );
    }
}

/// Display bounds measured by the strict PDF inspector (CropBox/rotation/UserUnit).
pub fn validate_source_geometry(
    value: &serde_json::Value,
    count: u32,
) -> Result<(), ValidationError> {
    let pages = value
        .as_array()
        .ok_or(ValidationError("invalid_source_geometry"))?;
    if !(1..=100).contains(&count)
        || pages.len() != count as usize
        || pages.iter().enumerate().any(|(i, p)| {
            p["page"].as_u64() != Some(i as u64 + 1)
                || ["widthPoints", "heightPoints"].iter().any(|key| {
                    !p[*key]
                        .as_f64()
                        .is_some_and(|n| n.is_finite() && n > 0.0 && n <= 2834.6457)
                })
        })
    {
        return Err(ValidationError("invalid_source_geometry"));
    }
    Ok(())
}
#[cfg(test)]
mod source_geometry_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn bounded_complete_ordered_geometry_is_required() {
        let valid = json!([{"page":1,"widthPoints":72.0,"heightPoints":144.0},{"page":2,"widthPoints":144.0,"heightPoints":72.0}]);
        assert!(validate_source_geometry(&valid, 2).is_ok());
        for bad in [
            json!(null),
            json!([]),
            json!([{"page":2,"widthPoints":72,"heightPoints":72}]),
            json!([{"page":1,"widthPoints":0,"heightPoints":72}]),
            json!([{"page":1,"widthPoints":2835,"heightPoints":72}]),
            json!([{"page":1,"widthPoints":"72","heightPoints":72}]),
        ] {
            assert!(validate_source_geometry(&bad, 1).is_err());
        }
        assert!(validate_source_geometry(&valid, 1).is_err());
    }
}

/// Validate optional passive page/profile associations without inferring executed use.
pub fn validate_page_profile_declarations(
    analysis: &serde_json::Value,
    count: Option<u32>,
) -> Result<(), ValidationError> {
    let Some(mapping) = analysis
        .get("pageProfileDeclarations")
        .filter(|v| !v.is_null())
    else {
        return Ok(());
    };
    let invalid = ValidationError("invalid_page_profile_declarations");
    let pages = mapping["pages"]
        .as_array()
        .ok_or(ValidationError("invalid_page_profile_declarations"))?;
    let profiles = analysis["profiles"]
        .as_array()
        .ok_or(ValidationError("invalid_page_profile_declarations"))?;
    if analysis["format"] != "pdf"
        || mapping["profile"] != "org.jrip.pdf-page-profile-declarations-v1"
        || mapping["scope"] != "reachable-page-resources-not-executed-content"
        || mapping["executedUseVerified"] != false
        || !(1..=100).contains(&pages.len())
        || count.is_some_and(|n| n as usize != pages.len())
        || profiles.len() > 64
    {
        return Err(invalid);
    }
    for (index, page) in pages.iter().enumerate() {
        let refs = page["profileIndices"]
            .as_array()
            .ok_or(ValidationError("invalid_page_profile_declarations"))?;
        if page["page"].as_u64() != Some(index as u64 + 1)
            || !matches!(
                page["status"].as_str(),
                Some("DECLARATIONS_INSPECTED" | "INCOMPLETE")
            )
            || refs.len() > 64
            || refs.iter().any(|v| {
                !v.as_u64().is_some_and(|n| {
                    n < profiles.len() as u64 && profiles[n as usize]["kind"] == "PDF_ICC_BASED"
                })
            })
            || refs.windows(2).any(|w| w[0].as_u64() >= w[1].as_u64())
        {
            return Err(invalid);
        }
    }
    Ok(())
}

#[cfg(test)]
mod page_profile_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn declarations_reject_wrong_roles_indices_pages_and_execution_claims() {
        let valid = json!({"format":"pdf","profiles":[{"kind":"PDF_OUTPUT_INTENT"},{"kind":"PDF_ICC_BASED"}],"pageProfileDeclarations":{"profile":"org.jrip.pdf-page-profile-declarations-v1","scope":"reachable-page-resources-not-executed-content","executedUseVerified":false,"pages":[{"page":1,"profileIndices":[1],"status":"DECLARATIONS_INSPECTED"}]}});
        assert!(validate_page_profile_declarations(&valid, Some(1)).is_ok());
        assert!(validate_page_profile_declarations(&valid, Some(2)).is_err());
        for indices in [
            json!([0]),
            json!([2]),
            json!([-1]),
            json!([1, 1]),
            json!(["1"]),
        ] {
            let mut bad = valid.clone();
            bad["pageProfileDeclarations"]["pages"][0]["profileIndices"] = indices;
            assert!(validate_page_profile_declarations(&bad, Some(1)).is_err());
        }
        let mut bad = valid.clone();
        bad["pageProfileDeclarations"]["executedUseVerified"] = json!(true);
        assert!(validate_page_profile_declarations(&bad, None).is_err());
        assert!(validate_page_profile_declarations(&json!({"profiles":[]}), Some(1)).is_ok());
    }
}
