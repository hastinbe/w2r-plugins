//! Buff Icons: the spell icons over units packed and wrapped by unit size, a countdown on them in
//! their last 10 seconds, and see-through health bars. `model` is pure and tested everywhere; the
//! game-facing layer is 32-bit Windows only, like the game.

pub mod model;

#[cfg(target_arch = "x86")]
mod plugin;
