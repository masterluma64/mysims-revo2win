//! Pieces both formats share.

/// A model's bones and their bind pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Rig {
    /// FNV32 of each bone's name.
    pub bone_hashes: Vec<u32>,
    /// One inverse bind matrix per bone, translation in elements 12 to 14.
    /// Both formats store the sixteen floats in this order.
    pub inverse_binds: Vec<[f32; 16]>,
}

/// A model-wide named parameter, such as `0x9281771A` = `"no_shadow"` (the
/// shadow model to use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraParam {
    pub key: u32,
    /// The value without its terminating NUL.
    pub value: Vec<u8>,
}

/// Splits a value block of NUL-terminated strings into one value per key.
pub fn split_extra_values(keys: &[u32], block: &[u8]) -> Option<Vec<ExtraParam>> {
    let mut rest = block;
    let mut out = Vec::with_capacity(keys.len());
    for &key in keys {
        let end = rest.iter().position(|&b| b == 0)?;
        out.push(ExtraParam { key, value: rest[..end].to_vec() });
        rest = &rest[end + 1..];
    }
    rest.is_empty().then_some(out)
}

/// The value block for `params`: each value followed by a NUL.
pub fn join_extra_values(params: &[ExtraParam]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in params {
        out.extend_from_slice(&p.value);
        out.push(0);
    }
    out
}

/// An axis-aligned box that grows to include points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    /// The box the RevoModel header holds in every shipped file: min at
    /// `f32::MAX`, max at `-f32::MAX`.
    pub const EMPTY: Bounds = Bounds { min: [f32::MAX; 3], max: [-f32::MAX; 3] };

    pub fn add(&mut self, p: [f32; 3]) {
        for (i, v) in p.into_iter().enumerate() {
            self.min[i] = self.min[i].min(v);
            self.max[i] = self.max[i].max(v);
        }
    }

    pub fn union(&mut self, other: &Bounds) {
        self.add(other.min);
        self.add(other.max);
    }

    pub fn is_empty(&self) -> bool {
        (0..3).any(|i| self.min[i] > self.max[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_values() {
        let params = vec![
            ExtraParam { key: 1, value: b"no_shadow".to_vec() },
            ExtraParam { key: 2, value: b"1".to_vec() },
        ];
        let block = join_extra_values(&params);
        assert_eq!(block, b"no_shadow\x001\x00");
        assert_eq!(split_extra_values(&[1, 2], &block).unwrap(), params);
        assert!(split_extra_values(&[1], &block).is_none());
        assert!(split_extra_values(&[1, 2, 3], &block).is_none());
    }

    #[test]
    fn bounds() {
        let mut b = Bounds::EMPTY;
        assert!(b.is_empty());
        b.add([1.0, -2.0, 3.0]);
        b.add([-1.0, 2.0, 0.0]);
        assert_eq!(b, Bounds { min: [-1.0, -2.0, 0.0], max: [1.0, 2.0, 3.0] });
    }
}
