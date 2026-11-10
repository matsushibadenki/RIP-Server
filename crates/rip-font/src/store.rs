use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub path: PathBuf,
    pub digest: String,
    pub fonts: Vec<Font>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    fonts: Vec<Font>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Current {
    snapshot: uuid::Uuid,
    sha256: String,
}
fn canonical(path: &Path) -> Result<()> {
    if std::fs::canonicalize(path).map_err(|_| "font_store_unavailable")? != path {
        return Err("invalid_font_store".into());
    }
    Ok(())
}
fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    canonical(path)?;
    let mut b = Vec::new();
    File::open(path)
        .map_err(|_| "font_store_unavailable")?
        .take(limit + 1)
        .read_to_end(&mut b)
        .map_err(|_| "font_store_unavailable")?;
    if b.len() as u64 > limit {
        return Err("invalid_font_store".into());
    }
    Ok(b)
}
pub fn current(root: &Path) -> Result<Option<Snapshot>> {
    let root = std::fs::canonicalize(root).map_err(|_| "font_store_unavailable")?;
    let base = root.join("font-store");
    if !base.exists() {
        return Ok(None);
    }
    canonical(&base)?;
    let index = base.join("current.json");
    if !index.exists() {
        return Ok(None);
    }
    let pointer: Current =
        serde_json::from_slice(&read(&index, 1024)?).map_err(|_| "invalid_font_store")?;
    let path = base.join("snapshots").join(pointer.snapshot.to_string());
    canonical(&path)?;
    let bytes = read(&path.join("manifest.json"), 256 * 1024)?;
    if hash(&bytes) != pointer.sha256 {
        return Err("font_store_integrity_mismatch".into());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| "invalid_font_store")?;
    if manifest.version != 1 || manifest.fonts.len() > 128 {
        return Err("invalid_font_store".into());
    }
    for f in &manifest.fonts {
        if !safe_name(&f.name)
            || f.file != format!("{}.{}", f.sha256, f.format)
            || !matches!(f.format.as_str(), "ttf" | "otf")
            || f.bytes > MAX_BYTES as u64
        {
            return Err("invalid_font_store".into());
        }
    }
    Ok(Some(Snapshot {
        path,
        digest: pointer.sha256,
        fonts: manifest.fonts,
    }))
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "font_store_write_failed")?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|_| "font_store_write_failed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444))
            .map_err(|_| "font_store_write_failed")?;
    }
    Ok(())
}
pub fn install(root: &Path, converted: Vec<Converted>) -> Result<Snapshot> {
    install_expected(root, converted, None)
}
pub fn install_expected(
    root: &Path,
    converted: Vec<Converted>,
    expected: Option<&str>,
) -> Result<Snapshot> {
    let root = std::fs::canonicalize(root).map_err(|_| "font_store_unavailable")?;
    let base = root.join("font-store");
    std::fs::create_dir_all(base.join("snapshots")).map_err(|_| "font_store_write_failed")?;
    canonical(&base)?;
    canonical(&base.join("snapshots"))?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(base.join("install.lock"))
        .map_err(|_| "font_store_write_failed")?;
    lock.try_lock().map_err(|_| "font_install_busy")?;
    let old = current(&root)?;
    if expected.is_some_and(|expected| {
        old.as_ref().map(|s| s.digest.as_str()).unwrap_or("none") != expected
    }) {
        return Err("font_snapshot_conflict".into());
    }
    let mut fonts = old.as_ref().map(|s| s.fonts.clone()).unwrap_or_default();
    let mut changed = false;
    for c in &converted {
        if let Some(old) = fonts.iter().find(|f| f.name == c.font.name) {
            if old.sha256 != c.font.sha256 {
                return Err("font_name_conflict".into());
            }
        } else {
            fonts.push(c.font.clone());
            changed = true;
        }
    }
    if !changed {
        return old.ok_or("invalid_font_store".into());
    }
    if fonts.len() > 128 || fonts.iter().map(|f| f.bytes).sum::<u64>() > 128 * 1024 * 1024 {
        return Err("font_store_limit".into());
    }
    fonts.sort_by(|a, b| a.name.cmp(&b.name));
    let id = uuid::Uuid::new_v4();
    let path = base.join("snapshots").join(id.to_string());
    std::fs::create_dir(&path).map_err(|_| "font_store_write_failed")?;
    struct Unpublished(PathBuf, bool);
    impl Drop for Unpublished {
        fn drop(&mut self) {
            if !self.1 {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
    let mut staged = Unpublished(path.clone(), false);
    for f in &fonts {
        if let Some(c) = converted.iter().find(|c| c.font.sha256 == f.sha256) {
            write(&path.join(&f.file), &c.bytes)?;
        } else {
            let previous = old.as_ref().ok_or("invalid_font_store")?.path.join(&f.file);
            let bytes = read(&previous, MAX_BYTES as u64)?;
            if hash(&bytes) != f.sha256 {
                return Err("font_store_integrity_mismatch".into());
            }
            std::fs::hard_link(&previous, path.join(&f.file))
                .map_err(|_| "font_store_write_failed")?;
        }
    }
    let bytes = serde_json::to_vec(&Manifest {
        version: 1,
        fonts: fonts.clone(),
    })
    .map_err(|_| "invalid_font_store")?;
    if bytes.len() > 256 * 1024 {
        return Err("font_store_limit".into());
    }
    let digest = hash(&bytes);
    write(&path.join("manifest.json"), &bytes)?;
    let mut map = String::from("(Fontmap.GS) .runlibfile\n");
    for f in &fonts {
        map += &format!("/{} (/jrip-fonts/{}) ;\n", f.name, f.file);
    }
    write(&path.join("Fontmap"), map.as_bytes())?;
    File::open(&path)
        .and_then(|f| f.sync_all())
        .map_err(|_| "font_store_write_failed")?;
    let pointer = serde_json::to_vec(&Current {
        snapshot: id,
        sha256: digest.clone(),
    })
    .map_err(|_| "invalid_font_store")?;
    let temp = base.join(format!("current-{id}.part"));
    write(&temp, &pointer)?;
    std::fs::rename(temp, base.join("current.json")).map_err(|_| "font_store_write_failed")?;
    staged.1 = true;
    File::open(&base)
        .and_then(|f| f.sync_all())
        .map_err(|_| "font_store_write_failed")?;
    Ok(Snapshot {
        path,
        digest,
        fonts,
    })
}
