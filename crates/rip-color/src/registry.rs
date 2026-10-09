//! Immutable, content-addressed profile storage for administrator-owned roots.
use crate::{Error, MAX_PROFILE, hash, validate_profile};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRecord {
    pub schema_version: u32,
    pub sha256: String,
    pub size_bytes: usize,
    pub color_space: String,
    pub device_link: bool,
}

pub struct Registry {
    root: PathBuf,
}
impl Registry {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, Error> {
        std::fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: std::fs::canonicalize(root)?,
        })
    }
    fn path(&self, digest: &str) -> Result<PathBuf, Error> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Profile);
        }
        Ok(self.root.join(format!("{digest}.icc")))
    }
    pub fn register(&self, bytes: &[u8]) -> Result<ProfileRecord, Error> {
        validate_profile(bytes)?;
        let digest = hash(bytes);
        let destination = self.path(&digest)?;
        let mut staging = tempfile::NamedTempFile::new_in(&self.root)?;
        staging.write_all(bytes)?;
        staging.as_file().sync_all()?;
        match staging.persist_noclobber(&destination) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.read(&digest)?;
            }
            Err(error) => return Err(Error::Io(error.error)),
        }
        Ok(record(bytes))
    }
    pub fn read(&self, digest: &str) -> Result<Vec<u8>, Error> {
        let path = self.path(digest)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.len() > MAX_PROFILE as u64 {
            return Err(Error::Profile);
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_PROFILE as u64 + 1)
            .read_to_end(&mut bytes)?;
        validate_profile(&bytes)?;
        if hash(&bytes) != digest {
            return Err(Error::Profile);
        }
        Ok(bytes)
    }
    pub fn describe(&self, digest: &str) -> Result<ProfileRecord, Error> {
        Ok(record(&self.read(digest)?))
    }
}
fn record(bytes: &[u8]) -> ProfileRecord {
    ProfileRecord {
        schema_version: 1,
        sha256: hash(bytes),
        size_bytes: bytes.len(),
        color_space: String::from_utf8_lossy(&bytes[16..20]).trim().to_string(),
        device_link: &bytes[12..16] == b"link",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_is_idempotent_and_detects_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::open(directory.path()).unwrap();
        let icc = lcms2::Profile::new_srgb().icc().unwrap();
        let first = registry.register(&icc).unwrap();
        assert_eq!(first.sha256, registry.register(&icc).unwrap().sha256);
        assert_eq!(registry.read(&first.sha256).unwrap(), icc);
        assert!(registry.read("../escape").is_err());
        std::fs::write(registry.path(&first.sha256).unwrap(), b"corrupted").unwrap();
        assert!(registry.read(&first.sha256).is_err());
        assert!(registry.register(&icc).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn registry_rejects_profile_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::open(directory.path().join("registry")).unwrap();
        let icc = lcms2::Profile::new_srgb().icc().unwrap();
        let external = directory.path().join("external.icc");
        std::fs::write(&external, &icc).unwrap();
        let digest = hash(&icc);
        std::os::unix::fs::symlink(external, registry.path(&digest).unwrap()).unwrap();
        assert!(registry.read(&digest).is_err());
        assert!(registry.register(&icc).is_err());
    }
}
