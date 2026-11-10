//! Bounded TIFF-to-PWG Raster encoder. Device colors are preserved; no ICC conversion.
pub mod describe;
mod placement;
pub mod preview;
use std::{
    io::{Read, Seek, Write},
    path::Path,
};
use tiff::{
    ColorType,
    decoder::{Decoder, DecodingResult, Limits},
    tags::Tag,
};
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Tiff(#[from] tiff::TiffError),
    #[error("unsupported_raster_layout_or_color")]
    Unsupported,
    #[error("raster_limit_exceeded")]
    Limit,
}
#[derive(Clone, Copy, Debug)]
pub enum Color {
    DeviceRgb,
    /// Samples are already in standard sRGB; no implicit color conversion.
    Srgb,
    DeviceCmyk,
    Black,
}
impl Color {
    fn layout(self) -> (usize, u32, ColorType) {
        match self {
            Self::DeviceRgb => (3, 1, ColorType::RGB(8)),
            Self::Srgb => (3, 19, ColorType::RGB(8)),
            Self::DeviceCmyk => (4, 6, ColorType::CMYK(8)),
            Self::Black => (1, 3, ColorType::Gray(8)),
        }
    }
}
/// Binding for devices advertising pwg-raster-document-sheet-back=normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplexBinding {
    LongEdge,
    ShortEdge,
}
impl DuplexBinding {
    fn tumble(self) -> u32 {
        u32::from(self == Self::ShortEdge)
    }
}
pub use rip_core::print::SheetBack;
#[derive(Debug, Clone, Copy)]
pub struct DuplexLayout {
    pub binding: DuplexBinding,
    pub sheet_back: SheetBack,
}
impl DuplexLayout {
    /// Front pages are always identity; backside values are PWG 5102.4 Table 9.
    fn transforms(self, backside: bool) -> (u32, u32) {
        if !backside {
            return (1, 1);
        }
        match (self.binding, self.sheet_back) {
            (DuplexBinding::LongEdge, SheetBack::Flipped) => (1, u32::MAX),
            (DuplexBinding::ShortEdge, SheetBack::Flipped) => (u32::MAX, 1),
            (DuplexBinding::LongEdge, SheetBack::Rotated)
            | (DuplexBinding::ShortEdge, SheetBack::ManualTumble) => (u32::MAX, u32::MAX),
            _ => (1, 1),
        }
    }
}
#[derive(Clone, Copy)]
struct PageLayout {
    total: u32,
    placement: Option<(rip_core::placement::Placement, [u32; 2])>,
    duplex: Option<DuplexLayout>,
    backside: bool,
}
impl PageLayout {
    fn transforms(self) -> (u32, u32) {
        self.duplex.map_or((1, 1), |d| d.transforms(self.backside))
    }
}
pub struct Options {
    pub dpi: u32,
    pub color: Color,
}
struct Bounded<'a, W> {
    writer: &'a mut W,
    bytes: usize,
}
impl<W: Write> Write for Bounded<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes + bytes.len() > 64 * 1024 * 1024 {
            return Err(std::io::Error::other("PWG output exceeds 32 MiB"));
        }
        let n = self.writer.write(bytes)?;
        self.bytes += n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
fn integer(header: &mut [u8], offset: usize, value: u32) {
    header[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
/// Paths are in caller-specified page order. Write to private staging: failures may leave a prefix.
pub fn convert<W: Write>(
    pages: &[impl AsRef<Path>],
    options: &Options,
    writer: &mut W,
) -> Result<(), Error> {
    convert_registered(pages, options, None, writer)
}
/// Only a verified ColorManagedArtifact may authorize an embedded ICC hash.
pub fn convert_registered<W: Write>(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    writer: &mut W,
) -> Result<(), Error> {
    convert_layout(pages, options, expected_icc, None, None, writer)
}
/// Encode duplex only in the native `normal` sheet-back coordinate system.
/// The caller must verify the device's sheet-back mode before using this output.
/// Pages remain in supplied order; no implicit blank back page or raster duplication.
/// Write to private staging because an error may leave an incomplete prefix.
pub fn convert_normal_duplex<W: Write>(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    binding: DuplexBinding,
    writer: &mut W,
) -> Result<(), Error> {
    convert_duplex(
        pages,
        options,
        expected_icc,
        DuplexLayout {
            binding,
            sheet_back: SheetBack::Normal,
        },
        writer,
    )
}
/// Render all bitmaps in the device-native sheet-back coordinates.
pub fn convert_duplex<W: Write>(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    layout: DuplexLayout,
    writer: &mut W,
) -> Result<(), Error> {
    convert_layout(pages, options, expected_icc, Some(layout), None, writer)
}
pub fn convert_positioned<W: Write>(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    placement: &rip_core::placement::Placement,
    media: [u32; 2],
    duplex: Option<DuplexLayout>,
    writer: &mut W,
) -> Result<(), Error> {
    placement.validate().map_err(|_| Error::Unsupported)?;
    convert_layout(
        pages,
        options,
        expected_icc,
        duplex,
        Some((*placement, media)),
        writer,
    )
}
fn convert_layout<W: Write>(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    duplex: Option<DuplexLayout>,
    placement: Option<(rip_core::placement::Placement, [u32; 2])>,
    writer: &mut W,
) -> Result<(), Error> {
    if pages.is_empty() || pages.len() > 100 || !(72..=2400).contains(&options.dpi) {
        return Err(Error::Limit);
    }
    let mut writer = Bounded { writer, bytes: 0 };
    writer.write_all(b"RaS2")?;
    let mut decoded = 0usize;
    for (index, path) in pages.iter().enumerate() {
        page(
            std::fs::File::open(path)?,
            options,
            expected_icc,
            PageLayout {
                total: pages.len() as u32,
                placement,
                duplex,
                backside: index % 2 == 1,
            },
            &mut decoded,
            &mut writer,
        )?;
    }
    writer.flush()?;
    Ok(())
}
fn page<R: Read + Seek, W: Write>(
    reader: R,
    options: &Options,
    expected_icc: Option<&str>,
    layout: PageLayout,
    decoded: &mut usize,
    writer: &mut W,
) -> Result<(), Error> {
    let mut limits = Limits::default();
    limits.decoding_buffer_size = 64 * 1024 * 1024;
    limits.intermediate_buffer_size = 64 * 1024 * 1024;
    let mut decoder = Decoder::new(reader)?.with_limits(limits);
    if decoder.more_images()
        || decoder
            .find_tag_unsigned::<u16>(Tag::Orientation)?
            .is_some_and(|o| o != 1)
    {
        return Err(Error::Unsupported);
    }
    use sha2::{Digest, Sha256};
    let embedded = decoder
        .find_tag(Tag::Unknown(34675))?
        .map(|value| value.into_u8_vec())
        .transpose()?;
    match (embedded, expected_icc) {
        (None, None) => {}
        (Some(bytes), Some(expected))
            if bytes.len() <= 4 * 1024 * 1024
                && format!("{:x}", Sha256::digest(&bytes)) == expected => {}
        _ => return Err(Error::Unsupported),
    }
    if let Some(unit) = decoder.find_tag_unsigned::<u16>(Tag::ResolutionUnit)? {
        if unit == 2 || unit == 3 {
            for tag in [Tag::XResolution, Tag::YResolution] {
                let tiff::decoder::ifd::Value::Rational(numerator, denominator) =
                    decoder.get_tag(tag)?
                else {
                    return Err(Error::Unsupported);
                };
                if denominator == 0 {
                    return Err(Error::Unsupported);
                }
                let dpi = f64::from(numerator) / f64::from(denominator)
                    * if unit == 3 { 2.54 } else { 1.0 };
                if !dpi.is_finite() || (dpi - options.dpi as f64).abs() > 0.01 {
                    return Err(Error::Unsupported);
                }
            }
        } else if unit != 1 {
            return Err(Error::Unsupported);
        }
    }
    let (mut width, mut height) = decoder.dimensions()?;
    let (channels, space, color) = options.color.layout();
    if decoder.colortype()? != color || width == 0 || height == 0 {
        return Err(Error::Unsupported);
    }
    let line = (width as usize).checked_mul(channels).ok_or(Error::Limit)?;
    let size = line.checked_mul(height as usize).ok_or(Error::Limit)?;
    *decoded = decoded.checked_add(size).ok_or(Error::Limit)?;
    if size > 64 * 1024 * 1024 || *decoded > 1024 * 1024 * 1024 {
        return Err(Error::Limit);
    }
    let DecodingResult::U8(mut pixels) = decoder.read_image()? else {
        return Err(Error::Unsupported);
    };
    if pixels.len() != size {
        return Err(Error::Unsupported);
    }
    if let Some((placement, media)) = layout.placement {
        let white = if matches!(options.color, Color::DeviceCmyk) {
            0
        } else {
            255
        };
        (pixels, width, height) = placement::compose(
            &pixels,
            [width, height],
            channels,
            white,
            &placement,
            media,
            options.dpi,
        )?;
    }
    if layout.placement.is_some() {
        *decoded = decoded.checked_add(pixels.len()).ok_or(Error::Limit)?;
        if *decoded > 1024 * 1024 * 1024 {
            return Err(Error::Limit);
        }
    }
    let line = width as usize * channels;
    if matches!(options.color, Color::Black) {
        for value in &mut pixels {
            *value = 255 - *value;
        }
    }
    let (cross, feed) = layout.transforms();
    if feed == u32::MAX {
        for y in 0..height as usize / 2 {
            for x in 0..line {
                pixels.swap(y * line + x, (height as usize - 1 - y) * line + x);
            }
        }
    }
    if cross == u32::MAX {
        for row in pixels.chunks_exact_mut(line) {
            for x in 0..width as usize / 2 {
                for c in 0..channels {
                    row.swap(x * channels + c, (width as usize - 1 - x) * channels + c);
                }
            }
        }
    }
    let mut header = [0u8; 1796];
    header[..9].copy_from_slice(b"PwgRaster");
    for (offset, value) in [
        (272, u32::from(layout.duplex.is_some())),
        (368, layout.duplex.map_or(0, |d| d.binding.tumble())),
        (276, options.dpi),
        (280, options.dpi),
        (340, 1),
        (
            352,
            (((width as u64 * 72 + options.dpi as u64 / 2) / options.dpi as u64) as u32).max(1),
        ),
        (
            356,
            (((height as u64 * 72 + options.dpi as u64 / 2) / options.dpi as u64) as u32).max(1),
        ),
        (372, width),
        (376, height),
        (384, 8),
        (388, channels as u32 * 8),
        (392, line as u32),
        (400, space),
        (420, channels as u32),
        (452, layout.total),
        (456, layout.transforms().0),
        (460, layout.transforms().1),
        (472, width),
        (476, height),
    ] {
        integer(&mut header, offset, value);
    }
    writer.write_all(&header)?;
    // Emit one row at a time, using literal runs of <=128 whole pixels (no lossy conversion).
    for row in pixels.chunks_exact(line) {
        writer.write_all(&[0])?;
        for block in row.chunks(128 * channels) {
            let count = block.len() / channels;
            if count == 1 {
                writer.write_all(&[0])?;
            } else {
                writer.write_all(&[(257 - count) as u8])?;
            }
            writer.write_all(block)?;
        }
    }
    Ok(())
}
/// Publish atomically without replacing existing artifacts; error paths remove staging files.
pub fn convert_file(
    pages: &[impl AsRef<Path>],
    options: &Options,
    output: &Path,
) -> Result<(), Error> {
    publish_file(pages, options, None, None, output)
}
/// Atomic normal-sheet-back duplex publication, without overwriting an existing file.
pub fn convert_normal_duplex_file(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    binding: DuplexBinding,
    output: &Path,
) -> Result<(), Error> {
    publish_file(
        pages,
        options,
        expected_icc,
        Some(DuplexLayout {
            binding,
            sheet_back: SheetBack::Normal,
        }),
        output,
    )
}
pub fn convert_duplex_file(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    layout: DuplexLayout,
    output: &Path,
) -> Result<(), Error> {
    publish_file(pages, options, expected_icc, Some(layout), output)
}
fn publish_file(
    pages: &[impl AsRef<Path>],
    options: &Options,
    expected_icc: Option<&str>,
    duplex: Option<DuplexLayout>,
    output: &Path,
) -> Result<(), Error> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staging = tempfile::NamedTempFile::new_in(parent)?;
    convert_layout(
        pages,
        options,
        expected_icc,
        duplex,
        None,
        staging.as_file_mut(),
    )?;
    staging.as_file().sync_all()?;
    staging
        .persist_noclobber(output)
        .map_err(|e| Error::Io(e.error))?;
    Ok(())
}

#[derive(Debug)]
pub struct PwgPage {
    pub offset: usize,
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub document_type: &'static str,
}
/// Validate all page headers and compressed scanlines without allocating decoded pixels.
pub fn inspect(bytes: &[u8]) -> Result<Vec<PwgPage>, Error> {
    inspect_layout(bytes, None)
}
/// Validate the complete stream against an explicit duplex binding and normal sheet-back.
/// The default inspector deliberately continues rejecting duplex streams.
pub fn inspect_normal_duplex(bytes: &[u8], binding: DuplexBinding) -> Result<Vec<PwgPage>, Error> {
    inspect_duplex(
        bytes,
        DuplexLayout {
            binding,
            sheet_back: SheetBack::Normal,
        },
    )
}
pub fn inspect_duplex(bytes: &[u8], layout: DuplexLayout) -> Result<Vec<PwgPage>, Error> {
    inspect_layout(bytes, Some(layout))
}
fn inspect_layout(bytes: &[u8], duplex: Option<DuplexLayout>) -> Result<Vec<PwgPage>, Error> {
    if bytes.len() > 64 * 1024 * 1024 || bytes.get(..4) != Some(b"RaS2") {
        return Err(Error::Unsupported);
    }
    let mut offset = 4usize;
    let mut pages = Vec::new();
    let mut decoded = 0u64;
    while offset < bytes.len() {
        let header = bytes.get(offset..offset + 1796).ok_or(Error::Unsupported)?;
        let u = |n| u32::from_be_bytes(header[n..n + 4].try_into().unwrap());
        let width = u(372);
        let height = u(376);
        let dpi = u(276);
        let (channels, document_type) = match u(400) {
            1 => (3, "rgb_8"),
            19 => (3, "srgb_8"),
            3 => (1, "black_8"),
            6 => (4, "cmyk_8"),
            _ => return Err(Error::Unsupported),
        };
        let size = width as u64 * height as u64 * channels;
        decoded = decoded.checked_add(size).ok_or(Error::Limit)?;
        if width == 0
            || height == 0
            || size > 64 * 1024 * 1024
            || decoded > 1024 * 1024 * 1024
            || !(72..=2400).contains(&dpi)
            || u(280) != dpi
            || u(384) != 8
            || u(388) != channels as u32 * 8
            || u(392) as u64 != width as u64 * channels
            || u(396) != 0
            || u(420) as u64 != channels
            || u(272) != u32::from(duplex.is_some())
            || u(368) != duplex.map_or(0, |d| d.binding.tumble())
            || u(456)
                != duplex
                    .map_or((1, 1), |d| d.transforms(pages.len() % 2 == 1))
                    .0
            || u(460)
                != duplex
                    .map_or((1, 1), |d| d.transforms(pages.len() % 2 == 1))
                    .1
            || pages.len() >= 100
        {
            return Err(Error::Unsupported);
        }
        pages.push(PwgPage {
            offset,
            width,
            height,
            dpi,
            document_type,
        });
        offset += 1796;
        let mut rows = 0u32;
        while rows < height {
            let repeat = *bytes.get(offset).ok_or(Error::Unsupported)? as u32 + 1;
            offset += 1;
            if repeat > height - rows {
                return Err(Error::Unsupported);
            }
            rows += repeat;
            let mut pixels = 0u32;
            while pixels < width {
                let code = *bytes.get(offset).ok_or(Error::Unsupported)? as u32;
                offset += 1;
                if code == 128 {
                    return Err(Error::Unsupported);
                }
                let count = if code < 128 { code + 1 } else { 257 - code };
                if count > width - pixels {
                    return Err(Error::Unsupported);
                }
                pixels += count;
                let length = if code < 128 {
                    channels as usize
                } else {
                    count as usize * channels as usize
                };
                bytes
                    .get(offset..offset + length)
                    .ok_or(Error::Unsupported)?;
                offset += length;
            }
        }
    }
    if pages.is_empty() {
        return Err(Error::Unsupported);
    }
    Ok(pages)
}
