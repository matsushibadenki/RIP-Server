fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        return Err(
            "Usage: rip-raster OUTPUT.pwg DPI rgb|cmyk|black PAGE1.tiff [PAGE2.tiff ...]".into(),
        );
    }
    let color = match args[2].as_str() {
        "rgb" => rip_raster::Color::DeviceRgb,
        "cmyk" => rip_raster::Color::DeviceCmyk,
        "black" => rip_raster::Color::Black,
        _ => return Err("Unknown device color space".into()),
    };
    rip_raster::convert_file(
        &args[3..],
        &rip_raster::Options {
            dpi: args[1].parse()?,
            color,
        },
        std::path::Path::new(&args[0]),
    )?;
    Ok(())
}
