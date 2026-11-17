//! A format-neutral model both formats convert to and from.
//!
//! RevoModel stores each attribute in its own array and draws with index
//! tuples; WindowsModel stores interleaved vertices and a triangle list.
//! [`Model`] holds the WindowsModel shape: one vertex per distinct index tuple,
//! with skin weights per vertex (RevoModel keeps them per matrix palette entry).

mod revo;
mod windows;

use std::collections::HashMap;

use crate::game::{Game, ResourceKey};
use crate::model::{Bounds, ExtraParam, Rig};

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub extras: Vec<ExtraParam>,
    pub rigs: Vec<Rig>,
    pub meshes: Vec<Mesh>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub material: ResourceKey,
    pub bounds: Bounds,
    pub unknown0: [u32; 2],
    pub name: u32,
    pub vertices: Vertices,
    /// Wound as both formats draw them.
    pub triangles: Vec<[u32; 3]>,
    /// The rig skinned vertices index into.
    pub rig: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Vertices {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    /// RGBA, gamma-encoded as stored.
    pub colors: Option<Vec<[u8; 4]>>,
    /// Texture coordinates by UV set (0 diffuse, 1 lightmap, 2 water or
    /// grid, 3 more in Kingdom), in set order.
    pub uvs: Vec<(u8, Vec<[f32; 2]>)>,
    pub skin: Option<Vec<VertexSkin>>,
}

impl Vertices {
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// Up to four bones (indices into the mesh's rig) and their weights. Unused
/// slots have a weight of 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VertexSkin {
    pub bones: [u8; 4],
    pub weights: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Material instances to replace, applied after the type ids are mapped.
    /// MySims needs this between its releases: the Wii one names materials
    /// with FNV64, the PC one with FNV32 (see [`material_name_map`]).
    pub material_instances: HashMap<u64, u64>,
    /// Leave meshes without normals as they are instead of computing smooth
    /// ones. The Wii games draw some meshes unlit and store none.
    pub keep_missing_normals: bool,
}

/// Things worth telling the user that did not stop the conversion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub notes: Vec<String>,
}

impl Report {
    fn note(&mut self, s: impl Into<String>) {
        let s = s.into();
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConvertError(pub String);

pub type Result<T> = std::result::Result<T, ConvertError>;

fn err<T>(s: impl Into<String>) -> Result<T> {
    Err(ConvertError(s.into()))
}

/// Material instance replacements for converting a MySims model from
/// `from` to `to` (one RevoModel, one WindowsModel), built from material
/// names. Empty unless exactly one side is the PC release of MySims.
pub fn material_name_map<'a>(
    names: impl IntoIterator<Item = &'a str>,
    from: (Game, Format),
    to: (Game, Format),
) -> HashMap<u64, u64> {
    let width = |(game, format): (Game, Format)| {
        if format == Format::Windows && game.windows_material_hash_is_32() { 32 } else { 64 }
    };
    let hash = |name: &str, bits| {
        if bits == 32 { u64::from(crate::hash::fnv32(name)) } else { crate::hash::fnv64(name) }
    };
    let (a, b) = (width(from), width(to));
    if a == b {
        return HashMap::new();
    }
    names.into_iter().map(|n| (hash(n, a), hash(n, b))).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Revo,
    Windows,
}

impl Model {
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::EMPTY;
        for m in &self.meshes {
            b.union(&m.bounds);
        }
        b
    }

    /// Maps material type ids to `game`'s and applies the instance map.
    fn map_materials(&mut self, game: Game, options: &Options, report: &mut Report) {
        let mut unmapped = 0;
        for mesh in &mut self.meshes {
            let key = &mut mesh.material;
            if let Some(set) = Game::is_material_set(key.type_id) {
                key.type_id = game.material_type(set);
            }
            match options.material_instances.get(&key.instance) {
                Some(&to) => key.instance = to,
                None if !options.material_instances.is_empty() => unmapped += 1,
                None => {}
            }
        }
        if unmapped > 0 {
            report.note(format!("{unmapped} material reference(s) had no name to rehash; kept as they were"));
        }
    }

    /// Computes area-weighted smooth normals for meshes that have none.
    fn fill_normals(&mut self, report: &mut Report) {
        for (i, mesh) in self.meshes.iter_mut().enumerate() {
            if mesh.vertices.normals.is_some() {
                continue;
            }
            mesh.vertices.normals = Some(smooth_normals(&mesh.vertices.positions, &mesh.triangles));
            report.note(format!("mesh {i} had no normals; computed smooth ones"));
        }
    }
}

pub fn smooth_normals(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    // Vertices at the same place share a normal, so split UV seams stay smooth.
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let mut sums: HashMap<[u32; 3], [f32; 3]> = HashMap::new();
    for t in triangles {
        let [a, b, c] = t.map(|i| positions[i as usize]);
        let (u, v) = (sub(b, a), sub(c, a));
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for p in [a, b, c] {
            let s = sums.entry(key(p)).or_default();
            (0..3).for_each(|k| s[k] += n[k]);
        }
    }
    positions
        .iter()
        .map(|&p| {
            let n = sums.get(&key(p)).copied().unwrap_or([0.0, 1.0, 0.0]);
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len > 0.0 { n.map(|c| c / len) } else { [0.0, 1.0, 0.0] }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_map_only_for_mysims_pc() {
        let names = ["glass"];
        let wii = (Game::MySims, Format::Revo);
        let pc = (Game::MySims, Format::Windows);
        let map = material_name_map(names, wii, pc);
        assert_eq!(map[&crate::hash::fnv64("glass")], u64::from(crate::hash::fnv32("glass")));
        let back = material_name_map(names, pc, wii);
        assert_eq!(back[&u64::from(crate::hash::fnv32("glass"))], crate::hash::fnv64("glass"));
        assert!(
            material_name_map(names, (Game::Kingdom, Format::Revo), (Game::Kingdom, Format::Windows))
                .is_empty()
        );
        assert!(
            material_name_map(names, (Game::Agents, Format::Revo), (Game::Kingdom, Format::Windows))
                .is_empty()
        );
    }

    #[test]
    fn normals_of_a_flat_quad() {
        let p = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        let n = smooth_normals(&p, &[[0, 1, 2], [2, 1, 3]]);
        for v in n {
            assert!((v[2] - 1.0).abs() < 1e-6, "{v:?}");
        }
    }
}
