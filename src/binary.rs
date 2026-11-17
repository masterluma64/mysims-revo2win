//! Bounds-checked reading and writing of fixed-size values in either byte order.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("at 0x{offset:x}: {message}")]
pub struct FormatError {
    pub offset: usize,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, FormatError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    endian: Endian,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], endian: Endian) -> Self {
        Self { data, pos: 0, endian }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn error(&self, message: impl fmt::Display) -> FormatError {
        FormatError { offset: self.pos, message: message.to_string() }
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(
                self.error(format_args!("offset 0x{pos:x} is past the end (0x{:x})", self.data.len()))
            );
        }
        self.pos = pos;
        Ok(())
    }

    pub fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&end| end <= self.data.len());
        let Some(end) = end else {
            return Err(self.error(format_args!("{n} bytes requested, {} left", self.remaining())));
        };
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.array()?;
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(b),
            Endian::Big => u16::from_be_bytes(b),
        })
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.array()?;
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(b),
            Endian::Big => u32::from_be_bytes(b),
        })
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> Result<u64> {
        let b = self.array()?;
        Ok(match self.endian {
            Endian::Little => u64::from_le_bytes(b),
            Endian::Big => u64::from_be_bytes(b),
        })
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn vec3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// A count read from the file, refused when `count * item_size` could not
    /// fit in what is left of it.
    pub fn count(&mut self, item_size: usize, what: &str) -> Result<usize> {
        let n = self.u32()? as usize;
        if n.saturating_mul(item_size.max(1)) > self.remaining() {
            return Err(self.error(format_args!("{what} count {n} does not fit in the file")));
        }
        Ok(n)
    }
}

pub struct Writer {
    data: Vec<u8>,
    endian: Endian,
}

impl Writer {
    pub fn new(endian: Endian) -> Self {
        Self { data: Vec::new(), endian }
    }

    pub fn pos(&self) -> usize {
        self.data.len()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.data.extend_from_slice(b);
    }

    pub fn zeros(&mut self, n: usize) {
        self.data.resize(self.data.len() + n, 0);
    }

    /// Pads with zeros up to the next multiple of `align`.
    pub fn align(&mut self, align: usize) {
        let pad = self.data.len().next_multiple_of(align) - self.data.len();
        self.zeros(pad);
    }

    pub fn u8(&mut self, v: u8) {
        self.data.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        match self.endian {
            Endian::Little => self.bytes(&v.to_le_bytes()),
            Endian::Big => self.bytes(&v.to_be_bytes()),
        }
    }

    pub fn u32(&mut self, v: u32) {
        match self.endian {
            Endian::Little => self.bytes(&v.to_le_bytes()),
            Endian::Big => self.bytes(&v.to_be_bytes()),
        }
    }

    pub fn u64(&mut self, v: u64) {
        match self.endian {
            Endian::Little => self.bytes(&v.to_le_bytes()),
            Endian::Big => self.bytes(&v.to_be_bytes()),
        }
    }

    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    pub fn vec3(&mut self, v: [f32; 3]) {
        v.iter().for_each(|c| self.f32(*c));
    }

    /// Writes a placeholder u32 and returns where it is, for [`Writer::patch_u32`].
    pub fn reserve_u32(&mut self) -> usize {
        let at = self.pos();
        self.u32(0);
        at
    }

    pub fn patch_u32(&mut self, at: usize, v: u32) {
        let b = match self.endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        };
        self.data[at..at + 4].copy_from_slice(&b);
    }
}

/// A length or offset as the u32 the formats store.
pub fn u32_len(n: usize) -> u32 {
    u32::try_from(n).expect("larger than 4 GiB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_both_orders() {
        for endian in [Endian::Little, Endian::Big] {
            let mut w = Writer::new(endian);
            w.u8(1);
            w.u16(0x0203);
            w.u32(0x0405_0607);
            w.u64(0x0809_0a0b_0c0d_0e0f);
            w.f32(1.5);
            let at = w.reserve_u32();
            w.patch_u32(at, 7);
            w.align(32);
            let bytes = w.into_bytes();
            assert_eq!(bytes.len(), 32);
            let mut r = Reader::new(&bytes, endian);
            assert_eq!(r.u8().unwrap(), 1);
            assert_eq!(r.u16().unwrap(), 0x0203);
            assert_eq!(r.u32().unwrap(), 0x0405_0607);
            assert_eq!(r.u64().unwrap(), 0x0809_0a0b_0c0d_0e0f);
            assert_eq!(r.f32().unwrap(), 1.5);
            assert_eq!(r.u32().unwrap(), 7);
        }
    }

    #[test]
    fn refuses_reads_past_the_end() {
        let mut r = Reader::new(&[1, 2, 3], Endian::Big);
        assert!(r.u32().is_err());
        assert!(r.seek(4).is_err());
        assert_eq!(r.u16().unwrap(), 0x0102);
    }
}
