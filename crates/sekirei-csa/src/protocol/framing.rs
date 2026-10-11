//! Bounded, timeout-tolerant CSA line framing.

use std::io::{self, BufRead, BufReader, Read};

pub(super) const MAX_CSA_LINE_BYTES: usize = 64 * 1024;

/// Retains partial input across one timeout and drains oversized lines before
/// returning an error, so a suffix can never be parsed as another command.
pub(super) struct CsaLineReader<R: Read> {
    inner: BufReader<R>,
    pending: Vec<u8>,
    discarding_oversized: bool,
}

impl<R: Read> CsaLineReader<R> {
    pub(super) fn new(inner: R) -> Self {
        Self {
            inner: BufReader::new(inner),
            pending: Vec::new(),
            discarding_oversized: false,
        }
    }

    pub(super) fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut consecutive_partial_timeouts = 0u8;
        loop {
            let available = match self.inner.fill_buf() {
                Ok(bytes) => bytes,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) && (!self.pending.is_empty() || self.discarding_oversized)
                        && consecutive_partial_timeouts == 0 =>
                {
                    consecutive_partial_timeouts += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };

            if available.is_empty() {
                if self.discarding_oversized {
                    self.discarding_oversized = false;
                    self.pending.clear();
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "CSA line exceeded 64 KiB before EOF",
                    ));
                }
                if self.pending.is_empty() {
                    return Ok(None);
                }
                self.pending.clear();
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed in the middle of a CSA line",
                ));
            }

            consecutive_partial_timeouts = 0;

            if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
                if !self.discarding_oversized {
                    if self.pending.len().saturating_add(newline) > MAX_CSA_LINE_BYTES {
                        self.pending.clear();
                        self.discarding_oversized = true;
                    } else {
                        self.pending.extend_from_slice(&available[..newline]);
                    }
                }
                self.inner.consume(newline + 1);

                if self.discarding_oversized {
                    self.discarding_oversized = false;
                    self.pending.clear();
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "CSA line exceeds the 64 KiB limit",
                    ));
                }

                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let bytes = std::mem::take(&mut self.pending);
                let line = String::from_utf8(bytes).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "CSA line is not valid UTF-8")
                })?;
                return Ok(Some(line));
            }

            let consumed = available.len();
            if !self.discarding_oversized {
                if self.pending.len().saturating_add(consumed) > MAX_CSA_LINE_BYTES {
                    self.pending.clear();
                    self.discarding_oversized = true;
                } else {
                    self.pending.extend_from_slice(available);
                }
            }
            self.inner.consume(consumed);
        }
    }
}
