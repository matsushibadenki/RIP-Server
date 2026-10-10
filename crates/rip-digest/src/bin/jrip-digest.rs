use std::io::Read;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path] = args.as_slice() else {
        return Err("usage: jrip-digest INPUT.json".into());
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err("input exceeds 1 MiB".into());
    }
    println!("{}", rip_digest::digest(&rip_digest::parse(&bytes)?)?);
    Ok(())
}
