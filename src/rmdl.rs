//! RevoModel (`RMDL`): the model format of the Wii releases of MySims, MySims
//! Kingdom (type `0xF9E50586`) and MySims Agents (type `0x2954E734`).
//!
//! Big-endian, addressed by absolute offsets. Each mesh is drawn by a GX
//! display list that indexes separate attribute arrays, so the file is close
//! to what the Wii's GPU consumes. See `docs/revomodel.md` for the layout.

use crate::binary::{Endian, Reader, Result, Writer, u32_len};
use crate::game::{Game, ResourceKey};
use crate::model::{Bounds, ExtraParam, Rig};

const MAGIC: &[u8; 4] = b"RMDL";
const HEADER_SIZE: usize = 0x58;
const MESH_INFO_SIZE: usize = 0x5c;

/// GX vertex attributes (`GXAttr`) the arrays can hold.
pub mod attr {
    pub const POSITION: u8 = 9;
    pub const NORMAL: u8 = 10;
    pub const COLOR0: u8 = 11;
    pub const COLOR1: u8 = 12;
    pub const TEX0: u8 = 13;
    pub const TEX7: u8 = 20;

    /// The UV set a texture coordinate attribute is for.
    pub fn uv_set(attribute: u8) -> Option<u8> {
        (TEX0..=TEX7).contains(&attribute).then(|| attribute - TEX0)
    }
}

/// GX display list opcodes.
pub mod op {
    pub const NOP: u8 = 0x00;
    /// Loads a position matrix from the indexed array into matrix memory.
    pub const LOAD_POSITION_MATRIX: u8 = 0x20;
    /// Loads a normal matrix from the indexed array into matrix memory.
    pub const LOAD_NORMAL_MATRIX: u8 = 0x28;
    pub const LOAD_TEXTURE_MATRIX: u8 = 0x30;
    pub const LOAD_LIGHT: u8 = 0x38;
    pub const QUADS: u8 = 0x80;
    pub const TRIANGLES: u8 = 0x90;
    pub const TRIANGLE_STRIP: u8 = 0x98;
    pub const TRIANGLE_FAN: u8 = 0xa0;
    pub const LINES: u8 = 0xa8;
    pub const LINE_STRIP: u8 = 0xb0;
    pub const POINTS: u8 = 0xb8;

    pub fn is_load(cmd: u8) -> bool {
        matches!(cmd, LOAD_POSITION_MATRIX | LOAD_NORMAL_MATRIX | LOAD_TEXTURE_MATRIX | LOAD_LIGHT)
    }

    pub fn is_draw(cmd: u8) -> bool {
        cmd & 0x80 != 0 && cmd & 0x78 <= 0x38
    }
}

/// The ten position matrix slots GX can index per vertex.
pub const MATRIX_SLOTS: usize = 10;

/// One display list command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A no-op. The exporter puts one after each run of matrix loads.
    Nop,
    /// `command`, then the array index and a descriptor: the matrix memory
    /// address in the low 12 bits and the number of floats minus one above.
    Load { command: u8, index: u16, descriptor: u16 },
    /// A primitive (`op::TRIANGLE_STRIP` and so on; the low three bits pick
    /// the vertex format, always 0). Each vertex has a position matrix index
    /// (`matrices`, only in skinned meshes) and one u16 per array, in array
    /// order (`indices`).
    Draw { primitive: u8, matrices: Vec<u8>, indices: Vec<u16> },
}

impl Command {
    pub fn load_position_matrix(palette_index: u16, slot: usize) -> Self {
        Command::Load {
            command: op::LOAD_POSITION_MATRIX,
            index: palette_index,
            descriptor: 0xb000 | (slot as u16 * 12),
        }
    }

    pub fn load_normal_matrix(palette_index: u16, slot: usize) -> Self {
        Command::Load {
            command: op::LOAD_NORMAL_MATRIX,
            index: palette_index,
            descriptor: 0x8000 | (0x400 + slot as u16 * 9),
        }
    }

    /// The position matrix slot a `Load` of a position matrix fills.
    pub fn position_slot(&self) -> Option<(u16, usize)> {
        match *self {
            Command::Load { command: op::LOAD_POSITION_MATRIX, index, descriptor } => {
                Some((index, usize::from(descriptor & 0xfff) / 12))
            }
            _ => None,
        }
    }
}

/// One attribute's values, stored big-endian and indexed by the display list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexArray {
    /// The GX attribute (`attr::POSITION` and so on).
    pub attribute: u8,
    /// Bytes per element: 12 for positions and normals (three f32), 8 for
    /// texture coordinates (two f32), 4 for colours (RGBA8).
    pub stride: u8,
    pub data: Vec<u8>,
}

impl VertexArray {
    pub fn len(&self) -> usize {
        if self.stride == 0 { 0 } else { self.data.len() / usize::from(self.stride) }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn f32(&self, index: usize, component: usize) -> Option<f32> {
        let at = index * usize::from(self.stride) + component * 4;
        let b = self.data.get(at..at + 4)?;
        Some(f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn element(&self, index: usize) -> Option<&[u8]> {
        let s = usize::from(self.stride);
        self.data.get(index * s..index * s + s)
    }
}

/// A matrix the skinned vertices are drawn with: a blend of up to four bones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaletteEntry {
    pub weights: [f32; 4],
    /// Indices into the mesh's `Skin::bones`; `0xFF` where unused.
    pub bones: [u8; 4],
}

impl PaletteEntry {
    pub const NO_BONE: u8 = 0xff;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    /// The matrices the display list loads (`Command::Load` indices).
    pub palette: Vec<PaletteEntry>,
    /// FNV32 names of the bones the palette uses.
    pub bones: Vec<u32>,
    /// The rig the mesh is bound to.
    pub rig: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub material: ResourceKey,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// The two words WindowsModel calls "unknown 0"; copied between formats.
    pub unknown0: [u32; 2],
    pub name: u32,
    /// In GX attribute order.
    pub arrays: Vec<VertexArray>,
    pub display_list: Vec<Command>,
    /// The size the display list is padded to, when more than the 32-byte
    /// boundary after its commands. The exporter sized these buffers by its
    /// own estimate; this keeps a re-written file identical.
    pub display_list_capacity: usize,
    pub skin: Option<Skin>,
}

impl Mesh {
    pub fn array(&self, attribute: u8) -> Option<&VertexArray> {
        self.arrays.iter().find(|a| a.attribute == attribute)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevoModel {
    /// `0x00000101` in every shipped file.
    pub version: u32,
    /// `Bounds::EMPTY` in every shipped file.
    pub bounds: Bounds,
    pub extras: Vec<ExtraParam>,
    pub rigs: Vec<Rig>,
    pub meshes: Vec<Mesh>,
}

impl RevoModel {
    pub const VERSION: u32 = 0x0000_0101;

    pub fn is_revo_model(data: &[u8]) -> bool {
        data.starts_with(MAGIC)
    }

    pub fn read(data: &[u8]) -> Result<Self> {
        let r = &mut Reader::new(data, Endian::Big);
        if r.bytes(4)? != MAGIC {
            return Err(r.error("not a RevoModel (no `RMDL` magic)"));
        }
        let version = r.u32()?;
        let bounds = Bounds { min: r.vec3()?, max: r.vec3()? };
        let mesh_count = r.u32()? as usize;
        let mesh_table = r.u32()? as usize;
        let extra_count = r.u32()? as usize;
        let extra_keys_at = r.u32()? as usize;
        let extra_values_at = r.u32()? as usize;
        let rig_table = r.u32()? as usize;

        let mut extras = Vec::with_capacity(extra_count.min(1024));
        if extra_count > 0 {
            r.seek(extra_keys_at)?;
            let keys = (0..extra_count).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
            r.seek(extra_values_at)?;
            for key in keys {
                let mut value = Vec::new();
                loop {
                    match r.u8()? {
                        0 => break,
                        b => value.push(b),
                    }
                }
                extras.push(ExtraParam { key, value });
            }
        }

        // The rig table is a list of offsets ending with a zero.
        let mut rig_offsets = Vec::new();
        let mut rigs = Vec::new();
        if rig_table != 0 {
            r.seek(rig_table)?;
            loop {
                let at = r.u32()? as usize;
                if at == 0 {
                    break;
                }
                rig_offsets.push(at);
            }
            for &at in &rig_offsets {
                r.seek(at)?;
                let n = r.count(68, "bone")?;
                let hashes_at = r.u32()? as usize;
                let matrices_at = r.u32()? as usize;
                r.seek(hashes_at)?;
                let bone_hashes = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
                r.seek(matrices_at)?;
                let inverse_binds = (0..n)
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
        }

        if mesh_count.saturating_mul(4 + MESH_INFO_SIZE) > data.len() {
            return Err(r.error(format_args!("mesh count {mesh_count} does not fit in the file")));
        }
        let mut meshes = Vec::with_capacity(mesh_count);
        for m in 0..mesh_count {
            r.seek(mesh_table + 4 * m)?;
            let at = r.u32()? as usize;
            r.seek(at)?;
            meshes.push(read_mesh(r, &rig_offsets).map_err(|mut e| {
                e.message = format!("mesh {m}: {}", e.message);
                e
            })?);
        }

        Ok(Self { version, bounds, extras, rigs, meshes })
    }

    /// Writes the file the way the game's exporter laid it out. Agents files
    /// end where their last section does; MySims and Kingdom files are padded
    /// to 32 bytes.
    pub fn write(&self, game: Game) -> Vec<u8> {
        let mut w = Writer::new(Endian::Big);
        w.bytes(MAGIC);
        w.u32(self.version);
        w.vec3(self.bounds.min);
        w.vec3(self.bounds.max);
        w.u32(u32_len(self.meshes.len()));
        w.u32(u32_len(HEADER_SIZE));
        w.u32(u32_len(self.extras.len()));
        let extra_keys_at = w.reserve_u32();
        let extra_values_at = w.reserve_u32();
        let rig_table_at = w.reserve_u32();
        w.zeros(HEADER_SIZE - w.pos());

        let table: Vec<usize> = self.meshes.iter().map(|_| w.reserve_u32()).collect();
        // Where each skinned mesh names its rig, filled in once the rigs are written.
        let mut rig_refs = Vec::new();
        for (mesh, slot) in self.meshes.iter().zip(table) {
            w.patch_u32(slot, u32_len(w.pos()));
            if let Some(rig_ref) = write_mesh(&mut w, mesh) {
                rig_refs.push((rig_ref, mesh.skin.as_ref().map_or(0, |s| s.rig)));
            }
        }

        if !self.extras.is_empty() {
            w.patch_u32(extra_keys_at, u32_len(w.pos()));
            self.extras.iter().for_each(|e| w.u32(e.key));
            w.patch_u32(extra_values_at, u32_len(w.pos()));
            for e in &self.extras {
                w.bytes(&e.value);
                w.u8(0);
            }
        }

        if !self.rigs.is_empty() {
            w.align(4);
            w.patch_u32(rig_table_at, u32_len(w.pos()));
            let slots: Vec<usize> = self.rigs.iter().map(|_| w.reserve_u32()).collect();
            w.u32(0);
            let mut rig_at = Vec::with_capacity(self.rigs.len());
            for (rig, slot) in self.rigs.iter().zip(slots) {
                let at = w.pos();
                rig_at.push(at);
                w.patch_u32(slot, u32_len(at));
                let n = rig.bone_hashes.len();
                w.u32(u32_len(n));
                w.u32(u32_len(at + 12));
                w.u32(u32_len(at + 12 + 4 * n));
                rig.bone_hashes.iter().for_each(|h| w.u32(*h));
                rig.inverse_binds.iter().flatten().for_each(|v| w.f32(*v));
            }
            for (at, rig) in rig_refs {
                if let Some(&offset) = rig_at.get(rig) {
                    w.patch_u32(at, u32_len(offset));
                }
            }
        }

        if game != Game::Agents {
            w.align(32);
        }
        w.into_bytes()
    }
}

fn read_mesh(r: &mut Reader<'_>, rig_offsets: &[usize]) -> Result<Mesh> {
    let dl_size = r.u32()? as usize;
    let dl_at = r.u32()? as usize;
    let array_count = r.u32()? as usize;
    let array_table = r.u32()? as usize;
    let material = ResourceKey { instance: r.u64()?, type_id: r.u32()?, group: r.u32()? };
    let bounds_min = r.vec3()?;
    let bounds_max = r.vec3()?;
    let unknown0 = [r.u32()?, r.u32()?];
    let name = r.u32()?;
    r.bytes(12)?;
    let skin_at = r.u32()? as usize;
    let bones_at = r.u32()? as usize;
    let rig_at = r.u32()? as usize;

    if array_count > 21 {
        return Err(r.error(format_args!("{array_count} vertex arrays")));
    }
    r.seek(array_table)?;
    let mut layout = Vec::with_capacity(array_count);
    for _ in 0..array_count {
        let attribute = r.u8()?;
        let stride = r.u8()?;
        r.bytes(2)?;
        let at = r.u32()? as usize;
        if stride == 0 {
            return Err(r.error(format_args!("attribute {attribute} has a stride of 0")));
        }
        layout.push((attribute, stride, at));
    }

    let skinned = skin_at != 0;
    r.seek(dl_at)?;
    let (display_list, dl_content) = read_display_list(r, dl_size, array_count, skinned)?;

    // Arrays carry no length: each is as long as the display list needs.
    let mut counts = vec![0usize; array_count];
    for cmd in &display_list {
        if let Command::Draw { indices, .. } = cmd {
            for v in indices.chunks_exact(array_count.max(1)) {
                for (count, &i) in counts.iter_mut().zip(v) {
                    *count = (*count).max(usize::from(i) + 1);
                }
            }
        }
    }
    let mut arrays = Vec::with_capacity(array_count);
    for (&(attribute, stride, at), count) in layout.iter().zip(counts) {
        r.seek(at)?;
        let data = r.bytes(count * usize::from(stride))?.to_vec();
        arrays.push(VertexArray { attribute, stride, data });
    }

    let skin = if skinned {
        r.seek(skin_at)?;
        let palette_at = r.u32()? as usize;
        let _scratch_at = r.u32()?;
        let n = r.count(20, "palette entry")?;
        r.seek(palette_at)?;
        let palette = (0..n)
            .map(|_| {
                Ok(PaletteEntry { weights: [r.f32()?, r.f32()?, r.f32()?, r.f32()?], bones: r.array()? })
            })
            .collect::<Result<Vec<_>>>()?;
        r.seek(bones_at)?;
        let n = r.count(4, "bone")?;
        let bones = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
        let Some(rig) = rig_offsets.iter().position(|&o| o == rig_at) else {
            return Err(r.error(format_args!("the rig at 0x{rig_at:x} is not in the rig table")));
        };
        Some(Skin { palette, bones, rig })
    } else {
        None
    };

    Ok(Mesh {
        material,
        bounds_min,
        bounds_max,
        unknown0,
        name,
        arrays,
        display_list,
        display_list_capacity: if dl_size > dl_content.next_multiple_of(32) { dl_size } else { 0 },
        skin,
    })
}

/// The commands, and how many bytes they take before the padding.
fn read_display_list(
    r: &mut Reader<'_>,
    size: usize,
    arrays: usize,
    skinned: bool,
) -> Result<(Vec<Command>, usize)> {
    let start = r.pos();
    let end = start + size;
    let mut content = 0;
    if end > r.len() {
        return Err(r.error(format_args!("the display list (0x{size:x} bytes) runs past the end")));
    }
    let mut out = Vec::new();
    while r.pos() < end {
        let cmd = r.u8()?;
        if cmd == op::NOP {
            let mut nops = 1;
            while r.pos() < end && r.peek() == Some(op::NOP) {
                r.u8()?;
                nops += 1;
            }
            if r.pos() == end {
                // Zeros up to the end are padding, not commands.
                break;
            }
            out.extend(std::iter::repeat_n(Command::Nop, nops));
            continue;
        }
        if op::is_load(cmd) {
            out.push(Command::Load { command: cmd, index: r.u16()?, descriptor: r.u16()? });
        } else if op::is_draw(cmd) {
            let n = usize::from(r.u16()?);
            let mut matrices = Vec::with_capacity(if skinned { n } else { 0 });
            let mut indices = Vec::with_capacity(n * arrays);
            for _ in 0..n {
                if skinned {
                    matrices.push(r.u8()?);
                }
                for _ in 0..arrays {
                    indices.push(r.u16()?);
                }
            }
            if r.pos() > end {
                return Err(r.error("a primitive runs past the end of the display list"));
            }
            out.push(Command::Draw { primitive: cmd, matrices, indices });
        } else {
            return Err(r.error(format_args!("unknown display list command 0x{cmd:02x}")));
        }
        content = r.pos() - start;
    }
    Ok((out, content))
}

/// Writes a mesh and its data; returns where its rig offset goes, if skinned.
fn write_mesh(w: &mut Writer, mesh: &Mesh) -> Option<usize> {
    let info = w.pos();
    let dl_size_at = w.reserve_u32();
    let dl_at = w.reserve_u32();
    w.u32(u32_len(mesh.arrays.len()));
    let table_at = w.reserve_u32();
    w.u64(mesh.material.instance);
    w.u32(mesh.material.type_id);
    w.u32(mesh.material.group);
    w.vec3(mesh.bounds_min);
    w.vec3(mesh.bounds_max);
    mesh.unknown0.iter().for_each(|v| w.u32(*v));
    w.u32(mesh.name);
    w.zeros(12);
    let skin_at = w.reserve_u32();
    let bones_at = w.reserve_u32();
    let rig_at = w.reserve_u32();
    debug_assert_eq!(w.pos() - info, MESH_INFO_SIZE);
    w.align(32);
    w.zeros(32);

    let dl_start = w.pos();
    w.patch_u32(dl_at, u32_len(dl_start));
    let skinned = mesh.skin.is_some();
    for cmd in &mesh.display_list {
        match cmd {
            Command::Nop => w.u8(op::NOP),
            Command::Load { command, index, descriptor } => {
                w.u8(*command);
                w.u16(*index);
                w.u16(*descriptor);
            }
            Command::Draw { primitive, matrices, indices } => {
                let per = mesh.arrays.len().max(1);
                let n = indices.len() / per;
                w.u8(*primitive);
                w.u16(n as u16);
                for (v, idx) in indices.chunks_exact(per).enumerate() {
                    if skinned {
                        w.u8(matrices.get(v).copied().unwrap_or(0));
                    }
                    idx.iter().for_each(|i| w.u16(*i));
                }
            }
        }
    }
    w.align(32);
    if w.pos() - dl_start < mesh.display_list_capacity {
        w.zeros(mesh.display_list_capacity - (w.pos() - dl_start));
        w.align(32);
    }
    w.patch_u32(dl_size_at, u32_len(w.pos() - dl_start));

    w.patch_u32(table_at, u32_len(w.pos()));
    let slots: Vec<usize> = mesh
        .arrays
        .iter()
        .map(|a| {
            w.u8(a.attribute);
            w.u8(a.stride);
            w.zeros(2);
            w.reserve_u32()
        })
        .collect();
    for (a, slot) in mesh.arrays.iter().zip(slots) {
        w.align(32);
        w.patch_u32(slot, u32_len(w.pos()));
        w.bytes(&a.data);
    }

    let skin = mesh.skin.as_ref()?;
    let at = w.pos();
    w.patch_u32(skin_at, u32_len(at));
    let n = skin.palette.len();
    w.u32(u32_len(at + 12));
    w.u32(u32_len(at + 12 + 20 * n));
    w.u32(u32_len(n));
    for p in &skin.palette {
        p.weights.iter().for_each(|v| w.f32(*v));
        w.bytes(&p.bones);
    }
    // Room the game uses for the blended matrices, as the exporter left it.
    for _ in 0..n {
        for _ in 0..3 {
            [1.0, 0.0, 0.0, 0.0].iter().for_each(|v| w.f32(*v));
        }
    }
    w.patch_u32(bones_at, u32_len(w.pos()));
    w.u32(u32_len(skin.bones.len()));
    skin.bones.iter().for_each(|h| w.u32(*h));
    Some(rig_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::types;

    fn floats(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_be_bytes()).collect()
    }

    fn sample(skinned: bool) -> RevoModel {
        let arrays = vec![
            VertexArray {
                attribute: attr::POSITION,
                stride: 12,
                data: floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0]),
            },
            VertexArray { attribute: attr::NORMAL, stride: 12, data: floats(&[0.0, 0.0, 1.0]) },
            VertexArray { attribute: attr::TEX0, stride: 8, data: floats(&[0.0, 0.0, 1.0, 1.0]) },
        ];
        let mut display_list = Vec::new();
        let matrices = if skinned {
            display_list.push(Command::load_position_matrix(0, 0));
            display_list.push(Command::load_normal_matrix(0, 0));
            vec![0; 4]
        } else {
            vec![]
        };
        display_list.push(Command::Draw {
            primitive: op::TRIANGLE_STRIP,
            matrices,
            indices: vec![0, 0, 0, 1, 0, 1, 2, 0, 0, 3, 0, 1],
        });
        RevoModel {
            version: RevoModel::VERSION,
            bounds: Bounds::EMPTY,
            extras: vec![ExtraParam { key: 0x9281_771a, value: b"no_shadow".to_vec() }],
            rigs: if skinned {
                vec![Rig { bone_hashes: vec![0x7bd0_5f71], inverse_binds: vec![[1.0; 16]] }]
            } else {
                vec![]
            },
            meshes: vec![Mesh {
                material: ResourceKey { instance: 0x6029_33d7_f1c0_77b9, type_id: types::MATERIAL, group: 0 },
                bounds_min: [0.0; 3],
                bounds_max: [1.0, 1.0, 0.0],
                unknown0: [0, 0],
                name: 0x2ca3_3bdb,
                arrays,
                display_list,
                display_list_capacity: 0,
                skin: skinned.then(|| Skin {
                    palette: vec![PaletteEntry {
                        weights: [1.0, 0.0, 0.0, 0.0],
                        bones: [0, 0xff, 0xff, 0xff],
                    }],
                    bones: vec![0x7bd0_5f71],
                    rig: 0,
                }),
            }],
        }
    }

    #[test]
    fn round_trip() {
        for skinned in [false, true] {
            let model = sample(skinned);
            let bytes = model.write(Game::Kingdom);
            assert_eq!(bytes.len() % 32, 0);
            let back = RevoModel::read(&bytes).unwrap();
            assert_eq!(back, model);
            assert_eq!(back.write(Game::Kingdom), bytes);
        }
    }

    #[test]
    fn layout_matches_the_exporter() {
        let bytes = sample(false).write(Game::Kingdom);
        let u32_at = |o: usize| u32::from_be_bytes(bytes[o..o + 4].try_into().unwrap());
        // One mesh, its info right after the table, its display list 32 bytes
        // past the next 32-byte boundary, its arrays on 32-byte boundaries.
        assert_eq!(u32_at(0x24), 0x58);
        assert_eq!(u32_at(0x58), 0x5c);
        assert_eq!(u32_at(0x5c + 4), 0xe0);
        let table = u32_at(0x5c + 12) as usize;
        for a in 0..3 {
            assert_eq!(u32_at(table + 8 * a + 4) % 32, 0);
        }
    }
}
