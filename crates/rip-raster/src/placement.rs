//! Bounded bilinear placement before device sheet-back transforms.
use crate::Error;
use rip_core::placement::Placement;
pub(crate) fn compose(
    source: &[u8],
    dimensions: [u32; 2],
    channels: usize,
    white: u8,
    placement: &Placement,
    media: [u32; 2],
    dpi: u32,
) -> Result<(Vec<u8>, u32, u32), Error> {
    let [width, height] = dimensions;
    let g = placement
        .geometry(width, height, media, dpi)
        .map_err(|_| Error::Unsupported)?;
    let length = u64::from(g.paper_width) * u64::from(g.paper_height) * channels as u64;
    if length > 64 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let mut out = vec![white; length as usize];
    let (rw, rh) = if placement.rotation.is_multiple_of(180) {
        (width, height)
    } else {
        (height, width)
    };
    let sample = |x: u32, y: u32, c: usize| {
        let (sx, sy) = match placement.rotation {
            0 => (x, y),
            90 => (y, height - 1 - x),
            180 => (width - 1 - x, height - 1 - y),
            270 => (width - 1 - y, x),
            _ => unreachable!(),
        };
        source[(sy as usize * width as usize + sx as usize) * channels + c] as f64
    };
    for y in g.y.max(0)..(g.y + g.image_height as i64).min(g.paper_height as i64) {
        let fy = (((y - g.y) as f64 + 0.5) * rh as f64 / g.image_height as f64 - 0.5)
            .clamp(0., rh as f64 - 1.);
        let y0 = fy.floor() as u32;
        let y1 = (y0 + 1).min(rh - 1);
        let ty = fy - y0 as f64;
        for x in g.x.max(0)..(g.x + g.image_width as i64).min(g.paper_width as i64) {
            let fx = (((x - g.x) as f64 + 0.5) * rw as f64 / g.image_width as f64 - 0.5)
                .clamp(0., rw as f64 - 1.);
            let x0 = fx.floor() as u32;
            let x1 = (x0 + 1).min(rw - 1);
            let tx = fx - x0 as f64;
            for c in 0..channels {
                let top = sample(x0, y0, c) * (1. - tx) + sample(x1, y0, c) * tx;
                let bottom = sample(x0, y1, c) * (1. - tx) + sample(x1, y1, c) * tx;
                out[(y as usize * g.paper_width as usize + x as usize) * channels + c] =
                    (top * (1. - ty) + bottom * ty).round() as u8;
            }
        }
    }
    Ok((out, g.paper_width, g.paper_height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rip_core::placement::{Placement, ScaleMode};
    fn p(rotation: u16) -> Placement {
        Placement {
            scale_mode: ScaleMode::Actual,
            scale_basis_points: 10000,
            rotation,
            align_x: 0,
            align_y: 0,
            offset_x_microns: 0,
            offset_y_microns: 0,
        }
    }
    #[test]
    fn asymmetric_pixels_rotate_clockwise_and_signed_offsets_clip_to_white() {
        let src = [1, 2, 3, 4, 5, 6];
        for (rotation, media, expected) in [
            (0, [200, 300], vec![1, 2, 3, 4, 5, 6]),
            (90, [300, 200], vec![5, 3, 1, 6, 4, 2]),
            (180, [200, 300], vec![6, 5, 4, 3, 2, 1]),
            (270, [300, 200], vec![2, 4, 6, 1, 3, 5]),
        ] {
            let (out, _, _) = compose(&src, [2, 3], 1, 255, &p(rotation), media, 254).unwrap();
            assert_eq!(out, expected);
        }
        let mut placement = p(0);
        placement.offset_x_microns = 100;
        assert_eq!(
            compose(&src, [2, 3], 1, 255, &placement, [200, 300], 254)
                .unwrap()
                .0,
            [255, 1, 255, 3, 255, 5]
        );
        placement.offset_x_microns = -100;
        assert_eq!(
            compose(&src, [2, 3], 1, 255, &placement, [200, 300], 254)
                .unwrap()
                .0,
            [2, 255, 4, 255, 6, 255]
        );
    }
    #[test]
    fn fit_custom_alignment_channel_whites_and_limits() {
        let mut placement = p(0);
        placement.scale_mode = ScaleMode::Fit;
        placement.align_x = 1;
        placement.align_y = 1;
        let g = placement.geometry(2, 3, [400, 400], 254).unwrap();
        assert_eq!((g.image_width, g.image_height, g.x, g.y), (3, 4, 1, 0));
        assert!(!g.clipped);
        placement.scale_mode = ScaleMode::Custom;
        placement.scale_basis_points = 5000;
        for (channels, white) in [(3, 255), (4, 0)] {
            let src = vec![90; 4 * 4 * channels];
            let (out, w, h) =
                compose(&src, [4, 4], channels, white, &placement, [400, 400], 254).unwrap();
            assert_eq!((w, h), (4, 4));
            assert_eq!(&out[..channels], vec![white; channels]);
            assert_eq!(out[(w as usize + 1) * channels], 90);
        }
        assert!(compose(&[1], [1, 1], 1, 255, &p(0), [1_000_000, 1_000_000], 2400).is_err());
        placement.rotation = 91;
        assert!(placement.validate().is_err());
    }
}
