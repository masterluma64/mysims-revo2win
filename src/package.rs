//! Reading resources out of the Wii games' `.package` files.
//!
//! MySims and MySims Kingdom use DBPF 2.0, little-endian:
//!
//! ```text
//! header (96 bytes)
//!   0x00 "DBPF", u32 major (2), u32 minor
//!   0x24 u32 entry count
//!   0x2C u32 index size
//!   0x40 u32 index offset
//! index
//!   u32 flags        bit 0: type constant, bit 1: group constant, bit 2: instance high constant
//!   u32 constants    one per set bit, in that order
//!   count × {
//!     [u32 type] [u32 group] [u32 instance high]   the fields that are not constant
//!     u32 instance low, u32 offset
//!     u32 size         bit 31 set
//!     u32 memory size  the size once decompressed
//!     [u16 compressed (0xFFFF), u16 committed]     not in every package; told by the index size
//!   }
//! ```
//!
//! MySims Agents uses its own big-endian variant:
//!
//! ```text
//! header (96 bytes)
//!   0x00 "FPBD", u32 major, u32 minor
//!   0x24 u32 entry count
//!   0x44 u32 index offset
//! index
//!   u64 type count
//!   type count × { u32 type, u32 group, u32 entries, u32 0 }
//!   entries, grouped by type in that order: {
//!     u64 instance, u32 offset, u32 size
//!     [u32 memory size]   only when the type's group is not 0
//!   }
//! ```
//!
//! An entry whose size and memory size differ is RefPack-compressed.

use crate::binary::{Endian, FormatError, Reader, Result};
use crate::game::ResourceKey;

const HEADER_SIZE: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub key: ResourceKey,
    pub offset: u32,
    /// Bytes in the package.
    pub size: u32,
    /// Bytes once decompressed.
    pub memory_size: u32,
}

impl Entry {
    pub fn is_compressed(&self) -> bool {
        self.size != self.memory_size
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// `DBPF`, MySims and Kingdom.
    Dbpf,
    /// `FPBD`, Agents.
    Agents,
}

pub struct Package<'a> {
    data: &'a [u8],
    pub variant: Variant,
    pub entries: Vec<Entry>,
}

/// Whether `data` starts like a package.
pub fn is_package(data: &[u8]) -> bool {
    data.starts_with(b"DBPF") || data.starts_with(b"FPBD")
}

impl<'a> Package<'a> {
    pub fn read(data: &'a [u8]) -> Result<Self> {
        let (variant, endian) = match data.get(..4) {
            Some(b"DBPF") => (Variant::Dbpf, Endian::Little),
            Some(b"FPBD") => (Variant::Agents, Endian::Big),
            _ => return Err(FormatError { offset: 0, message: "not a package (no DBPF or FPBD)".into() }),
        };
        if data.len() < HEADER_SIZE {
            return Err(FormatError {
                offset: data.len(),
                message: format!("truncated: {} bytes, the header alone is {HEADER_SIZE}", data.len()),
            });
        }
        let mut r = Reader::new(data, endian);
        r.seek(0x24)?;
        let count = r.u32()? as usize;
        let entries = match variant {
            Variant::Dbpf => {
                r.seek(0x2C)?;
                let index_size = r.u32()? as usize;
                r.seek(0x40)?;
                let index_offset = r.u32()? as usize;
                r.seek(index_offset)?;
                read_dbpf_index(&mut r, count, index_size)?
            }
            Variant::Agents => {
                r.seek(0x44)?;
                let index_offset = r.u32()? as usize;
                r.seek(index_offset)?;
                read_agents_index(&mut r, count)?
            }
        };
        if let Some(e) = entries.iter().find(|e| u64::from(e.offset) + u64::from(e.size) > data.len() as u64)
        {
            return Err(FormatError {
                offset: data.len(),
                message: format!("truncated: {} runs past the end of the file", e.key),
            });
        }
        Ok(Self { data, variant, entries })
    }

    /// The bytes of `entry`, decompressed if needed.
    pub fn extract(&self, entry: &Entry) -> Result<Vec<u8>> {
        let start = entry.offset as usize;
        let stored = start
            .checked_add(entry.size as usize)
            .and_then(|end| self.data.get(start..end))
            .ok_or_else(|| FormatError {
                offset: start,
                message: format!("{} runs past the end of the package", entry.key),
            })?;
        if !entry.is_compressed() {
            return Ok(stored.to_vec());
        }
        let out = refpack::decompress(stored).map_err(|e| FormatError {
            offset: start + e.offset,
            message: format!("{}: {}", entry.key, e.message),
        })?;
        if out.len() != entry.memory_size as usize {
            return Err(FormatError {
                offset: start,
                message: format!(
                    "{}: decompressed to {} bytes, the index says {}",
                    entry.key,
                    out.len(),
                    entry.memory_size
                ),
            });
        }
        Ok(out)
    }
}

fn read_dbpf_index(r: &mut Reader<'_>, count: usize, index_size: usize) -> Result<Vec<Entry>> {
    let flags = r.u32()?;
    if flags & !7 != 0 {
        return Err(r.error(format_args!("unknown index flags 0x{flags:x}")));
    }
    let constant = |bit: u32, r: &mut Reader<'_>| -> Result<Option<u32>> {
        if flags & bit != 0 { r.u32().map(Some) } else { Ok(None) }
    };
    let type_id = constant(1, r)?;
    let group = constant(2, r)?;
    let high = constant(4, r)?;
    let fields = 3 - flags.count_ones() as usize;
    let header_size = 4 + 4 * flags.count_ones() as usize;
    // 16 bytes always, 4 per field not held constant, and 4 more for the
    // compression flag and committed word when the index has room for them.
    let short = 16 + 4 * fields;
    let with_flags = count > 0 && index_size.saturating_sub(header_size) / count == short + 4;
    let entry_size = if with_flags { short + 4 } else { short };
    if count.saturating_mul(entry_size) > r.remaining() {
        return Err(r.error(format_args!("{count} entries do not fit in the file")));
    }
    let field = |fixed: Option<u32>, r: &mut Reader<'_>| fixed.map_or_else(|| r.u32(), Ok);
    (0..count)
        .map(|_| {
            let type_id = field(type_id, r)?;
            let group = field(group, r)?;
            let high = field(high, r)?;
            let low = r.u32()?;
            let offset = r.u32()?;
            let size = r.u32()? & 0x7FFF_FFFF;
            let memory_size = r.u32()?;
            if with_flags {
                r.u16()?; // compressed: 0xFFFF
                r.u16()?; // committed
            }
            let instance = (u64::from(high) << 32) | u64::from(low);
            Ok(Entry { key: ResourceKey { instance, type_id, group }, offset, size, memory_size })
        })
        .collect()
}

fn read_agents_index(r: &mut Reader<'_>, count: usize) -> Result<Vec<Entry>> {
    let type_count = r.u64()?;
    if type_count.saturating_mul(16) > r.remaining() as u64 {
        return Err(r.error(format_args!("{type_count} types do not fit in the file")));
    }
    let mut types = Vec::with_capacity(type_count as usize);
    for _ in 0..type_count {
        let type_id = r.u32()?;
        let group = r.u32()?;
        let entries = r.u32()? as usize;
        r.u32()?;
        types.push((type_id, group, entries));
    }
    let listed: usize = types.iter().map(|t| t.2).sum();
    if listed != count {
        return Err(r.error(format_args!("the types list {listed} entries, the header {count}")));
    }
    if count.saturating_mul(16) > r.remaining() {
        return Err(r.error(format_args!("{count} entries do not fit in the file")));
    }
    let mut entries = Vec::with_capacity(count);
    for (type_id, group, n) in types {
        for _ in 0..n {
            let instance = r.u64()?;
            let offset = r.u32()?;
            let size = r.u32()?;
            let memory_size = if group != 0 { r.u32()? } else { size };
            entries.push(Entry { key: ResourceKey { instance, type_id, group }, offset, size, memory_size });
        }
    }
    Ok(entries)
}

/// RefPack (also called QFS) decompression.
///
/// ```text
/// u8 flags     0x80: sizes are 4 bytes, else 3; 0x01: a compressed size comes first
/// u8 0xFB
/// [u24/u32 compressed size]   big-endian, when flags & 0x01
/// u24/u32 decompressed size   big-endian
/// commands
/// ```
///
/// | control      | literals              | copy length                     | copy distance - 1                         |
/// |--------------|-----------------------|---------------------------------|-------------------------------------------|
/// | `0xxxxxxx`   | `c & 3`               | `((c & 0x1C) >> 2) + 3`         | `((c & 0x60) << 3) + b1`                  |
/// | `10xxxxxx`   | `b1 >> 6`             | `(c & 0x3F) + 4`                | `((b1 & 0x3F) << 8) + b2`                 |
/// | `110xxxxx`   | `c & 3`               | `((c & 0x0C) << 6) + b3 + 5`    | `((c & 0x10) << 12) + (b1 << 8) + b2`     |
/// | `0xE0..0xFB` | `((c & 0x1F) + 1) * 4`| none                            |                                           |
/// | `0xFC..0xFF` | `c & 3`, then the end | none                            |                                           |
pub mod refpack {
    use crate::binary::{FormatError, Result};

    /// Whether `data` starts with a RefPack header.
    pub fn is_refpack(data: &[u8]) -> bool {
        data.len() >= 5 && data[1] == 0xFB && data[0] & !0x81 == 0x10
    }

    pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
        let err =
            |offset: usize, message: String| FormatError { offset, message: format!("RefPack: {message}") };
        if !is_refpack(data) {
            return Err(err(0, "no RefPack header".into()));
        }
        let size_len = if data[0] & 0x80 != 0 { 4 } else { 3 };
        let mut pos = 2;
        if data[0] & 0x01 != 0 {
            pos += size_len;
        }
        let size = data
            .get(pos..pos + size_len)
            .ok_or_else(|| err(pos, "truncated header".into()))?
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        pos += size_len;

        // No command makes more than 1028 bytes from 4, so the input bounds
        // the output however large the header says it is.
        let mut out = Vec::with_capacity(size.min(data.len().saturating_mul(257)));
        let byte =
            |at: usize| data.get(at).copied().map(usize::from).ok_or_else(|| err(at, "truncated".into()));
        while out.len() < size {
            let c = byte(pos)?;
            let (literals, copy, distance, len) = match c {
                0xFC..=0xFF => (c & 3, 0, 0, 1),
                0xE0..=0xFB => (((c & 0x1F) + 1) * 4, 0, 0, 1),
                0xC0..=0xDF => {
                    let (b1, b2, b3) = (byte(pos + 1)?, byte(pos + 2)?, byte(pos + 3)?);
                    (c & 3, ((c & 0x0C) << 6) + b3 + 5, ((c & 0x10) << 12) + (b1 << 8) + b2, 4)
                }
                0x80..=0xBF => {
                    let (b1, b2) = (byte(pos + 1)?, byte(pos + 2)?);
                    (b1 >> 6, (c & 0x3F) + 4, ((b1 & 0x3F) << 8) + b2, 3)
                }
                _ => {
                    let b1 = byte(pos + 1)?;
                    (c & 3, ((c & 0x1C) >> 2) + 3, ((c & 0x60) << 3) + b1, 2)
                }
            };
            pos += len;
            let plain = data.get(pos..pos + literals).ok_or_else(|| err(pos, "truncated literals".into()))?;
            out.extend_from_slice(plain);
            pos += literals;
            if copy > 0 {
                let from = out
                    .len()
                    .checked_sub(distance + 1)
                    .ok_or_else(|| err(pos, "copy from before the start".into()))?;
                // One byte at a time: the source may run into what is being written.
                for i in 0..copy {
                    out.push(out[from + i]);
                }
            }
            if c >= 0xFC {
                break;
            }
        }
        if out.len() != size {
            return Err(err(pos, format!("made {} bytes, the header says {size}", out.len())));
        }
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn literals_and_overlapping_copy() {
            // 3 literals then a 9-byte copy from 3 back, then 1 literal and the end.
            let data = [0x10, 0xFB, 0x00, 0x00, 0x0D, 0x1B, 0x02, b'a', b'b', b'c', 0xFD, b'!'];
            assert_eq!(decompress(&data).unwrap(), b"abcabcabcabc!");
        }

        #[test]
        fn long_literal_run() {
            // 0xE0: 4 literals; 0xFC: none, the end.
            let data = [0x10, 0xFB, 0x00, 0x00, 0x04, 0xE0, 1, 2, 3, 4, 0xFC];
            assert_eq!(decompress(&data).unwrap(), [1, 2, 3, 4]);
        }

        #[test]
        fn two_and_three_byte_copies() {
            // "abcd", then 0x80 form: 0 literals, copy 4 from 4 back; then the
            // 0xC0 form: 0 literals, copy 5 from 1 back; end.
            let data = [
                0x10, 0xFB, 0x00, 0x00, 0x0D, //
                0xE0, b'a', b'b', b'c', b'd', //
                0x80, 0x00, 0x03, //
                0xC0, 0x00, 0x00, 0x00, //
                0xFC,
            ];
            assert_eq!(decompress(&data).unwrap(), b"abcdabcdddddd");
        }

        #[test]
        fn compressed_size_and_wide_sizes() {
            // flags 0x81: a 4-byte compressed size, then a 4-byte size.
            let data = [0x91, 0xFB, 0, 0, 0, 13, 0, 0, 0, 2, 0xFE, b'h', b'i'];
            assert_eq!(decompress(&data).unwrap(), b"hi");
        }

        #[test]
        fn refuses_bad_input() {
            let good = [0x10, 0xFB, 0x00, 0x00, 0x0D, 0x1B, 0x02, b'a', b'b', b'c', 0xFD, b'!'];
            assert!(decompress(&good[..8]).is_err(), "truncated");
            assert!(decompress(b"DDS \0\0\0").is_err(), "no header");
            // A copy reaching before the first byte.
            assert!(decompress(&[0x10, 0xFB, 0, 0, 3, 0x00, 0x05]).is_err());
            // Ends before the size in the header.
            assert!(decompress(&[0x10, 0xFB, 0, 0, 9, 0xFD, b'x']).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::Writer;

    fn header(
        w: &mut Writer,
        magic: &[u8; 4],
        count: u32,
        index_size: u32,
        index_at: usize,
        index_offset: u32,
    ) {
        w.bytes(magic);
        w.u32(2);
        w.zeros(0x24 - w.pos());
        w.u32(count);
        w.u32(0);
        w.u32(index_size);
        w.zeros(index_at - w.pos());
        w.u32(index_offset);
        w.zeros(HEADER_SIZE - w.pos());
    }

    #[test]
    fn dbpf_with_constant_fields() {
        let payload = [0x10, 0xFB, 0, 0, 2, 0xFE, b'h', b'i'];
        let mut w = Writer::new(Endian::Little);
        let index_offset = (HEADER_SIZE + payload.len()) as u32;
        header(&mut w, b"DBPF", 1, 12 + 20, 0x40, index_offset);
        w.bytes(&payload);
        w.u32(1 | 4); // type and instance high constant
        w.u32(0xF9E5_0586);
        w.u32(0);
        w.u32(0x1234); // group
        w.u32(1); // instance low
        w.u32(HEADER_SIZE as u32);
        w.u32(0x8000_0000 | payload.len() as u32);
        w.u32(2);
        w.u16(0xFFFF);
        w.u16(1);
        let data = w.into_bytes();
        let package = Package::read(&data).unwrap();
        assert_eq!(package.variant, Variant::Dbpf);
        let entry = package.entries[0];
        assert_eq!(entry.key, ResourceKey { instance: 1, type_id: 0xF9E5_0586, group: 0x1234 });
        assert_eq!(package.extract(&entry).unwrap(), b"hi");
    }

    #[test]
    fn agents() {
        let mut w = Writer::new(Endian::Big);
        let index_offset = HEADER_SIZE as u32 + 4;
        header(&mut w, b"FPBD", 2, 0, 0x44, index_offset);
        w.bytes(b"abcd");
        w.u64(2);
        for (type_id, group) in [(0x2954_E734, 0), (0xE664_0542, 7)] {
            w.u32(type_id);
            w.u32(group);
            w.u32(1);
            w.u32(0);
        }
        w.u64(0xAA);
        w.u32(HEADER_SIZE as u32);
        w.u32(4);
        w.u64(0xBB);
        w.u32(HEADER_SIZE as u32 + 1);
        w.u32(2);
        w.u32(2);
        let data = w.into_bytes();
        let package = Package::read(&data).unwrap();
        assert_eq!(package.variant, Variant::Agents);
        let [a, b] = package.entries[..] else { panic!("{:?}", package.entries) };
        assert_eq!(a.key, ResourceKey { instance: 0xAA, type_id: 0x2954_E734, group: 0 });
        assert_eq!(package.extract(&a).unwrap(), b"abcd");
        assert_eq!(b.key, ResourceKey { instance: 0xBB, type_id: 0xE664_0542, group: 7 });
        assert_eq!(package.extract(&b).unwrap(), b"bc");
    }

    #[test]
    fn truncated() {
        assert!(Package::read(b"DBPF\x02\0\0\0").is_err());
    }
}
