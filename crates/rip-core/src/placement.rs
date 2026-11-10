//! Paper placement in integer units; page geometry is quantized once to pixels.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaleMode {
    Actual,
    Fit,
    Custom,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Placement {
    pub scale_mode: ScaleMode,
    /// 10000 = 100%; used only by Custom.
    pub scale_basis_points: u32,
    /// Clockwise quarter turns.
    pub rotation: u16,
    /// 0/1/2: left/center/right and top/center/bottom.
    pub align_x: u8,
    pub align_y: u8,
    pub offset_x_microns: i32,
    pub offset_y_microns: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    pub paper_width: u32,
    pub paper_height: u32,
    pub image_width: u32,
    pub image_height: u32,
    pub x: i64,
    pub y: i64,
    pub clipped: bool,
}
impl Placement {
    pub fn validate(&self) -> Result<(), crate::ValidationError> {
        if !matches!(self.rotation, 0 | 90 | 180 | 270)
            || self.align_x > 2
            || self.align_y > 2
            || !(100..=40000).contains(&self.scale_basis_points)
            || self.offset_x_microns.unsigned_abs() > 1_000_000
            || self.offset_y_microns.unsigned_abs() > 1_000_000
        {
            return Err(crate::ValidationError("invalid_placement"));
        }
        Ok(())
    }
    pub fn geometry(
        &self,
        width: u32,
        height: u32,
        media: [u32; 2],
        dpi: u32,
    ) -> Result<Geometry, crate::ValidationError> {
        self.validate()?;
        if width == 0 || height == 0 || !(72..=2400).contains(&dpi) || media.contains(&0) {
            return Err(crate::ValidationError("invalid_placement"));
        }
        let pixel = |n: u32| ((u64::from(n) * u64::from(dpi) + 12700) / 25400) as u32;
        let pw = pixel(media[0]);
        let ph = pixel(media[1]);
        if pw == 0 || ph == 0 {
            return Err(crate::ValidationError("invalid_placement"));
        }
        let (rw, rh) = if self.rotation.is_multiple_of(180) {
            (width, height)
        } else {
            (height, width)
        };
        let scale = match self.scale_mode {
            ScaleMode::Actual => 1.,
            ScaleMode::Fit => (pw as f64 / rw as f64).min(ph as f64 / rh as f64),
            ScaleMode::Custom => self.scale_basis_points as f64 / 10000.,
        };
        let round = |v: f64| (v + 0.5).floor() as i64;
        let iw = round(rw as f64 * scale).max(1) as u32;
        let ih = round(rh as f64 * scale).max(1) as u32;
        let x = round((pw as f64 - iw as f64) * self.align_x as f64 / 2.)
            + round(self.offset_x_microns as f64 * dpi as f64 / 25400.);
        let y = round((ph as f64 - ih as f64) * self.align_y as f64 / 2.)
            + round(self.offset_y_microns as f64 * dpi as f64 / 25400.);
        Ok(Geometry {
            paper_width: pw,
            paper_height: ph,
            image_width: iw,
            image_height: ih,
            x,
            y,
            clipped: x < 0 || y < 0 || x + iw as i64 > pw as i64 || y + ih as i64 > ph as i64,
        })
    }
}
