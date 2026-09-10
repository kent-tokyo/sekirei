//! Bounded inspection of external YaneuraOu-style SFNN headers.
//!
//! The header is self-describing, but the parameter blocks after it are
//! architecture-specific. This module therefore validates only the header;
//! inference compatibility remains a separate adapter gate.

use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::Path;

const HEADER_BYTES: u64 = 12;
const MAX_ARCHITECTURE_BYTES: u32 = 4096;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Header metadata read from an external SFNN artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SfnnHeader {
    /// Format version written by the producer.
    pub version: u32,
    /// Producer architecture hash from the file header.
    pub hash: u32,
    /// UTF-8 architecture descriptor.
    pub architecture: String,
    /// Declared layer-stack count, when present in the descriptor.
    pub layer_stacks: Option<usize>,
    /// Complete artifact size in bytes.
    pub file_bytes: u64,
}

/// Read and validate only the bounded SFNN header.
pub fn read_sfnn_header(path: &Path) -> io::Result<SfnnHeader> {
    let file_bytes = std::fs::metadata(path)?.len();
    if file_bytes < HEADER_BYTES {
        return Err(invalid("file is shorter than the 12-byte SFNN header"));
    }
    if file_bytes > MAX_FILE_BYTES {
        return Err(invalid("file exceeds the 2 GiB safety limit"));
    }

    let mut file = File::open(path)?;
    let mut fixed = [0u8; HEADER_BYTES as usize];
    file.read_exact(&mut fixed)?;
    let version = u32::from_le_bytes(fixed[0..4].try_into().unwrap());
    let hash = u32::from_le_bytes(fixed[4..8].try_into().unwrap());
    let architecture_bytes = u32::from_le_bytes(fixed[8..12].try_into().unwrap());
    if architecture_bytes == 0 || architecture_bytes > MAX_ARCHITECTURE_BYTES {
        return Err(invalid("architecture length is outside the safety limit"));
    }
    if HEADER_BYTES + u64::from(architecture_bytes) > file_bytes {
        return Err(invalid("truncated architecture descriptor"));
    }

    let mut architecture_raw = vec![0u8; architecture_bytes as usize];
    file.read_exact(&mut architecture_raw)?;
    let architecture = String::from_utf8(architecture_raw)
        .map_err(|_| invalid("architecture descriptor is not valid UTF-8"))?;
    if !architecture.contains("ModelType=SFNN") {
        return Err(invalid("architecture is not declared as SFNN"));
    }
    if !architecture.contains("Features=") || !architecture.contains("Network=") {
        return Err(invalid("architecture lacks Features= or Network="));
    }

    let layer_stacks = architecture
        .split_once("LayerStack=")
        .and_then(|(_, suffix)| suffix.split('}').next())
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| invalid("LayerStack is not an integer"))
                .and_then(|count| {
                    if (1..=4096).contains(&count) {
                        Ok(count)
                    } else {
                        Err(invalid("LayerStack is outside the safety limit"))
                    }
                })
        })
        .transpose()?;

    Ok(SfnnHeader {
        version,
        hash,
        architecture,
        layer_stacks,
        file_bytes,
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("sekirei-{name}-{}.bin", std::process::id()))
    }

    fn write_fixture(path: &Path, architecture: &str) {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0x1234_abcd_u32.to_le_bytes());
        data.extend_from_slice(&(architecture.len() as u32).to_le_bytes());
        data.extend_from_slice(architecture.as_bytes());
        data.extend_from_slice(&[0u8; 4]);
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn reads_bounded_sfnn_header() {
        let path = fixture_path("sfnn-header");
        write_fixture(
            &path,
            "ModelType=SFNNWithoutPsqt;Features=HalfKP;Network=1536-15-32{LayerStack=9}",
        );
        let header = read_sfnn_header(&path).unwrap();
        assert_eq!(header.version, 1);
        assert_eq!(header.hash, 0x1234_abcd);
        assert_eq!(header.layer_stacks, Some(9));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_non_sfnn_architecture() {
        let path = fixture_path("non-sfnn");
        write_fixture(&path, "Features=HalfKP;Network=1536-15-32");
        let error = read_sfnn_header(&path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_truncated_descriptor() {
        let path = fixture_path("truncated");
        std::fs::write(&path, [1u8; 12]).unwrap();
        let error = read_sfnn_header(&path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        let _ = std::fs::remove_file(path);
    }
}
