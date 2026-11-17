//! Telling which game a model file came from.
//!
//! Nothing in either format names the game, but the games differ in ways
//! that show: Agents has its own type ids, and MySims keys each model's
//! materials by the model's group where Kingdom puts its resources in group 0.

use crate::game::{Game, types};
use crate::rmdl::RevoModel;
use crate::wmdl::{ElementType, WindowsModel, usage};

/// The game a RevoModel is from. `type_id` is the resource type when known
/// (from a file name or a package).
pub fn revo_game(model: &RevoModel, type_id: Option<u32>) -> Game {
    if type_id == Some(types::REVO_MODEL_AGENTS) {
        return Game::Agents;
    }
    let materials = || model.meshes.iter().map(|m| m.material);
    if materials().any(|k| matches!(k.type_id, types::MATERIAL_AGENTS | types::MATERIAL_SET_AGENTS)) {
        Game::Agents
    } else if materials().any(|k| k.group != 0) {
        Game::MySims
    } else {
        Game::Kingdom
    }
}

/// The game a WindowsModel is from.
pub fn windows_game(model: &WindowsModel) -> Game {
    // Skin weights are the surest sign: three in MySims, four in Kingdom.
    let weights = model.meshes.iter().flat_map(|m| &m.keys).find(|k| k.usage == usage::WEIGHTS);
    match weights.map(|k| k.kind) {
        Some(ElementType::Float3) => Game::MySims,
        Some(ElementType::Float4) => Game::Kingdom,
        _ if model.meshes.iter().any(|m| m.material.group != 0) => Game::MySims,
        _ => Game::Kingdom,
    }
}

/// The resource type id in a file name such as
/// `0x00000000!0x00a94a37dbe5a6db.0xf9e50586`.
pub fn type_from_file_name(name: &str) -> Option<u32> {
    let ext = name.rsplit_once('.')?.1;
    let hex = ext.strip_prefix("0x").or_else(|| ext.strip_prefix("0X"))?;
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_in_name() {
        assert_eq!(type_from_file_name("0x00000000!0x00a94a37dbe5a6db.0xf9e50586"), Some(types::REVO_MODEL));
        assert_eq!(type_from_file_name("a.0x2954E734"), Some(types::REVO_MODEL_AGENTS));
        assert_eq!(type_from_file_name("model.rmdl"), None);
        assert_eq!(type_from_file_name("model"), None);
    }
}
