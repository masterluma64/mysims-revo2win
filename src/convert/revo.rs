//! RevoModel to and from [`Model`].

use std::collections::HashMap;

use super::{Mesh, Model, Options, Report, Result, VertexSkin, Vertices, err};
use crate::game::Game;
use crate::model::Bounds;
use crate::rmdl::{self, Command, MATRIX_SLOTS, PaletteEntry, RevoModel, Skin, VertexArray, attr, op};
use crate::strip;

/// The most vertices one GX primitive can draw.
const MAX_PRIMITIVE_VERTICES: usize = u16::MAX as usize;

impl Model {
    pub fn from_revo(revo: &RevoModel, report: &mut Report) -> Result<Model> {
        let meshes = revo
            .meshes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                mesh_from_revo(m, revo, report).map_err(|e| super::ConvertError(format!("mesh {i}: {}", e.0)))
            })
            .collect::<Result<_>>()?;
        Ok(Model { extras: revo.extras.clone(), rigs: revo.rigs.clone(), meshes })
    }

    /// A RevoModel for `game`'s Wii release.
    pub fn to_revo(&self, game: Game, options: &Options, report: &mut Report) -> Result<RevoModel> {
        let mut model = self.clone();
        model.map_materials(game, options, report);
        if !options.keep_missing_normals {
            model.fill_normals(report);
        }
        let meshes = model
            .meshes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                mesh_to_revo(m, &model).map_err(|e| super::ConvertError(format!("mesh {i}: {}", e.0)))
            })
            .collect::<Result<_>>()?;
        Ok(RevoModel {
            version: RevoModel::VERSION,
            bounds: Bounds::EMPTY,
            extras: model.extras.clone(),
            rigs: model.rigs.clone(),
            meshes,
        })
    }
}

fn check_stride(a: &VertexArray, want: u8, what: &str) -> Result<()> {
    if a.stride == want {
        Ok(())
    } else {
        err(format!("{what} array has {}-byte elements (only {want} is supported)", a.stride))
    }
}

fn mesh_from_revo(m: &rmdl::Mesh, revo: &RevoModel, report: &mut Report) -> Result<Mesh> {
    let Some(pos) = m.array(attr::POSITION) else {
        return err("no position array");
    };
    check_stride(pos, 12, "position")?;
    let nrm = m.array(attr::NORMAL);
    if let Some(a) = nrm {
        check_stride(a, 12, "normal")?;
    }
    let clr = m.array(attr::COLOR0);
    if let Some(a) = clr {
        check_stride(a, 4, "colour")?;
    }
    if m.array(attr::COLOR1).is_some() {
        report.note("a second vertex colour was dropped (WindowsModel has one)");
    }
    let mut uv_arrays = Vec::new();
    for a in &m.arrays {
        if let Some(set) = attr::uv_set(a.attribute) {
            check_stride(a, 8, "texture coordinate")?;
            uv_arrays.push((set, a));
        }
    }
    let column = |attribute: u8| m.arrays.iter().position(|a| a.attribute == attribute);
    let pos_col = column(attr::POSITION).expect("checked above");
    let nrm_col = column(attr::NORMAL);
    let clr_col = column(attr::COLOR0);
    let uv_cols: Vec<usize> = uv_arrays.iter().map(|(set, _)| column(attr::TEX0 + set).unwrap()).collect();

    let rig = match &m.skin {
        Some(skin) => match revo.rigs.get(skin.rig) {
            Some(r) => Some((skin, r)),
            None => return err(format!("rig {} does not exist", skin.rig)),
        },
        None => None,
    };

    let per = m.arrays.len();
    let mut vertices = Vertices {
        normals: nrm.map(|_| Vec::new()),
        colors: clr.map(|_| Vec::new()),
        uvs: uv_arrays.iter().map(|(set, _)| (*set, Vec::new())).collect(),
        skin: rig.map(|_| Vec::new()),
        ..Default::default()
    };
    // One vertex per distinct (palette entry, indices) tuple.
    let mut lookup: HashMap<(u16, &[u16]), u32> = HashMap::new();
    let mut slots = [None::<u16>; MATRIX_SLOTS];
    let mut triangles = Vec::new();
    let mut skipped_primitives = 0;

    for cmd in &m.display_list {
        let (primitive, matrices, indices) = match cmd {
            Command::Nop => continue,
            Command::Load { .. } => {
                if let Some((index, slot)) = cmd.position_slot()
                    && slot < MATRIX_SLOTS
                {
                    slots[slot] = Some(index);
                }
                continue;
            }
            Command::Draw { primitive, matrices, indices } => (*primitive & 0xf8, matrices, indices),
        };
        let mut ids = Vec::with_capacity(indices.len() / per.max(1));
        for (v, idx) in indices.chunks_exact(per.max(1)).enumerate() {
            let palette = match rig {
                Some(_) => {
                    let raw = matrices.get(v).copied().unwrap_or(0);
                    let slot = usize::from(raw / 3);
                    match slots.get(slot).copied().flatten() {
                        Some(p) => p,
                        None => return err(format!("a vertex uses matrix slot {slot} before it is loaded")),
                    }
                }
                None => u16::MAX,
            };
            let next = vertices.positions.len() as u32;
            let id = *lookup.entry((palette, idx)).or_insert(next);
            if id == next {
                push_vertex(&mut vertices, m, idx, pos_col, nrm_col, clr_col, &uv_cols, rig, palette)?;
            }
            ids.push(id);
        }
        match primitive {
            op::TRIANGLES => triangles.extend(ids.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]])),
            op::TRIANGLE_STRIP => triangles.extend(strip::strip_triangles(&ids)),
            op::TRIANGLE_FAN => {
                triangles.extend((1..ids.len().saturating_sub(1)).map(|i| [ids[0], ids[i], ids[i + 1]]))
            }
            op::QUADS => {
                for q in ids.as_chunks::<4>().0 {
                    triangles.push([q[0], q[1], q[2]]);
                    triangles.push([q[0], q[2], q[3]]);
                }
            }
            _ => skipped_primitives += 1,
        }
    }
    if skipped_primitives > 0 {
        report.note("lines and points in a display list were dropped");
    }
    triangles.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    if vertices.len() > usize::from(u16::MAX) + 1 {
        return err(format!("{} vertices; a WindowsModel mesh holds at most 65536", vertices.len()));
    }

    Ok(Mesh {
        material: m.material,
        bounds: Bounds { min: m.bounds_min, max: m.bounds_max },
        unknown0: m.unknown0,
        name: m.name,
        vertices,
        triangles,
        rig: m.skin.as_ref().map(|s| s.rig),
    })
}

#[allow(clippy::too_many_arguments)]
fn push_vertex(
    out: &mut Vertices,
    m: &rmdl::Mesh,
    idx: &[u16],
    pos_col: usize,
    nrm_col: Option<usize>,
    clr_col: Option<usize>,
    uv_cols: &[usize],
    rig: Option<(&Skin, &crate::model::Rig)>,
    palette: u16,
) -> Result<()> {
    let float3 = |col: usize| -> Result<[f32; 3]> {
        let a = &m.arrays[col];
        let i = usize::from(idx[col]);
        match (a.f32(i, 0), a.f32(i, 1), a.f32(i, 2)) {
            (Some(x), Some(y), Some(z)) => Ok([x, y, z]),
            _ => err(format!("index {i} is past the end of attribute {}", a.attribute)),
        }
    };
    out.positions.push(float3(pos_col)?);
    if let (Some(col), Some(n)) = (nrm_col, out.normals.as_mut()) {
        n.push(float3(col)?);
    }
    if let (Some(col), Some(c)) = (clr_col, out.colors.as_mut()) {
        let e = m.arrays[col].element(usize::from(idx[col])).unwrap_or(&[255; 4]);
        c.push([e[0], e[1], e[2], e[3]]);
    }
    for (&col, (_, uv)) in uv_cols.iter().zip(out.uvs.iter_mut()) {
        let a = &m.arrays[col];
        let i = usize::from(idx[col]);
        uv.push([a.f32(i, 0).unwrap_or(0.0), a.f32(i, 1).unwrap_or(0.0)]);
    }
    if let (Some((skin, rig)), Some(s)) = (rig, out.skin.as_mut()) {
        let Some(entry) = skin.palette.get(usize::from(palette)) else {
            return err(format!("matrix {palette} is not in the palette"));
        };
        s.push(skin_from_palette(entry, &skin.bones, &rig.bone_hashes)?);
    }
    Ok(())
}

/// A palette entry's bones as indices into the rig, heaviest first.
fn skin_from_palette(entry: &PaletteEntry, mesh_bones: &[u32], rig_bones: &[u32]) -> Result<VertexSkin> {
    let mut pairs = Vec::with_capacity(4);
    for (&b, &w) in entry.bones.iter().zip(&entry.weights) {
        if b == PaletteEntry::NO_BONE {
            continue;
        }
        let Some(&hash) = mesh_bones.get(usize::from(b)) else {
            return err(format!("palette bone {b} is not in the mesh's bone list"));
        };
        let Some(index) = rig_bones.iter().position(|&h| h == hash) else {
            return err(format!("bone 0x{hash:08x} is not in the rig"));
        };
        let Ok(index) = u8::try_from(index) else {
            return err("a rig with more than 255 bones");
        };
        pairs.push((index, w));
    }
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut skin = VertexSkin { bones: [0; 4], weights: [0.0; 4] };
    for (k, (b, w)) in pairs.into_iter().enumerate() {
        skin.bones[k] = b;
        skin.weights[k] = w;
    }
    Ok(skin)
}

/// Collects distinct values into an array, returning each input's index.
struct ArrayBuilder {
    attribute: u8,
    stride: u8,
    data: Vec<u8>,
    seen: HashMap<Vec<u8>, u16>,
}

impl ArrayBuilder {
    fn new(attribute: u8, stride: u8) -> Self {
        Self { attribute, stride, data: Vec::new(), seen: HashMap::new() }
    }

    fn add(&mut self, bytes: Vec<u8>) -> Result<u16> {
        if let Some(&i) = self.seen.get(&bytes) {
            return Ok(i);
        }
        let i = self.seen.len();
        let Ok(i) = u16::try_from(i) else {
            return err(format!("more than 65536 distinct values for attribute {}", self.attribute));
        };
        self.data.extend_from_slice(&bytes);
        self.seen.insert(bytes, i);
        Ok(i)
    }

    fn finish(self) -> VertexArray {
        VertexArray { attribute: self.attribute, stride: self.stride, data: self.data }
    }
}

fn be_floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_be_bytes()).collect()
}

fn mesh_to_revo(m: &Mesh, model: &Model) -> Result<rmdl::Mesh> {
    let v = &m.vertices;
    let n = v.len();
    let mut builders = vec![ArrayBuilder::new(attr::POSITION, 12)];
    if v.normals.is_some() {
        builders.push(ArrayBuilder::new(attr::NORMAL, 12));
    }
    if v.colors.is_some() {
        builders.push(ArrayBuilder::new(attr::COLOR0, 4));
    }
    for (set, _) in &v.uvs {
        if *set > 7 {
            return err(format!("UV set {set} (GX has 8)"));
        }
        builders.push(ArrayBuilder::new(attr::TEX0 + set, 8));
    }
    builders.sort_by_key(|b| b.attribute);

    // Each vertex's index into every array, in array order.
    let per = builders.len();
    let mut indices = vec![0u16; n * per];
    for (col, b) in builders.iter_mut().enumerate() {
        for i in 0..n {
            let bytes = match b.attribute {
                attr::POSITION => be_floats(&v.positions[i]),
                attr::NORMAL => be_floats(&v.normals.as_ref().unwrap()[i]),
                attr::COLOR0 => v.colors.as_ref().unwrap()[i].to_vec(),
                a => {
                    let set = attr::uv_set(a).unwrap();
                    let uvs = &v.uvs.iter().find(|(s, _)| *s == set).unwrap().1;
                    be_floats(&uvs[i])
                }
            };
            indices[i * per + col] = b.add(bytes)?;
        }
    }

    let skin_data = match (&v.skin, m.rig) {
        (Some(skin), Some(rig)) => match model.rigs.get(rig) {
            Some(r) => Some(build_palette(skin, &r.bone_hashes, rig)?),
            None => return err(format!("rig {rig} does not exist")),
        },
        _ => None,
    };

    let mut display_list = Vec::new();
    let draw = |dl: &mut Vec<Command>, strip_ids: &[u32], matrix_of: &dyn Fn(u32) -> u8| {
        for chunk in strip_chunks(strip_ids) {
            let mut idx = Vec::with_capacity(chunk.len() * per);
            for &id in chunk {
                idx.extend_from_slice(&indices[id as usize * per..id as usize * per + per]);
            }
            let matrices = match skin_data {
                Some(_) => chunk.iter().map(|&id| matrix_of(id)).collect(),
                None => Vec::new(),
            };
            dl.push(Command::Draw { primitive: op::TRIANGLE_STRIP, matrices, indices: idx });
        }
    };

    match &skin_data {
        None => {
            for s in strip::stripify(&m.triangles) {
                draw(&mut display_list, &s, &|_| 0);
            }
        }
        Some((_, vertex_palette)) => {
            for batch in matrix_batches(&m.triangles, vertex_palette) {
                for (slot, &p) in batch.palette.iter().enumerate() {
                    display_list.push(Command::load_position_matrix(p, slot));
                    display_list.push(Command::load_normal_matrix(p, slot));
                }
                display_list.push(Command::Nop);
                let slot_of: HashMap<u16, u8> =
                    batch.palette.iter().enumerate().map(|(s, &p)| (p, s as u8 * 3)).collect();
                let matrix_of = |id: u32| slot_of[&vertex_palette[id as usize]];
                for s in strip::stripify(&batch.triangles) {
                    draw(&mut display_list, &s, &matrix_of);
                }
            }
        }
    }

    Ok(rmdl::Mesh {
        material: m.material,
        bounds_min: m.bounds.min,
        bounds_max: m.bounds.max,
        unknown0: m.unknown0,
        name: m.name,
        arrays: builders.into_iter().map(ArrayBuilder::finish).collect(),
        display_list,
        display_list_capacity: 0,
        skin: skin_data.map(|(skin, _)| skin),
    })
}

/// Splits a strip that is too long for one primitive, overlapping by two
/// vertices and keeping the winding (each piece starts on an even triangle).
fn strip_chunks(s: &[u32]) -> Vec<&[u32]> {
    if s.len() <= MAX_PRIMITIVE_VERTICES {
        return vec![s];
    }
    let step = (MAX_PRIMITIVE_VERTICES - 2) & !1;
    let mut out = Vec::new();
    let mut start = 0;
    while start + 2 < s.len() {
        out.push(&s[start..(start + step + 2).min(s.len())]);
        start += step;
    }
    out
}

/// The mesh's skin as a matrix palette: one entry per distinct set of bones
/// and weights. Returns it with each vertex's palette index.
fn build_palette(skin: &[VertexSkin], rig_bones: &[u32], rig: usize) -> Result<(Skin, Vec<u16>)> {
    // The bones the mesh uses, in rig order.
    let mut used = vec![false; rig_bones.len()];
    for s in skin {
        for (b, w) in s.bones.iter().zip(&s.weights) {
            if *w != 0.0 {
                match used.get_mut(usize::from(*b)) {
                    Some(u) => *u = true,
                    None => return err(format!("bone {b} is not in the rig")),
                }
            }
        }
    }
    let local: Vec<usize> = (0..rig_bones.len()).filter(|&b| used[b]).collect();
    if local.len() > usize::from(PaletteEntry::NO_BONE) {
        return err("a mesh uses more than 254 bones");
    }
    let to_local: HashMap<usize, u8> = local.iter().enumerate().map(|(l, &b)| (b, l as u8)).collect();

    let mut palette = Vec::new();
    let mut seen: HashMap<([u8; 4], [u32; 4]), u16> = HashMap::new();
    let mut vertex_palette = Vec::with_capacity(skin.len());
    for s in skin {
        let mut entry = PaletteEntry { weights: [0.0; 4], bones: [PaletteEntry::NO_BONE; 4] };
        let mut k = 0;
        for (b, w) in s.bones.iter().zip(&s.weights) {
            if *w != 0.0 {
                entry.bones[k] = to_local[&usize::from(*b)];
                entry.weights[k] = *w;
                k += 1;
            }
        }
        let key = (entry.bones, entry.weights.map(f32::to_bits));
        let next = palette.len();
        let index = *seen.entry(key).or_insert_with(|| {
            palette.push(entry);
            next as u16
        });
        if palette.len() > usize::from(u16::MAX) {
            return err("more than 65535 distinct bone weightings");
        }
        vertex_palette.push(index);
    }
    let bones = local.iter().map(|&b| rig_bones[b]).collect();
    Ok((Skin { palette, bones, rig }, vertex_palette))
}

struct Batch {
    palette: Vec<u16>,
    triangles: Vec<[u32; 3]>,
}

/// Groups triangles so each group needs at most [`MATRIX_SLOTS`] palette
/// entries, the number GX can index at once.
fn matrix_batches(triangles: &[[u32; 3]], vertex_palette: &[u16]) -> Vec<Batch> {
    let mut out: Vec<Batch> = Vec::new();
    let mut current = Batch { palette: Vec::new(), triangles: Vec::new() };
    for t in triangles {
        let needed: Vec<u16> = {
            let mut p: Vec<u16> = t.iter().map(|&v| vertex_palette[v as usize]).collect();
            p.sort_unstable();
            p.dedup();
            p
        };
        let new = needed.iter().filter(|p| !current.palette.contains(p)).count();
        if current.palette.len() + new > MATRIX_SLOTS {
            out.push(std::mem::replace(&mut current, Batch { palette: Vec::new(), triangles: Vec::new() }));
        }
        for p in needed {
            if !current.palette.contains(&p) {
                current.palette.push(p);
            }
        }
        current.triangles.push(*t);
    }
    if !current.triangles.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_strips_split_on_even_triangles() {
        let s: Vec<u32> = (0..200_000).collect();
        let chunks = strip_chunks(&s);
        assert!(chunks.iter().all(|c| c.len() <= MAX_PRIMITIVE_VERTICES));
        let whole: Vec<[u32; 3]> = strip::strip_triangles(&s).collect();
        let parts: Vec<[u32; 3]> = chunks.iter().flat_map(|c| strip::strip_triangles(c)).collect();
        assert_eq!(whole, parts);
    }

    #[test]
    fn batches_fit_the_matrix_slots() {
        // Twelve triangles, each on its own palette entry.
        let tris: Vec<[u32; 3]> = (0..12).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect();
        let palette: Vec<u16> = (0..36).map(|v| (v / 3) as u16).collect();
        let batches = matrix_batches(&tris, &palette);
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|b| b.palette.len() <= MATRIX_SLOTS));
        assert_eq!(batches.iter().map(|b| b.triangles.len()).sum::<usize>(), 12);
    }
}
