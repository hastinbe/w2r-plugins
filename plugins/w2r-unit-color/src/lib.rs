//! Unit Color: custom colours for the eight teams, local only. `model`, `profiles` and `json` are
//! pure and tested everywhere; the game-facing layer is 32-bit Windows only, like the game.

pub mod allied;
pub mod json;
pub mod model;
pub mod profiles;

#[cfg(target_arch = "x86")]
mod plugin;
