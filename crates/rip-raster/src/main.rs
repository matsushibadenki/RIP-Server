fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        return Err(
            "Usage: rip-raster OUTPUT.pwg DPI rgb|cmyk|black [--duplex-normal=long-edge|short-edge | --duplex=long-edge|short-edge --sheet-back=normal|rotated|flipped|manual-tumble] PAGE1.tiff [PAGE2.tiff ...]".into(),
        );
    }
    let color = match args[2].as_str() {
        "rgb" => rip_raster::Color::DeviceRgb,
        "cmyk" => rip_raster::Color::DeviceCmyk,
        "black" => rip_raster::Color::Black,
        _ => return Err("Unknown device color space".into()),
    };
    let options = rip_raster::Options {
        dpi: args[1].parse()?,
        color,
    };
    let output = std::path::Path::new(&args[0]);
    if let Some(value) = args[3].strip_prefix("--duplex-normal=") {
        let binding = match value {
            "long-edge" => rip_raster::DuplexBinding::LongEdge,
            "short-edge" => rip_raster::DuplexBinding::ShortEdge,
            _ => return Err("Unknown normal duplex binding".into()),
        };
        rip_raster::convert_normal_duplex_file(&args[4..], &options, None, binding, output)?;
    } else if let Some(value) = args[3].strip_prefix("--duplex=") {
        let binding = match value {
            "long-edge" => rip_raster::DuplexBinding::LongEdge,
            "short-edge" => rip_raster::DuplexBinding::ShortEdge,
            _ => return Err("Unknown duplex binding".into()),
        };
        let sheet_back = args
            .get(4)
            .and_then(|v| v.strip_prefix("--sheet-back="))
            .ok_or("Explicit --sheet-back is required")?
            .parse()?;
        rip_raster::convert_duplex_file(
            &args[5..],
            &options,
            None,
            rip_raster::DuplexLayout {
                binding,
                sheet_back,
            },
            output,
        )?;
    } else {
        rip_raster::convert_file(&args[3..], &options, output)?;
    }
    Ok(())
}
