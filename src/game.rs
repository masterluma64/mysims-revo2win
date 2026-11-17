//! The games and the resource type ids that differ between them.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Game {
    /// MySims: RevoModel on the Wii, WindowsModel in the Cozy Bundle.
    MySims,
    /// MySims Kingdom: RevoModel on the Wii, WindowsModel in the Cozy Bundle.
    Kingdom,
    /// MySims Agents: RevoModel on the Wii only, with its own type ids.
    Agents,
}

/// Resource type ids.
pub mod types {
    /// WindowsModel.
    pub const WINDOWS_MODEL: u32 = 0xb359_c791;
    /// RevoModel in MySims and Kingdom.
    pub const REVO_MODEL: u32 = 0xf9e5_0586;
    /// RevoModel in Agents.
    pub const REVO_MODEL_AGENTS: u32 = 0x2954_e734;
    pub const MATERIAL: u32 = 0x01d0_e75d;
    pub const MATERIAL_SET: u32 = 0x0201_9972;
    pub const MATERIAL_AGENTS: u32 = 0xe664_0542;
    pub const MATERIAL_SET_AGENTS: u32 = 0x787e_842a;
}

impl Game {
    pub const ALL: [Game; 3] = [Game::MySims, Game::Kingdom, Game::Agents];

    pub fn name(self) -> &'static str {
        match self {
            Game::MySims => "mysims",
            Game::Kingdom => "kingdom",
            Game::Agents => "agents",
        }
    }

    pub fn revo_model_type(self) -> u32 {
        match self {
            Game::Agents => types::REVO_MODEL_AGENTS,
            _ => types::REVO_MODEL,
        }
    }

    /// Whether the Cozy Bundle has this game, so whether it reads WindowsModels.
    pub fn has_windows_release(self) -> bool {
        self != Game::Agents
    }

    /// The type id this game uses for a material (`set == false`) or a
    /// material set.
    pub fn material_type(self, set: bool) -> u32 {
        match (self, set) {
            (Game::Agents, false) => types::MATERIAL_AGENTS,
            (Game::Agents, true) => types::MATERIAL_SET_AGENTS,
            (_, false) => types::MATERIAL,
            (_, true) => types::MATERIAL_SET,
        }
    }

    /// Whether `type_id` names a material set (in any game), or `None` when it
    /// is no material type at all.
    pub fn is_material_set(type_id: u32) -> Option<bool> {
        match type_id {
            types::MATERIAL | types::MATERIAL_AGENTS => Some(false),
            types::MATERIAL_SET | types::MATERIAL_SET_AGENTS => Some(true),
            _ => None,
        }
    }

    /// The game a RevoModel resource type belongs to, as far as it tells.
    pub fn from_revo_model_type(type_id: u32) -> Option<&'static [Game]> {
        match type_id {
            types::REVO_MODEL => Some(&[Game::MySims, Game::Kingdom]),
            types::REVO_MODEL_AGENTS => Some(&[Game::Agents]),
            _ => None,
        }
    }

    /// Whether material instances are 32-bit name hashes. The Cozy Bundle
    /// MySims names its materials with FNV32 where its Wii release used FNV64;
    /// everything else uses FNV64.
    pub fn windows_material_hash_is_32(self) -> bool {
        self == Game::MySims
    }
}

impl fmt::Display for Game {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Game {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mysims" | "ms" => Ok(Game::MySims),
            "kingdom" | "msk" | "mysims-kingdom" => Ok(Game::Kingdom),
            "agents" | "msa" | "mysims-agents" => Ok(Game::Agents),
            _ => Err(format!("unknown game `{s}` (expected mysims, kingdom or agents)")),
        }
    }
}

/// A resource key as both model formats store it: 64-bit instance, type, group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ResourceKey {
    pub instance: u64,
    pub type_id: u32,
    pub group: u32,
}

impl fmt::Display for ResourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:08x}!0x{:016x}.0x{:08x}", self.group, self.instance, self.type_id)
    }
}
