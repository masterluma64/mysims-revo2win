//! WindowsModel to and from [`Model`].

use super::{Mesh, Model, Options, Report, Result, VertexSkin, Vertices, err};
use crate::game::Game;
use crate::model::{Bounds, join_extra_values, split_extra_values};
use crate::wmdl::{self, ElementType, VertexKey, WindowsModel, usage};

impl Model {
    pub fn from_windows(win: &WindowsModel, report: &mut Report) -> Result<Model> {
        let extras = match &win.extra_values {
            Some(block) => match split_extra_values(&win.extra_keys, block) {
                Some(e) => e,
                None => return err("the extra parameter values are not one string per key"),
            },
            None if win.extra_keys.is_empty() => Vec::new(),
            None => return err("extra parameter keys without values"),
        };
        let meshes = win
            .meshes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                mesh_from_windows(m, win, report)
                    .map_err(|e| super::ConvertError(format!("mesh {i}: {}", e.0)))
            })
            .collect::<Result<_>>()?;
        Ok(Model { extras, rigs: win.rigs.clone(), meshes })
    }

    /// A WindowsModel for `game`'s Cozy Bundle release. MySims stores three
    /// skin weights per vertex, Kingdom four.
    pub fn to_windows(&self, game: Game, options: &Options, report: &mut Report) -> Result<WindowsModel> {
        if !game.has_windows_release() {
            return err(format!("{game} has no PC release to write a WindowsModel for"));
        }
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
                mesh_to_windows(m, game, report)
                    .map_err(|e| super::ConvertError(format!("mesh {i}: {}", e.0)))
            })
            .collect::<Result<_>>()?;
        let bounds =
            if model.meshes.is_empty() { Bounds { min: [0.0; 3], max: [0.0; 3] } } else { model.bounds() };
        Ok(WindowsModel {
            minor_version: WindowsModel::MINOR_VERSION,
            major_version: WindowsModel::MAJOR_VERSION,
            header_pad: [0; 2],
            bounds_min: bounds.min,
            bounds_max: bounds.max,
            extra_keys: model.extras.iter().map(|e| e.key).collect(),
            extra_values: (!model.extras.is_empty()).then(|| join_extra_values(&model.extras)),
            extra_pad: 0,
            rigs: model.rigs.clone(),
            meshes,
        })
    }
}

fn mesh_from_windows(m: &wmdl::Mesh, win: &WindowsModel, report: &mut Report) -> Result<Mesh> {
    let stride = m.stride();
    let count = m.vertex_count as usize;
    let key = |want: u8, kinds: &[ElementType]| {
        m.keys.iter().find(|k| k.usage == want && kinds.contains(&k.kind)).copied()
    };
    for k in &m.keys {
        let size = k.kind.size();
        if size.is_none_or(|s| k.offset as usize + s > stride) {
            report.note(format!(
                "a vertex element (type {}, usage {}) did not fit the vertex and was dropped",
                k.kind.to_u8(),
                k.usage
            ));
        }
    }
    let fits = |k: &VertexKey| k.kind.size().is_some_and(|s| k.offset as usize + s <= stride);
    let f32_at = |v: usize, offset: usize| {
        let at = v * stride + offset;
        f32::from_le_bytes(m.vertices[at..at + 4].try_into().unwrap())
    };
    let floats = |k: VertexKey, n: usize| -> Vec<[f32; 4]> {
        (0..count)
            .map(|v| {
                let mut out = [0.0; 4];
                for (c, o) in out.iter_mut().enumerate().take(n) {
                    *o = f32_at(v, k.offset as usize + 4 * c);
                }
                out
            })
            .collect()
    };

    let Some(pos) = key(usage::POSITION, &[ElementType::Float3]).filter(fits) else {
        return err("no position element");
    };
    let mut vertices = Vertices {
        positions: floats(pos, 3).into_iter().map(|p| [p[0], p[1], p[2]]).collect(),
        ..Default::default()
    };
    if let Some(k) = key(usage::NORMAL, &[ElementType::Float3]).filter(fits) {
        vertices.normals = Some(floats(k, 3).into_iter().map(|p| [p[0], p[1], p[2]]).collect());
    }
    if let Some(k) = key(usage::COLOR, &[ElementType::Color]).filter(fits) {
        vertices.colors = Some(
            (0..count)
                .map(|v| {
                    let b = &m.vertices[v * stride + k.offset as usize..][..4];
                    [b[3], b[2], b[1], b[0]]
                })
                .collect(),
        );
    }
    let mut uv_keys: Vec<VertexKey> = m
        .keys
        .iter()
        .filter(|k| k.usage == usage::UV && k.kind == ElementType::Float2 && fits(k))
        .copied()
        .collect();
    uv_keys.sort_by_key(|k| k.set);
    uv_keys.dedup_by_key(|k| k.set);
    for k in uv_keys {
        vertices.uvs.push((k.set, floats(k, 2).into_iter().map(|p| [p[0], p[1]]).collect()));
    }

    let rig = usize::try_from(m.rig_index).ok();
    if let Some(r) = rig {
        if r >= win.rigs.len() {
            return err(format!("rig {r} does not exist"));
        }
        let bones = key(usage::BONES, &[ElementType::Byte4]).filter(fits);
        let weights = key(usage::WEIGHTS, &[ElementType::Float3, ElementType::Float4]).filter(fits);
        let (Some(bones), Some(weights)) = (bones, weights) else {
            return err("skinned, but without bone indices and weights");
        };
        let stored = if weights.kind == ElementType::Float3 { 3 } else { 4 };
        let bone_count = win.rigs[r].bone_hashes.len();
        let w = floats(weights, stored);
        let mut skin = Vec::with_capacity(count);
        for (v, mut weights) in w.into_iter().enumerate() {
            if stored == 3 {
                weights[3] = (1.0 - weights[0] - weights[1] - weights[2]).max(0.0);
                // Float noise from the subtraction is not a fourth bone.
                if weights[3] < 1e-5 {
                    weights[3] = 0.0;
                }
            }
            let b = &m.vertices[v * stride + bones.offset as usize..][..4];
            let mut s = VertexSkin { bones: [b[0], b[1], b[2], b[3]], weights };
            for k in 0..4 {
                if s.weights[k] != 0.0 && usize::from(s.bones[k]) >= bone_count {
                    return err(format!("vertex {v} uses bone {} of a {bone_count}-bone rig", s.bones[k]));
                }
            }
            for k in 0..4 {
                if s.weights[k] == 0.0 {
                    s.bones[k] = 0;
                }
            }
            skin.push(s);
        }
        vertices.skin = Some(skin);
    }

    let mut triangles = Vec::with_capacity(m.indices.len() / 3);
    for t in m.indices.as_chunks::<3>().0 {
        let t = [u32::from(t[0]), u32::from(t[1]), u32::from(t[2])];
        if t.iter().any(|&i| i as usize >= count) {
            return err("a triangle indexes past the last vertex");
        }
        triangles.push(t);
    }

    Ok(Mesh {
        material: m.material,
        bounds: Bounds { min: m.bounds_min, max: m.bounds_max },
        unknown0: m.unknown0,
        name: m.name,
        vertices,
        triangles,
        rig,
    })
}

/// The vertex elements a game's own files use, in their order: position,
/// then weights and bones when skinned, normal, colour, UV sets.
fn layout(v: &Vertices, game: Game) -> (Vec<VertexKey>, usize) {
    let mut keys = Vec::new();
    let mut offset = 0u32;
    let mut add = |kind: ElementType, usage: u8, set: u8| {
        keys.push(VertexKey { offset, kind, usage, set });
        offset += kind.size().unwrap() as u32;
    };
    add(ElementType::Float3, usage::POSITION, 0);
    if v.skin.is_some() {
        let weights = if game == Game::MySims { ElementType::Float3 } else { ElementType::Float4 };
        add(weights, usage::WEIGHTS, 0);
        add(ElementType::Byte4, usage::BONES, 0);
    }
    if v.normals.is_some() {
        add(ElementType::Float3, usage::NORMAL, 0);
    }
    if v.colors.is_some() {
        add(ElementType::Color, usage::COLOR, 0);
    }
    for (set, _) in &v.uvs {
        add(ElementType::Float2, usage::UV, *set);
    }
    (keys, offset as usize)
}

fn mesh_to_windows(m: &Mesh, game: Game, report: &mut Report) -> Result<wmdl::Mesh> {
    let v = &m.vertices;
    let n = v.len();
    if n > usize::from(u16::MAX) + 1 {
        return err(format!("{n} vertices; a WindowsModel mesh holds at most 65536"));
    }
    let (keys, stride) = layout(v, game);
    let mut data = Vec::with_capacity(n * stride);
    let mut dropped_fourth = 0usize;
    for i in 0..n {
        for k in &keys {
            match (k.usage, k.kind) {
                (usage::POSITION, _) => v.positions[i].iter().for_each(|f| data.extend(f.to_le_bytes())),
                (usage::NORMAL, _) => {
                    v.normals.as_ref().unwrap()[i].iter().for_each(|f| data.extend(f.to_le_bytes()))
                }
                (usage::COLOR, _) => {
                    let [r, g, b, a] = v.colors.as_ref().unwrap()[i];
                    data.extend([a, b, g, r]);
                }
                (usage::UV, _) => {
                    let uvs = &v.uvs.iter().find(|(s, _)| *s == k.set).unwrap().1;
                    uvs[i].iter().for_each(|f| data.extend(f.to_le_bytes()));
                }
                (usage::WEIGHTS, kind) => {
                    let s = sorted(v.skin.as_ref().unwrap()[i]);
                    let stored = if kind == ElementType::Float3 { 3 } else { 4 };
                    if stored == 3 && s.weights[3] > 0.0 {
                        dropped_fourth += 1;
                    }
                    s.weights[..stored].iter().for_each(|f| data.extend(f.to_le_bytes()));
                }
                (usage::BONES, _) => data.extend(sorted(v.skin.as_ref().unwrap()[i]).bones),
                _ => unreachable!("layout only makes the elements above"),
            }
        }
    }
    if dropped_fourth > 0 {
        report.note(format!(
            "{dropped_fourth} vertices have four bones; MySims keeps three weights and gives the fourth \
             bone what is left"
        ));
    }
    let mut indices = Vec::with_capacity(m.triangles.len() * 3);
    for t in &m.triangles {
        indices.extend(t.iter().map(|&i| i as u16));
    }
    Ok(wmdl::Mesh {
        material: m.material,
        bounds_min: m.bounds.min,
        bounds_max: m.bounds.max,
        unknown0: m.unknown0,
        name: m.name,
        unknown1: WindowsModel::MESH_UNKNOWN1,
        unknown2: [0; 12],
        vertex_count: n as u32,
        keys,
        vertices: data,
        indices,
        rig_index: match (m.rig, &v.skin) {
            (Some(r), Some(_)) => r as i32,
            _ => -1,
        },
    })
}

/// Heaviest bone first; unused slots repeat the last used bone.
fn sorted(mut s: VertexSkin) -> VertexSkin {
    let mut order = [0usize, 1, 2, 3];
    order.sort_by(|&a, &b| s.weights[b].total_cmp(&s.weights[a]));
    let (bones, weights) = (s.bones, s.weights);
    for (k, &o) in order.iter().enumerate() {
        s.bones[k] = bones[o];
        s.weights[k] = weights[o];
    }
    for k in 1..4 {
        if s.weights[k] == 0.0 {
            s.bones[k] = s.bones[k - 1];
        }
    }
    s
}
