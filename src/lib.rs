//! Conversion between the two model formats of the MySims games: RevoModel,
//! used by the Wii releases of MySims, MySims Kingdom and MySims Agents, and
//! WindowsModel, used by the Cozy Bundle releases of MySims and MySims
//! Kingdom on PC.
//!
//! [`rmdl`] and [`wmdl`] read and write each format as it is stored.
//! [`convert`] goes between them through [`convert::Model`], a plain list of
//! vertices and triangles per mesh.

pub mod binary;
pub mod convert;
pub mod detect;
pub mod game;
pub mod hash;
pub mod model;
pub mod package;
pub mod rmdl;
pub mod strip;
pub mod wmdl;

pub use convert::{Model, Options};
pub use game::Game;
pub use rmdl::RevoModel;
pub use wmdl::WindowsModel;
