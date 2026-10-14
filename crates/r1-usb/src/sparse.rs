//! Android's sparse image format (AOSP's libsparse), which fastboot sends and the bootloader writes (ADR-0067): a
//! header, then chunks of 4 KiB blocks, each raw data, a 4-byte fill, or "don't care" (left as it is on the device).
//! The `super` builder writes one, and reads it back to check exactly which bytes it would change.

use anyhow::{Context, Result, bail};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const MAGIC: u32 = 0xed26_ff3a;
pub const BLOCK: u64 = 4096;
const FILE_HEADER: u16 = 28;
const CHUNK_HEADER: u16 = 12;
const RAW: u16 = 0xcac1;
const FILL: u16 = 0xcac2;
const DONT_CARE: u16 = 0xcac3;
const CRC32: u16 = 0xcac4;
/// The most a raw chunk carries: fastboot resplits for the device anyway; this keeps each write small.
const RAW_MAX: u64 = 16 << 20;

/// Writes a sparse image of a device [blocks] long, front to back.
pub struct Writer<W: Write + Seek> {
    out: W,
    blocks: u64,
    at: u64,
    chunks: u32,
}

impl<W: Write + Seek> Writer<W> {
    pub fn new(mut out: W, blocks: u64) -> Result<Self> {
        if blocks > u32::MAX as u64 {
            bail!("a device too big for one sparse image");
        }
        out.write_all(&[0u8; FILE_HEADER as usize])?;
        Ok(Self { out, blocks, at: 0, chunks: 0 })
    }

    /// Blocks written so far: where the next chunk starts.
    pub fn at(&self) -> u64 {
        self.at
    }

    fn chunk(&mut self, kind: u16, blocks: u64, body: &[u8]) -> Result<()> {
        if blocks == 0 {
            return Ok(());
        }
        if self.at + blocks > self.blocks {
            bail!("a chunk past the device's end");
        }
        self.out.write_all(&kind.to_le_bytes())?;
        self.out.write_all(&0u16.to_le_bytes())?;
        self.out.write_all(&(blocks as u32).to_le_bytes())?;
        self.out.write_all(&(CHUNK_HEADER as u32 + body.len() as u32).to_le_bytes())?;
        self.out.write_all(body)?;
        self.at += blocks;
        self.chunks += 1;
        Ok(())
    }

    /// Leaves the device as it is up to block [to].
    pub fn skip_to(&mut self, to: u64) -> Result<()> {
        if to < self.at {
            bail!("the sparse image would go backwards (block {to} after {})", self.at);
        }
        self.chunk(DONT_CARE, to - self.at, &[])
    }

    /// [data] (a whole number of blocks) at the current block: zero blocks as fills, the rest raw.
    pub fn data(&mut self, data: &[u8]) -> Result<()> {
        if !(data.len() as u64).is_multiple_of(BLOCK) {
            bail!("data that isn't a whole number of 4 KiB blocks");
        }
        let blocks: Vec<&[u8]> = data.chunks(BLOCK as usize).collect();
        let mut i = 0;
        while i < blocks.len() {
            let zero = blocks[i].iter().all(|&b| b == 0);
            let mut j = i + 1;
            while j < blocks.len() && blocks[j].iter().all(|&b| b == 0) == zero && (zero || ((j - i) as u64) < RAW_MAX / BLOCK) {
                j += 1;
            }
            if zero {
                self.chunk(FILL, (j - i) as u64, &0u32.to_le_bytes())?;
            } else {
                self.chunk(RAW, (j - i) as u64, &data[i * BLOCK as usize..j * BLOCK as usize])?;
            }
            i = j;
        }
        Ok(())
    }

    /// Leaves the rest of the device as it is, and writes the header.
    pub fn finish(mut self) -> Result<W> {
        let end = self.blocks;
        self.skip_to(end)?;
        self.out.seek(SeekFrom::Start(0))?;
        let mut h = Vec::with_capacity(FILE_HEADER as usize);
        h.extend_from_slice(&MAGIC.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&FILE_HEADER.to_le_bytes());
        h.extend_from_slice(&CHUNK_HEADER.to_le_bytes());
        h.extend_from_slice(&(BLOCK as u32).to_le_bytes());
        h.extend_from_slice(&(self.blocks as u32).to_le_bytes());
        h.extend_from_slice(&self.chunks.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        self.out.write_all(&h)?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// What a sparse image writes, as read back from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    /// Raw data at a byte offset on the device: where it is in the image file, and how long.
    Raw { at: u64, len: u64, file_at: u64 },
    /// A 4-byte pattern repeated.
    Fill { at: u64, len: u64, value: u32 },
}

impl Span {
    pub fn range(&self) -> (u64, u64) {
        match *self {
            Span::Raw { at, len, .. } | Span::Fill { at, len, .. } => (at, at + len),
        }
    }
}

/// An image's device size in bytes, and every span it writes (never "don't care" ones), in order.
pub fn spans(path: &Path) -> Result<(u64, Vec<Span>)> {
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut h = [0u8; FILE_HEADER as usize];
    f.read_exact(&mut h).context("not a sparse image")?;
    let u16_at = |o: usize| u16::from_le_bytes([h[o], h[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes(h[o..o + 4].try_into().unwrap());
    if u32_at(0) != MAGIC || u16_at(4) != 1 || u16_at(8) != FILE_HEADER || u16_at(10) != CHUNK_HEADER || u32_at(12) as u64 != BLOCK {
        bail!("not a sparse image this writes");
    }
    let (total, chunks) = (u32_at(16) as u64, u32_at(20));
    let mut at_block = 0u64;
    let mut file_at = FILE_HEADER as u64;
    let mut out = Vec::new();
    for _ in 0..chunks {
        let mut c = [0u8; CHUNK_HEADER as usize];
        f.read_exact(&mut c).context("a chunk cut short")?;
        let kind = u16::from_le_bytes([c[0], c[1]]);
        let blocks = u32::from_le_bytes(c[4..8].try_into().unwrap()) as u64;
        let size = u32::from_le_bytes(c[8..12].try_into().unwrap()) as u64;
        let body = size.checked_sub(CHUNK_HEADER as u64).context("a chunk smaller than its header")?;
        file_at += CHUNK_HEADER as u64;
        let (at, len) = (at_block * BLOCK, blocks * BLOCK);
        match kind {
            RAW => {
                if body != len {
                    bail!("a raw chunk whose data isn't its blocks");
                }
                out.push(Span::Raw { at, len, file_at });
            }
            FILL => {
                let mut v = [0u8; 4];
                f.read_exact(&mut v)?;
                out.push(Span::Fill { at, len, value: u32::from_le_bytes(v) });
                f.seek_relative(body as i64 - 4)?;
                file_at += body;
                at_block += blocks;
                continue;
            }
            DONT_CARE | CRC32 => {}
            k => bail!("a chunk of kind {k:#x}"),
        }
        f.seek_relative(body as i64)?;
        file_at += body;
        if kind != CRC32 {
            at_block += blocks;
        }
    }
    if at_block != total {
        bail!("the chunks cover {at_block} blocks, the header says {total}");
    }
    Ok((total * BLOCK, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn written_and_read_back() {
        let mut w = Writer::new(Cursor::new(Vec::new()), 100).unwrap();
        w.skip_to(3).unwrap();
        let mut data = vec![7u8; 4 * BLOCK as usize];
        data[BLOCK as usize..2 * BLOCK as usize].fill(0);
        w.data(&data).unwrap();
        w.skip_to(50).unwrap();
        w.data(&[0u8; BLOCK as usize]).unwrap();
        assert!(w.skip_to(10).is_err(), "never backwards");
        let bytes = w.finish().unwrap().into_inner();
        let path = std::env::temp_dir().join(format!("r1-usb-sparse-{}", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let (size, spans) = spans(&path).unwrap();
        assert_eq!(size, 100 * BLOCK);
        let ranges: Vec<_> = spans.iter().map(|s| (s.range(), matches!(s, Span::Raw { .. }))).collect();
        assert_eq!(
            ranges,
            vec![
                ((3 * BLOCK, 4 * BLOCK), true),
                ((4 * BLOCK, 5 * BLOCK), false),
                ((5 * BLOCK, 7 * BLOCK), true),
                ((50 * BLOCK, 51 * BLOCK), false),
            ]
        );
        if let Span::Raw { file_at, len, .. } = spans[0] {
            assert_eq!(&bytes[file_at as usize..(file_at + len) as usize], &data[..BLOCK as usize]);
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn data_must_be_whole_blocks() {
        let mut w = Writer::new(Cursor::new(Vec::new()), 10).unwrap();
        assert!(w.data(&[1u8; 100]).is_err());
        w.skip_to(9).unwrap();
        assert!(w.data(&[1u8; 2 * BLOCK as usize]).is_err(), "past the end");
    }
}
