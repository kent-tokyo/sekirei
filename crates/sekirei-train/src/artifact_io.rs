//! Durable same-directory publication for trainer artifacts.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temporary_sibling(path: &Path) -> io::Result<std::path::PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact path must name a file",
        )
    })?;
    Ok(path.with_file_name(format!(
        ".{}.tmp-{}-{}",
        file_name.to_string_lossy(),
        std::process::id(),
        WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Write an artifact without exposing a truncated destination when the
/// trainer is interrupted during serialization or flushing.
pub(crate) fn write_atomic_stream<F>(path: &Path, write: F) -> io::Result<()>
where
    F: FnOnce(&mut BufWriter<File>) -> io::Result<()>,
{
    let temp_path = temporary_sibling(path)?;
    let write_result = (|| -> io::Result<()> {
        let file = File::create(&temp_path)?;
        let mut output = BufWriter::new(file);
        write(&mut output)?;
        output.flush()?;
        let file = output.into_inner().map_err(|error| error.into_error())?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    write_atomic_stream(path, |output| output.write_all(contents))
}

/// Copy a completed checkpoint into its stable selection path without ever
/// exposing a partially copied destination.
pub(crate) fn copy_atomic(source: &Path, destination: &Path) -> io::Result<u64> {
    let temp_path = temporary_sibling(destination)?;
    let result = (|| -> io::Result<u64> {
        let copied = fs::copy(source, &temp_path)?;
        File::open(&temp_path)?.sync_all()?;
        fs::rename(&temp_path, destination)?;
        Ok(copied)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}
