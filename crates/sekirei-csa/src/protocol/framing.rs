//! Bounded, timeout-tolerant CSA line framing.

use std::io::{self, BufRead, BufReader, Read};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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
        self.read_line_until(None)
    }

    pub(super) fn pending_bytes(&self) -> usize {
        self.pending.len()
    }

    /// Keep the same partial-line buffer across soft socket timeouts, while
    /// enforcing one absolute deadline even if fragments keep arriving.
    pub(super) fn read_line_until(
        &mut self,
        deadline: Option<Instant>,
    ) -> io::Result<Option<String>> {
        let mut consecutive_partial_timeouts = 0u8;
        loop {
            if deadline.is_some_and(|limit| Instant::now() >= limit) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "CSA line receive deadline exceeded",
                ));
            }
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

impl CsaLineReader<TcpStream> {
    pub(super) fn set_read_timeout(&self, timeout: Duration) -> io::Result<()> {
        self.inner.get_ref().set_read_timeout(Some(timeout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    enum Step {
        Bytes(&'static [u8]),
        Error(io::ErrorKind),
    }

    struct ScriptedRead(VecDeque<Step>);

    impl Read for ScriptedRead {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            match self.0.pop_front() {
                Some(Step::Bytes(bytes)) => {
                    let len = bytes.len().min(output.len());
                    output[..len].copy_from_slice(&bytes[..len]);
                    if len < bytes.len() {
                        self.0.push_front(Step::Bytes(&bytes[len..]));
                    }
                    Ok(len)
                }
                Some(Step::Error(kind)) => Err(io::Error::from(kind)),
                None => Ok(0),
            }
        }
    }

    #[test]
    fn timeout_without_partial_input_is_returned_immediately() {
        let mut reader = CsaLineReader::new(ScriptedRead(VecDeque::from([Step::Error(
            io::ErrorKind::TimedOut,
        )])));
        assert_eq!(
            reader.read_line().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn only_one_partial_timeout_is_tolerated() {
        let mut reader = CsaLineReader::new(ScriptedRead(VecDeque::from([
            Step::Bytes(b"partial"),
            Step::Error(io::ErrorKind::WouldBlock),
            Step::Error(io::ErrorKind::TimedOut),
        ])));
        assert_eq!(
            reader.read_line().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn oversized_unterminated_line_is_rejected_at_eof() {
        let input = vec![b'x'; MAX_CSA_LINE_BYTES + 1];
        let mut reader = CsaLineReader::new(input.as_slice());
        assert_eq!(
            reader.read_line().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn split_crlf_line_is_joined_and_carriage_return_is_removed() {
        let mut reader = CsaLineReader::new(ScriptedRead(VecDeque::from([
            Step::Bytes(b"Game_"),
            Step::Bytes(b"ID:test\r\nnext\n"),
        ])));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("Game_ID:test"));
        assert_eq!(reader.read_line().unwrap().as_deref(), Some("next"));
        assert_eq!(reader.read_line().unwrap(), None);
    }
}
