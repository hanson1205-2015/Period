use std::fs;
use std::io::Write;
use std::path::Path;

use sha2::{Digest, Sha256};

pub fn download(url: &str, dest: &Path) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("failed to download '{}': {}", url, e))?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| format!("failed to read '{}': {}", url, e))?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create directory {}: {}", parent.display(), e))?;
    }
    let mut file = fs::File::create(dest)
        .map_err(|e| format!("cannot create {}: {}", dest.display(), e))?;
    file.write_all(&bytes)
        .map_err(|e| format!("cannot write {}: {}", dest.display(), e))?;
    Ok(bytes)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn verify_checksum(bytes: &[u8], expected: &str) -> Result<(), String> {
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!("checksum mismatch: expected {} got {}", expected, actual))
    }
}
