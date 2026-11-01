//! Pause Pro: how the map looks while a match is paused (the game's grey-out, in colour, or
//! tinted with the pauser's colour), a box saying who paused, and a 5-second countdown before a
//! resume. `model` and `tone` are pure and tested everywhere; the game-facing layer is 32-bit
//! Windows only, like the game.

pub mod model;
pub mod tone;

#[cfg(target_arch = "x86")]
mod countdown;
#[cfg(target_arch = "x86")]
mod pause_box;
#[cfg(target_arch = "x86")]
mod plugin;
