//! WindowsModel (`WMDL`, type `0xB359C791`): the model format of the Cozy
//! Bundle releases of MySims and MySims Kingdom. Little-endian, sections
//! packed one after another with no alignment.
//!
//! ```text
//! u8 4; "WMDL"; u8 minor (2); u8 major (1); u8[2] pad
//! vec3 bounds_min, bounds_max
//! u32 extra_count; u32 extra_keys[extra_count]
//! if extra_count > 0 { u32 size; u8 values[size] } else { u8 pad }
//! u32 rig_count;  rig  { u32 bone_count; u32 bone_hashes[n]; f32 inverse_binds[n][16] }
//! u32 mesh_count; mesh {
//!   ResourceKey material; vec3 bounds_min, bounds_max; u32 unknown[2]
//!   u32 name; u32 unknown (3); u8[12] unknown
//!   u32 vertex_count; u32 face_count
//!   u32 key_count; key { u32 offset; u8 type; u8 usage; u8 set }   (7 bytes)
//!   u32 vertex_bytes; u8 vertices[vertex_bytes]
//!   u32 face_bytes;   u16 indices[face_count * 3]
//!   i32 rig_index (-1 = not skinned)
//! }
//! ```

use crate::binary::{Endian, Reader, Result, Writer, u32_len};
use crate::game::ResourceKey;
pub use crate::model::Rig;

const MAGIC: &[u8; 5] = b"\x04WMDL";

/// How a vertex element is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    Float2,
    Float3,
    Float4,
    /// Four bytes: bone indices.
    Byte4,
    /// Four bytes, stored A, B, G, R.
    Color,
    Other(u8),
}

impl ElementType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Float2,
            2 => Self::Float3,
            3 => Self::Float4,
            4 => Self::Byte4,
            5 => Self::Color,
            other => Self::Other(other),
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::Float2 => 1,
            Self::Float3 => 2,
            Self::Float4 => 3,
            Self::Byte4 => 4,
            Self::Color => 5,
            Self::Other(v) => v,
        }
    }

    pub fn size(self) -> Option<usize> {
        match self {
            Self::Float2 => Some(8),
            Self::Float3 => Some(12),
            Self::Float4 => Some(16),
            Self::Byte4 | Self::Color => Some(4),
            Self::Other(_) => None,
        }
    }
}

/// What a vertex element holds.
pub mod usage {
    pub const POSITION: u8 = 0;
    pub const NORMAL: u8 = 1;
    /// The element's `set` is the UV set.
    pub const UV: u8 = 2;
    pub const BONES: u8 = 3;
    pub const WEIGHTS: u8 = 4;
    pub const COLOR: u8 = 6;
}

/// One element of the interleaved vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexKey {
    pub offset: u32,
    pub kind: ElementType,
    pub usage: u8,
    pub set: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub material: ResourceKey,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub unknown0: [u32; 2],
    pub name: u32,
    /// 3 in every shipped file.
    pub unknown1: u32,
    /// Zero in every shipped file.
    pub unknown2: [u8; 12],
    pub vertex_count: u32,
    pub keys: Vec<VertexKey>,
    /// Interleaved vertices, `vertex_count` of them.
    pub vertices: Vec<u8>,
    /// Triangle list.
    pub indices: Vec<u16>,
    /// Index into the model's rigs; -1 when the mesh is not skinned.
    pub rig_index: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowsModel {
    pub minor_version: u8,
    pub major_version: u8,
    pub header_pad: [u8; 2],
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub extra_keys: Vec<u32>,
    /// The extra parameters' values: one NUL-terminated string per key.
    /// `None` when there are no extra parameters.
    pub extra_values: Option<Vec<u8>>,
    /// The byte written instead of the value block when there are no extra
    /// parameters.
    pub extra_pad: u8,
    pub rigs: Vec<Rig>,
    pub meshes: Vec<Mesh>,
}

impl WindowsModel {
    pub const MINOR_VERSION: u8 = 2;
    pub const MAJOR_VERSION: u8 = 1;
    pub const MESH_UNKNOWN1: u32 = 3;

    pub fn is_windows_model(data: &[u8]) -> bool {
        data.starts_with(MAGIC)
    }

    pub fn read(data: &[u8]) -> Result<Self> {
        let r = &mut Reader::new(data, Endian::Little);
        if r.bytes(5)? != MAGIC {
            return Err(r.error("not a WindowsModel (no `\\x04WMDL` magic)"));
        }
        let minor_version = r.u8()?;
        let major_version = r.u8()?;
        let header_pad = r.array()?;
        let bounds_min = r.vec3()?;
        let bounds_max = r.vec3()?;

        let extra_count = r.count(4, "extra parameter")?;
        let extra_keys = (0..extra_count).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
        let (extra_values, extra_pad) = if extra_count > 0 {
            let size = r.count(1, "extra value byte")?;
            (Some(r.bytes(size)?.to_vec()), 0)
        } else {
            (None, r.u8()?)
        };

        let rig_count = r.count(4, "rig")?;
        let mut rigs = Vec::with_capacity(rig_count);
        for _ in 0..rig_count {
            let bones = r.count(68, "bone")?;
            let bone_hashes = (0..bones).map(|_| r.u32()).collect::<Result<_>>()?;
            let inverse_binds = (0..bones)
                .map(|_| {
                    let mut m = [0f32; 16];
                    for v in &mut m {
                        *v = r.f32()?;
                    }
                    Ok(m)
                })
                .collect::<Result<_>>()?;
            rigs.push(Rig { bone_hashes, inverse_binds });
        }

        let mesh_count = r.count(80, "mesh")?;
        let mut meshes = Vec::with_capacity(mesh_count);
        for _ in 0..mesh_count {
            meshes.push(read_mesh(r)?);
        }
        if r.remaining() != 0 {
            return Err(r.error(format_args!("{} bytes after the last mesh", r.remaining())));
        }
        Ok(Self {
            minor_version,
            major_version,
            header_pad,
            bounds_min,
            bounds_max,
            extra_keys,
            extra_values,
            extra_pad,
            rigs,
            meshes,
        })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new(Endian::Little);
        w.bytes(MAGIC);
        w.u8(self.minor_version);
        w.u8(self.major_version);
        w.bytes(&self.header_pad);
        w.vec3(self.bounds_min);
        w.vec3(self.bounds_max);
        w.u32(u32_len(self.extra_keys.len()));
        self.extra_keys.iter().for_each(|k| w.u32(*k));
        match &self.extra_values {
            Some(values) if !self.extra_keys.is_empty() => {
                w.u32(u32_len(values.len()));
                w.bytes(values);
            }
            _ => w.u8(self.extra_pad),
        }
        w.u32(u32_len(self.rigs.len()));
        for rig in &self.rigs {
            w.u32(u32_len(rig.bone_hashes.len()));
            rig.bone_hashes.iter().for_each(|h| w.u32(*h));
            rig.inverse_binds.iter().flatten().for_each(|v| w.f32(*v));
        }
        w.u32(u32_len(self.meshes.len()));
        for mesh in &self.meshes {
            write_mesh(&mut w, mesh);
        }
        w.into_bytes()
    }
}

fn read_mesh(r: &mut Reader<'_>) -> Result<Mesh> {
    let material = ResourceKey { instance: r.u64()?, type_id: r.u32()?, group: r.u32()? };
    let bounds_min = r.vec3()?;
    let bounds_max = r.vec3()?;
    let unknown0 = [r.u32()?, r.u32()?];
    let name = r.u32()?;
    let unknown1 = r.u32()?;
    let unknown2 = r.array()?;
    let vertex_count = r.u32()?;
    let face_count = r.u32()? as usize;
    let key_count = r.count(7, "vertex key")?;
    let keys = (0..key_count)
        .map(|_| {
            Ok(VertexKey {
                offset: r.u32()?,
                kind: ElementType::from_u8(r.u8()?),
                usage: r.u8()?,
                set: r.u8()?,
            })
        })
        .collect::<Result<_>>()?;
    let vertex_bytes = r.count(1, "vertex byte")?;
    if vertex_count == 0 && vertex_bytes != 0
        || vertex_count != 0 && vertex_bytes % vertex_count as usize != 0
    {
        return Err(r.error(format_args!("{vertex_bytes} vertex bytes for {vertex_count} vertices")));
    }
    let vertices = r.bytes(vertex_bytes)?.to_vec();
    let face_bytes = r.u32()? as usize;
    if face_bytes != face_count * 6 {
        return Err(r.error(format_args!("{face_bytes} index bytes for {face_count} triangles")));
    }
    let indices = (0..face_count * 3).map(|_| r.u16()).collect::<Result<_>>()?;
    let rig_index = r.i32()?;
    Ok(Mesh {
        material,
        bounds_min,
        bounds_max,
        unknown0,
        name,
        unknown1,
        unknown2,
        vertex_count,
        keys,
        vertices,
        indices,
        rig_index,
    })
}

fn write_mesh(w: &mut Writer, mesh: &Mesh) {
    w.u64(mesh.material.instance);
    w.u32(mesh.material.type_id);
    w.u32(mesh.material.group);
    w.vec3(mesh.bounds_min);
    w.vec3(mesh.bounds_max);
    mesh.unknown0.iter().for_each(|v| w.u32(*v));
    w.u32(mesh.name);
    w.u32(mesh.unknown1);
    w.bytes(&mesh.unknown2);
    w.u32(mesh.vertex_count);
    w.u32(u32_len(mesh.indices.len() / 3));
    w.u32(u32_len(mesh.keys.len()));
    for key in &mesh.keys {
        w.u32(key.offset);
        w.u8(key.kind.to_u8());
        w.u8(key.usage);
        w.u8(key.set);
    }
    w.u32(u32_len(mesh.vertices.len()));
    w.bytes(&mesh.vertices);
    w.u32(u32_len(mesh.indices.len() * 2));
    mesh.indices.iter().for_each(|i| w.u16(*i));
    w.u32(mesh.rig_index as u32);
}

impl Mesh {
    pub fn stride(&self) -> usize {
        if self.vertex_count == 0 { 0 } else { self.vertices.len() / self.vertex_count as usize }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::types;

    pub(crate) fn sample() -> WindowsModel {
        let mut vertices = Vec::new();
        for i in 0..3u8 {
            for v in [f32::from(i), 0.0, 1.0, 0.0, 1.0, 0.0, 0.5, f32::from(i) / 2.0] {
                vertices.extend_from_slice(&v.to_le_bytes());
            }
        }
        WindowsModel {
            minor_version: 2,
            major_version: 1,
            header_pad: [0, 0],
            bounds_min: [0.0; 3],
            bounds_max: [2.0, 0.0, 1.0],
            extra_keys: vec![0x9281_771a],
            extra_values: Some(b"no_shadow\0".to_vec()),
            extra_pad: 0,
            rigs: vec![Rig { bone_hashes: vec![7], inverse_binds: vec![[1.0; 16]] }],
            meshes: vec![Mesh {
                material: ResourceKey { instance: 5, type_id: types::MATERIAL, group: 9 },
                bounds_min: [0.0; 3],
                bounds_max: [2.0, 0.0, 1.0],
                unknown0: [0, 0],
                name: 0x2ca3_3bdb,
                unknown1: 3,
                unknown2: [0; 12],
                vertex_count: 3,
                keys: vec![
                    VertexKey { offset: 0, kind: ElementType::Float3, usage: usage::POSITION, set: 0 },
                    VertexKey { offset: 12, kind: ElementType::Float3, usage: usage::NORMAL, set: 0 },
                    VertexKey { offset: 24, kind: ElementType::Float2, usage: usage::UV, set: 0 },
                ],
                vertices,
                indices: vec![0, 1, 2],
                rig_index: -1,
            }],
        }
    }

    #[test]
    fn round_trip() {
        let model = sample();
        let bytes = model.write();
        assert!(WindowsModel::is_windows_model(&bytes));
        assert_eq!(WindowsModel::read(&bytes).unwrap(), model);
    }

    #[test]
    fn rejects_truncated_and_trailing() {
        let bytes = sample().write();
        assert!(WindowsModel::read(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(WindowsModel::read(&longer).is_err());
    }
}
