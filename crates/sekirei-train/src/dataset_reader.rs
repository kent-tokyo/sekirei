//! Optional Pure Rust compression adapters for positions JSONL.
//!
//! Magic bytes take precedence over extensions. Recognized compressed suffixes
//! never fall back to plain text. Decoders consume every member/frame and reject
//! incomplete streams, invalid checksums, and trailing non-frame bytes. The
//! caller can hash the original source below this adapter in the same pass.

use std::io::{self, BufRead, BufReader, Chain, Cursor, Read};
use std::path::Path;

const ZSTD_MAGIC: u32 = 0xfd2f_b528;
const ZSTD_SKIP_MIN: u32 = 0x184d_2a50;
const ZSTD_SKIP_MAX: u32 = 0x184d_2a5f;

// Four bytes are replayed after detection, without rereading the original input.
type BufferedInput<R> = BufReader<Chain<Cursor<Vec<u8>>, R>>;

/// Statically dispatched decoded input; codecs are opt-in Cargo features.
pub struct DatasetReader<R: Read> {
    inner: ReaderInner<R>,
    compression: &'static str,
    failed: bool,
}

enum ReaderInner<R: Read> {
    Plain(BufferedInput<R>),
    #[cfg(feature = "positions-gzip")]
    Gzip(Box<BufReader<flate2::bufread::MultiGzDecoder<BufferedInput<R>>>>),
    #[cfg(feature = "positions-zstd")]
    Zstd(Box<BufReader<MultiZstdDecoder<BufferedInput<R>>>>),
}

impl<R: Read> DatasetReader<R> {
    pub fn new(mut source: R, path: &Path) -> io::Result<Self> {
        let mut prefix = [0u8; 4];
        let mut length = 0;
        while length < prefix.len() {
            match source.read(&mut prefix[length..]) {
                Ok(0) => break,
                Ok(count) => length += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        let compression = detect_compression(&prefix[..length], path);
        let input = BufReader::new(Cursor::new(prefix[..length].to_vec()).chain(source));
        let inner = match compression {
            "gzip" => {
                #[cfg(feature = "positions-gzip")]
                {
                    ReaderInner::Gzip(Box::new(BufReader::new(
                        flate2::bufread::MultiGzDecoder::new(input),
                    )))
                }
                #[cfg(not(feature = "positions-gzip"))]
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "gzip positions require the positions-gzip Cargo feature",
                    ));
                }
            }
            "zstd" => {
                #[cfg(feature = "positions-zstd")]
                {
                    ReaderInner::Zstd(Box::new(BufReader::new(MultiZstdDecoder::new(input))))
                }
                #[cfg(not(feature = "positions-zstd"))]
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "zstd positions require the positions-zstd Cargo feature",
                    ));
                }
            }
            _ => ReaderInner::Plain(input),
        };
        Ok(Self {
            inner,
            compression,
            failed: false,
        })
    }

    pub fn compression(&self) -> &'static str {
        self.compression
    }
}

fn detect_compression(prefix: &[u8], path: &Path) -> &'static str {
    if prefix.starts_with(&[0x1f, 0x8b]) {
        return "gzip";
    }
    if let Ok(magic) = <[u8; 4]>::try_from(prefix) {
        let magic = u32::from_le_bytes(magic);
        if magic == ZSTD_MAGIC || (ZSTD_SKIP_MIN..=ZSTD_SKIP_MAX).contains(&magic) {
            return "zstd";
        }
    }
    match path
        .extension()
        .and_then(|suffix| suffix.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("gz" | "gzip") => "gzip",
        Some("zst" | "zstd") => "zstd",
        _ => "plain",
    }
}

fn failed_decoder_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "positions decoder previously failed",
    )
}

impl<R: Read> Read for DatasetReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.failed {
            return Err(failed_decoder_error());
        }
        let result = match &mut self.inner {
            ReaderInner::Plain(reader) => reader.read(output),
            #[cfg(feature = "positions-gzip")]
            ReaderInner::Gzip(reader) => reader.read(output),
            #[cfg(feature = "positions-zstd")]
            ReaderInner::Zstd(reader) => reader.read(output),
        };
        self.failed = result
            .as_ref()
            .err()
            .is_some_and(|error| error.kind() != io::ErrorKind::Interrupted);
        result
    }
}

impl<R: Read> BufRead for DatasetReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.failed {
            return Err(failed_decoder_error());
        }
        let result = match &mut self.inner {
            ReaderInner::Plain(reader) => reader.fill_buf(),
            #[cfg(feature = "positions-gzip")]
            ReaderInner::Gzip(reader) => reader.fill_buf(),
            #[cfg(feature = "positions-zstd")]
            ReaderInner::Zstd(reader) => reader.fill_buf(),
        };
        self.failed = result
            .as_ref()
            .err()
            .is_some_and(|error| error.kind() != io::ErrorKind::Interrupted);
        result
    }

    fn consume(&mut self, amount: usize) {
        match &mut self.inner {
            ReaderInner::Plain(reader) => reader.consume(amount),
            #[cfg(feature = "positions-gzip")]
            ReaderInner::Gzip(reader) => reader.consume(amount),
            #[cfg(feature = "positions-zstd")]
            ReaderInner::Zstd(reader) => reader.consume(amount),
        }
    }
}

/// Explicit 8 MiB history-window limit; larger-window archives are rejected.
/// Decoder scratch, the current block, and the scanner's longest JSONL row add
/// to this bound. This does not claim an 8 MiB total resident-memory limit.
#[cfg(feature = "positions-zstd")]
const ZSTD_MAX_WINDOW_BYTES: u64 = 8 * 1024 * 1024;

#[cfg(feature = "positions-zstd")]
struct MultiZstdDecoder<R: BufRead> {
    source: R,
    decoder: ruzstd::decoding::FrameDecoder,
    active: bool,
    seen_frame: bool,
    expected_size: Option<u64>,
    output_bytes: u64,
}

#[cfg(feature = "positions-zstd")]
impl<R: BufRead> MultiZstdDecoder<R> {
    fn new(source: R) -> Self {
        let mut decoder = ruzstd::decoding::FrameDecoder::new();
        decoder.set_max_window_size(ZSTD_MAX_WINDOW_BYTES);
        Self {
            source,
            decoder,
            active: false,
            seen_frame: false,
            expected_size: None,
            output_bytes: 0,
        }
    }

    fn finish_frame(&mut self) -> io::Result<()> {
        if self
            .expected_size
            .is_some_and(|size| size != self.output_bytes)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "zstd frame content size mismatch: declared {:?}, decoded {} bytes",
                    self.expected_size, self.output_bytes
                ),
            ));
        }
        // ruzstd's `hash` feature computes the checksum, but does not compare
        // it. Its calculated value is final only after all output is drained.
        if let Some(expected) = self.decoder.get_checksum_from_data()
            && self.decoder.get_calculated_checksum() != Some(expected)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zstd frame checksum mismatch",
            ));
        }
        self.active = false;
        Ok(())
    }

    fn begin_frame(&mut self) -> io::Result<bool> {
        use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
        loop {
            if self.source.fill_buf()?.is_empty() {
                if self.seen_frame {
                    return Ok(false);
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "empty zstd input has no complete frame",
                ));
            }
            // Observe only the magic and descriptor while ruzstd parses the
            // header. This distinguishes an absent content size from size 0.
            let mut header = HeaderProbe {
                source: &mut self.source,
                prefix: [0; 5],
                count: 0,
            };
            let initialized = self.decoder.init(&mut header);
            let descriptor = header.prefix[4];
            match initialized {
                Ok(()) => {
                    if descriptor & 0x08 != 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "zstd frame header reserved bit is set",
                        ));
                    }
                    self.expected_size =
                        (descriptor & 0xe0 != 0).then(|| self.decoder.content_size());
                    self.output_bytes = 0;
                    self.active = true;
                    self.seen_frame = true;
                    return Ok(true);
                }
                Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                    length,
                    ..
                })) => {
                    // The header already consumed magic + length. Read the
                    // payload rather than seek: this both detects truncation
                    // and keeps the caller's physical-input digest complete.
                    let copied = io::copy(
                        &mut self.source.by_ref().take(u64::from(length)),
                        &mut io::sink(),
                    )?;
                    if copied != u64::from(length) {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated zstd skippable frame payload",
                        ));
                    }
                    self.seen_frame = true;
                }
                Err(error) => return Err(zstd_error(error)),
            }
        }
    }
}

#[cfg(feature = "positions-zstd")]
fn zstd_error(error: ruzstd::decoding::errors::FrameDecoderError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid zstd frame (window limit {ZSTD_MAX_WINDOW_BYTES} bytes): {error}"),
    )
}

#[cfg(feature = "positions-zstd")]
impl<R: BufRead> Read for MultiZstdDecoder<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        use ruzstd::decoding::BlockDecodingStrategy;
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if !self.active && !self.begin_frame()? {
                return Ok(0);
            }
            if self.decoder.can_collect() > 0 || self.decoder.is_finished() {
                let count = self.decoder.read(output)?;
                self.output_bytes =
                    self.output_bytes.checked_add(count as u64).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "zstd decoded byte count overflow",
                        )
                    })?;
                if self.decoder.is_finished() && self.decoder.can_collect() == 0 {
                    self.finish_frame()?;
                }
                if count != 0 {
                    return Ok(count);
                }
                // Empty/skippable frames must not signal EOF before later frames.
                continue;
            }
            self.decoder
                .decode_blocks(&mut self.source, BlockDecodingStrategy::UptoBlocks(1))
                .map_err(zstd_error)?;
        }
    }
}

#[cfg(feature = "positions-zstd")]
struct HeaderProbe<R> {
    source: R,
    prefix: [u8; 5],
    count: usize,
}

#[cfg(feature = "positions-zstd")]
impl<R: Read> Read for HeaderProbe<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.source.read(output)?;
        let copied = count.min(self.prefix.len() - self.count);
        self.prefix[self.count..self.count + copied].copy_from_slice(&output[..copied]);
        self.count += copied;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ShortRead<R>(R);
    impl<R: Read> Read for ShortRead<R> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let length = output.len().min(1);
            self.0.read(&mut output[..length])
        }
    }

    fn decode(data: &[u8], path: &str) -> io::Result<Vec<u8>> {
        let mut reader = DatasetReader::new(ShortRead(data), Path::new(path))?;
        let mut decoded = Vec::new();
        reader.read_to_end(&mut decoded)?;
        Ok(decoded)
    }

    #[test]
    fn plain_short_reads_preserve_bytes_and_bufread_boundaries() {
        let input = b"\n{\"row\":1}\r\n{\"row\":2}";
        assert_eq!(decode(input, "data.jsonl").unwrap(), input);
        assert_eq!(decode(b"", "data.jsonl").unwrap(), b"");
        let mut source = Cursor::new(input);
        {
            let mut reader = DatasetReader::new(&mut source, Path::new("data.jsonl")).unwrap();
            assert_eq!(reader.compression(), "plain");
            assert_eq!(reader.read(&mut []).unwrap(), 0);
            let mut row = String::new();
            assert_eq!(reader.read_line(&mut row).unwrap(), 1);
            assert_eq!(row, "\n");
            row.clear();
            reader.read_line(&mut row).unwrap();
            assert_eq!(row, "{\"row\":1}\r\n");
            io::copy(&mut reader, &mut io::sink()).unwrap();
        }
        assert_eq!(source.position(), input.len() as u64);
    }

    #[test]
    fn interrupted_underlying_read_can_be_retried() {
        struct InterruptOnce {
            source: Cursor<Vec<u8>>,
            interrupted: bool,
        }
        impl Read for InterruptOnce {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.source.position() >= 4 && !self.interrupted {
                    self.interrupted = true;
                    return Err(io::ErrorKind::Interrupted.into());
                }
                self.source.read(output)
            }
        }
        let input = b"first row\nsecond row\n";
        let source = InterruptOnce {
            source: Cursor::new(input.to_vec()),
            interrupted: false,
        };
        let mut reader = DatasetReader::new(source, Path::new("data.jsonl")).unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, input);
    }

    #[test]
    fn magic_has_precedence_over_compressed_suffixes() {
        assert_eq!(
            detect_compression(&[0x1f, 0x8b], Path::new("data.zst")),
            "gzip"
        );
        assert_eq!(
            detect_compression(&ZSTD_MAGIC.to_le_bytes(), Path::new("data.gz")),
            "zstd"
        );
        for magic in ZSTD_SKIP_MIN..=ZSTD_SKIP_MAX {
            assert_eq!(
                detect_compression(&magic.to_le_bytes(), Path::new("data")),
                "zstd"
            );
        }
        for suffix in ["gz", "GZIP"] {
            assert_eq!(
                detect_compression(b"{}\n", Path::new(&format!("data.{suffix}"))),
                "gzip"
            );
        }
        for suffix in ["zst", "ZSTD"] {
            assert_eq!(
                detect_compression(b"{}\n", Path::new(&format!("data.{suffix}"))),
                "zstd"
            );
        }
    }

    #[cfg(not(feature = "positions-gzip"))]
    #[test]
    fn disabled_gzip_rejects_magic_and_suffix() {
        for (bytes, path) in [(&[0x1f, 0x8b][..], "data"), (&b"{}\n"[..], "data.gz")] {
            let error = decode(bytes, path).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("positions-gzip"));
        }
    }

    #[cfg(not(feature = "positions-zstd"))]
    #[test]
    fn disabled_zstd_rejects_magic_skippable_and_suffix() {
        for (bytes, path) in [
            (&ZSTD_MAGIC.to_le_bytes()[..], "data"),
            (&ZSTD_SKIP_MIN.to_le_bytes()[..], "data"),
            (&b"{}\n"[..], "data.zst"),
        ] {
            let error = decode(bytes, path).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("positions-zstd"));
        }
    }

    #[cfg(feature = "positions-gzip")]
    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    #[cfg(feature = "positions-gzip")]
    #[test]
    fn gzip_reads_all_members_and_checks_crc_truncation_and_trailing_bytes() {
        let frame = gzip(b"first\n");
        let mut combined = frame.clone();
        combined.extend(gzip(b""));
        combined.extend(gzip(b"second\n"));
        assert_eq!(decode(&combined, "data.zst").unwrap(), b"first\nsecond\n");
        for length in 0..frame.len() {
            assert!(
                decode(&frame[..length], "data.gz").is_err(),
                "accepted prefix {length}"
            );
        }
        for offset in [frame.len() - 8, frame.len() - 4] {
            let mut corrupt = frame.clone();
            corrupt[offset] ^= 1;
            assert!(decode(&corrupt, "data").is_err());
        }
        combined.extend_from_slice(b"trailing garbage");
        assert!(decode(&combined, "data.gz").is_err());
        assert!(decode(b"{}\n", "data.gzip").is_err());
    }

    #[cfg(feature = "positions-zstd")]
    fn zstd(data: &[u8]) -> Vec<u8> {
        ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
    }

    #[cfg(feature = "positions-zstd")]
    fn raw_zstd(data: &[u8], declared_size: u8) -> Vec<u8> {
        let mut frame = ZSTD_MAGIC.to_le_bytes().to_vec();
        frame.extend_from_slice(&[0x20, declared_size]);
        let block_header = ((data.len() as u32) << 3) | 1;
        frame.extend_from_slice(&block_header.to_le_bytes()[..3]);
        frame.extend_from_slice(data);
        frame
    }

    #[cfg(feature = "positions-zstd")]
    fn skippable(data: &[u8]) -> Vec<u8> {
        let mut frame = ZSTD_SKIP_MIN.to_le_bytes().to_vec();
        frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
        frame.extend_from_slice(data);
        frame
    }

    #[cfg(feature = "positions-zstd")]
    #[test]
    fn zstd_reads_compressed_concatenated_empty_and_skippable_frames() {
        let first = b"first JSONL row repeated\n".repeat(100);
        let compressed = zstd(&first);
        assert!(compressed.len() < first.len());
        assert_ne!(compressed[4] & 4, 0, "fixture must carry a checksum");
        let mut combined = skippable(b"metadata before");
        combined.extend(compressed);
        combined.extend(raw_zstd(b"", 0));
        combined.extend(skippable(b"middle"));
        combined.extend(zstd(b"last\n"));
        combined.extend(skippable(b"after"));
        let expected = [first.as_slice(), b"last\n"].concat();
        assert_eq!(decode(&combined, "data.gz").unwrap(), expected);
        assert_eq!(
            decode(&skippable(b"only metadata"), "data.zstd").unwrap(),
            b""
        );
    }

    #[cfg(feature = "positions-zstd")]
    #[test]
    fn zstd_rejects_every_partial_frame_bad_checksum_and_trailing_garbage() {
        let frame = zstd(b"first\n");
        for length in 0..frame.len() {
            assert!(
                decode(&frame[..length], "data.zst").is_err(),
                "accepted prefix {length}"
            );
        }
        let mut corrupt = frame.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(
            decode(&corrupt, "data")
                .unwrap_err()
                .to_string()
                .contains("checksum mismatch")
        );
        for trailing in [&b"x"[..], &b"xyz"[..], &b"trailing garbage"[..]] {
            let combined = [frame.as_slice(), trailing].concat();
            assert!(decode(&combined, "data.zstd").is_err());
        }
        assert!(decode(b"{}\n", "data.zstd").is_err());
        let skip = skippable(b"metadata");
        for length in 0..skip.len() {
            assert!(decode(&skip[..length], "data.zst").is_err());
        }
    }

    #[cfg(feature = "positions-zstd")]
    #[test]
    fn zstd_validates_declared_size_reserved_bit_dictionary_and_window_limit() {
        assert_eq!(decode(&raw_zstd(b"abc", 3), "data.zst").unwrap(), b"abc");
        for size in [0u32, 2, 4] {
            // A separate 1 KiB window allows the raw block to decode even
            // when its declared content size is 0 or smaller than the block.
            // This exercises the adapter's final size check specifically.
            let mut declared = ZSTD_MAGIC.to_le_bytes().to_vec();
            declared.extend_from_slice(&[0x80, 0x00]);
            declared.extend_from_slice(&size.to_le_bytes());
            declared.extend_from_slice(&[0x19, 0x00, 0x00, b'a', b'b', b'c']);
            assert!(
                decode(&declared, "data.zst")
                    .unwrap_err()
                    .to_string()
                    .contains("content size mismatch")
            );
            // Single-segment frames also use content size as window size;
            // ruzstd may correctly reject their oversized block earlier.
            assert!(decode(&raw_zstd(b"abc", size as u8), "data.zst").is_err());
        }
        let mut reserved = raw_zstd(b"abc", 3);
        reserved[4] |= 8;
        assert!(
            decode(&reserved, "data.zst")
                .unwrap_err()
                .to_string()
                .contains("reserved bit")
        );
        let mut large_window = ZSTD_MAGIC.to_le_bytes().to_vec();
        large_window.extend_from_slice(&[0x00, 0x70]); // 16 MiB window.
        let error = decode(&large_window, "data.zst").unwrap_err().to_string();
        assert!(error.contains("8388608"));
        assert!(error.contains("window"));
        let mut dictionary = ZSTD_MAGIC.to_le_bytes().to_vec();
        dictionary.extend_from_slice(&[0x21, 1, 0]); // Dict 1, single segment, size 0.
        assert!(decode(&dictionary, "data.zst").is_err());
        let mut unknown_size = ZSTD_MAGIC.to_le_bytes().to_vec();
        unknown_size.extend_from_slice(&[0x00, 0x00, 0x19, 0x00, 0x00]);
        unknown_size.extend_from_slice(b"abc");
        assert_eq!(decode(&unknown_size, "data.zst").unwrap(), b"abc");
    }

    #[cfg(feature = "positions-zstd")]
    #[test]
    fn zstd_error_remains_an_error_when_caller_retries() {
        let mut corrupt = zstd(b"first\n");
        *corrupt.last_mut().unwrap() ^= 1;
        let mut reader = DatasetReader::new(corrupt.as_slice(), Path::new("data.zst")).unwrap();
        assert!(reader.read_to_end(&mut Vec::new()).is_err());
        assert!(reader.read(&mut [0; 1]).is_err());
        assert!(reader.fill_buf().is_err());
    }
}
