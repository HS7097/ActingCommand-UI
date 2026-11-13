// SPDX-License-Identifier: GPL-3.0-only
//! SHA-256 of whole files and streams, as lowercase hex: the one way every byte the wizard lays
//! down is checked against what its maker recorded.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// Whether `text` is a SHA-256 written as 64 hex digits (either case).
pub fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A writer that only hashes and counts what goes through it.
pub struct Hasher {
    inner: Sha256,
    bytes: u64,
}

impl Hasher {
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
            bytes: 0,
        }
    }

    /// The lowercase hex digest and the byte count.
    pub fn finish(self) -> (String, u64) {
        (hex(&self.inner.finalize()), self.bytes)
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for Hasher {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.update(buffer);
        self.bytes += buffer.len() as u64;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The SHA-256 and the size of the file at `path`, streamed; `progress` hears each chunk's size.
pub fn sha256_file(path: &Path, progress: &mut dyn FnMut(u64)) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.write_all(&buffer[..read])?;
        progress(read as u64);
    }
    Ok(hasher.finish())
}
