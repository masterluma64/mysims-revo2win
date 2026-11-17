//! Checks against the games' own files. Run with
//! `cargo test --release -- --ignored` after pointing the environment at them:
//!
//! - `REVO2WIN_RMDL_DIR`: a folder of RevoModel files (any depth), with
//!   `mysims`, `kingdom` and `agents` somewhere in the path of each so the
//!   game is known;
//! - `REVO2WIN_WMDL_DIR`: a folder of WindowsModel files (`*.0xb359c791`).

use std::path::{Path, PathBuf};

use revo2win::{Game, RevoModel, WindowsModel};

fn files(var: &str) -> Option<Vec<PathBuf>> {
    let root = std::env::var_os(var)?;
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Some(out)
}

fn game_of(path: &Path) -> Game {
    let p = path.to_string_lossy().to_ascii_lowercase();
    if p.contains("agents") || p.contains("msa") {
        Game::Agents
    } else if p.contains("kingdom") || p.contains("msk") {
        Game::Kingdom
    } else {
        Game::MySims
    }
}

#[test]
#[ignore]
fn revo_models_round_trip() {
    let Some(files) = files("REVO2WIN_RMDL_DIR") else {
        eprintln!("REVO2WIN_RMDL_DIR is not set");
        return;
    };
    let (mut same, mut differ, mut failed) = (0, Vec::new(), Vec::new());
    for path in &files {
        let data = std::fs::read(path).unwrap();
        if !RevoModel::is_revo_model(&data) {
            continue;
        }
        match RevoModel::read(&data) {
            Ok(model) => {
                // MySims files end with whatever padding the exporter left,
                // from none to the next 32 bytes; only that may differ.
                let out = model.write(game_of(path));
                let n = out.len().min(data.len());
                let tail_is_padding = |b: &[u8]| b[n..].iter().all(|&x| x == 0);
                if out[..n] == data[..n] && tail_is_padding(&out) && tail_is_padding(&data) {
                    same += 1;
                } else {
                    differ.push(path.clone());
                }
            }
            Err(e) => failed.push(format!("{}: {e}", path.display())),
        }
    }
    eprintln!("{same} re-written as they were, {} differ, {} failed", differ.len(), failed.len());
    for p in differ.iter().take(10) {
        eprintln!("differs: {}", p.display());
    }
    for f in failed.iter().take(10) {
        eprintln!("failed: {f}");
    }
    assert!(failed.is_empty());
    assert!(differ.len() * 1000 <= same, "more than 1 in 1000 files differ");
}

#[test]
#[ignore]
fn windows_models_round_trip() {
    let Some(files) = files("REVO2WIN_WMDL_DIR") else {
        eprintln!("REVO2WIN_WMDL_DIR is not set");
        return;
    };
    let mut n = 0;
    for path in files.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("0xb359c791"))) {
        let data = std::fs::read(path).unwrap();
        let model = WindowsModel::read(&data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(model.write(), data, "{}", path.display());
        n += 1;
    }
    eprintln!("{n} WindowsModels round-trip");
}

mod canon {
    use revo2win::convert::Model;
    use revo2win::game::Game;

    /// A vertex as plain numbers: position, normal, colour, UV sets and skin
    /// (bone hash and weight to 1/10000, heaviest first).
    pub type Vertex = Vec<i64>;

    #[derive(Debug, PartialEq)]
    pub struct Mesh {
        pub material: (u64, u32, Option<bool>),
        pub name: u32,
        pub unknown0: [u32; 2],
        pub bounds: [u32; 6],
        pub triangles: Vec<[Vertex; 3]>,
    }

    pub fn of(model: &Model, positions_only: bool) -> Vec<Mesh> {
        model
            .meshes
            .iter()
            .map(|m| {
                let v = &m.vertices;
                let vertex = |i: usize| -> Vertex {
                    let mut out: Vertex = v.positions[i].iter().map(|f| i64::from(f.to_bits())).collect();
                    if positions_only {
                        return out;
                    }
                    if let Some(n) = &v.normals {
                        out.extend(n[i].iter().map(|f| i64::from(f.to_bits())));
                    }
                    if let Some(c) = &v.colors {
                        out.push(i64::from(u32::from_be_bytes(c[i])));
                    }
                    for (set, uv) in &v.uvs {
                        out.push(-1 - i64::from(*set));
                        out.extend(uv[i].iter().map(|f| i64::from(f.to_bits())));
                    }
                    if let (Some(skin), Some(rig)) = (&v.skin, m.rig) {
                        let s = skin[i];
                        let mut pairs: Vec<(i64, i64)> = (0..4)
                            .filter(|&k| s.weights[k] > 1e-4)
                            .map(|k| {
                                let hash = model.rigs[rig].bone_hashes[usize::from(s.bones[k])];
                                (i64::from(hash), (f64::from(s.weights[k]) * 10000.0).round() as i64)
                            })
                            .collect();
                        pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                        out.push(-100);
                        for (h, w) in pairs {
                            out.extend([h, w]);
                        }
                    }
                    out
                };
                let mut triangles: Vec<[Vertex; 3]> = m
                    .triangles
                    .iter()
                    .map(|t| {
                        let t = t.map(|i| vertex(i as usize));
                        let r = (0..3).min_by(|&a, &b| t[a].cmp(&t[b])).unwrap();
                        [t[r].clone(), t[(r + 1) % 3].clone(), t[(r + 2) % 3].clone()]
                    })
                    // Triangles that collapse to a line or point draw nothing.
                    .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
                    .collect();
                triangles.sort();
                Mesh {
                    material: (
                        m.material.instance,
                        m.material.group,
                        Game::is_material_set(m.material.type_id),
                    ),
                    name: m.name,
                    unknown0: m.unknown0,
                    bounds: [m.bounds.min, m.bounds.max]
                        .concat()
                        .iter()
                        .map(|f| f.to_bits())
                        .collect::<Vec<_>>()[..]
                        .try_into()
                        .unwrap(),
                    triangles,
                }
            })
            .collect()
    }
}

fn keep_normals() -> revo2win::Options {
    revo2win::Options { keep_missing_normals: true, ..Default::default() }
}

#[test]
#[ignore]
fn revo_to_windows_and_back_keeps_everything() {
    use revo2win::convert::{Model, Report};
    let Some(files) = files("REVO2WIN_RMDL_DIR") else {
        eprintln!("REVO2WIN_RMDL_DIR is not set");
        return;
    };
    let (mut n, mut bad) = (0, Vec::new());
    for path in &files {
        let data = std::fs::read(path).unwrap();
        if !RevoModel::is_revo_model(&data) {
            continue;
        }
        let game = game_of(path);
        let pc = if game == Game::Agents { Game::Kingdom } else { game };
        let r = &mut Report::default();
        let m0 = Model::from_revo(&RevoModel::read(&data).unwrap(), r).unwrap();
        let win = m0.to_windows(pc, &keep_normals(), r).unwrap().write();
        let m1 = Model::from_windows(&WindowsModel::read(&win).unwrap(), r).unwrap();
        let revo = m1.to_revo(game, &keep_normals(), r).unwrap().write(game);
        let m2 = Model::from_revo(&RevoModel::read(&revo).unwrap(), r).unwrap();
        let c0 = canon::of(&m0, false);
        if canon::of(&m1, false) != c0 || canon::of(&m2, false) != c0 {
            bad.push(path.clone());
        }
        n += 1;
    }
    eprintln!("{n} RevoModels through WindowsModel and back, {} changed", bad.len());
    for p in bad.iter().take(10) {
        eprintln!("changed: {}", p.display());
    }
    assert!(bad.is_empty());
}

#[test]
#[ignore]
fn windows_to_revo_and_back_keeps_everything() {
    use revo2win::convert::{Model, Report};
    use revo2win::detect;
    let Some(files) = files("REVO2WIN_WMDL_DIR") else {
        eprintln!("REVO2WIN_WMDL_DIR is not set");
        return;
    };
    let (mut n, mut bad) = (0, Vec::new());
    for path in files.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("0xb359c791"))) {
        let data = std::fs::read(path).unwrap();
        let win = WindowsModel::read(&data).unwrap();
        let game = detect::windows_game(&win);
        let r = &mut Report::default();
        let m0 = Model::from_windows(&win, r).unwrap();
        let revo = m0.to_revo(game, &keep_normals(), r).unwrap().write(game);
        let m1 = Model::from_revo(&RevoModel::read(&revo).unwrap(), r).unwrap();
        let back = m1.to_windows(game, &keep_normals(), r).unwrap().write();
        let m2 = Model::from_windows(&WindowsModel::read(&back).unwrap(), r).unwrap();
        let c0 = canon::of(&m0, false);
        if canon::of(&m1, false) != c0 || canon::of(&m2, false) != c0 {
            bad.push(path.clone());
        }
        n += 1;
    }
    eprintln!("{n} WindowsModels through RevoModel and back, {} changed", bad.len());
    for p in bad.iter().take(10) {
        eprintln!("changed: {}", p.display());
    }
    assert!(bad.is_empty());
}

/// The Wii and PC releases of MySims and Kingdom share most models: with the
/// files named by resource key, the converted Wii model should match the PC one.
#[test]
#[ignore]
fn wii_models_match_their_pc_versions() {
    use revo2win::convert::{Model, Report};
    use std::collections::{HashMap, HashSet};
    let (Some(revo), Some(win)) = (files("REVO2WIN_RMDL_DIR"), files("REVO2WIN_WMDL_DIR")) else {
        eprintln!("REVO2WIN_RMDL_DIR and REVO2WIN_WMDL_DIR are both needed");
        return;
    };
    let key = |p: &Path| p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase());
    let pc: HashMap<String, &PathBuf> = win
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("0xb359c791")))
        .filter_map(|p| Some((key(p)?, p)))
        .collect();
    let (mut pairs, mut same, mut same_shape) = (0, 0, 0);
    let mut seen = HashSet::new();
    for path in &revo {
        let Some(k) = key(path) else { continue };
        let Some(other) = pc.get(&k) else { continue };
        if !seen.insert(k) {
            continue;
        }
        let r = &mut Report::default();
        let a = Model::from_revo(&RevoModel::read(&std::fs::read(path).unwrap()).unwrap(), r).unwrap();
        let b = Model::from_windows(&WindowsModel::read(&std::fs::read(other).unwrap()).unwrap(), r).unwrap();
        pairs += 1;
        same += usize::from(canon::of(&a, false) == canon::of(&b, false));
        let shape = |m: &Model| canon::of(m, true).into_iter().map(|m| m.triangles).collect::<Vec<_>>();
        same_shape += usize::from(shape(&a) == shape(&b));
    }
    eprintln!("{pairs} Wii/PC pairs: {same} identical, {same_shape} with the same triangles");
    assert!(pairs == 0 || same_shape * 10 >= pairs * 9, "fewer than 90% have the same triangles");
}
