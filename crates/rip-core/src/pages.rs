//! Canonical, one-based selection of already-rendered pages.
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
        if ranges.len() > 100 {
            return Err(ValidationError("invalid_page_ranges"));
        }
        let mut canonical: Vec<PageRange> = Vec::new();
        for range in ranges {
            if range.first == 0 || range.last < range.first || range.last > 100 {
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
