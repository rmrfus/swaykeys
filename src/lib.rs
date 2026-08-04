//! swaykeys — the sway config, read the way sway reads it.
//!
//! Split into a library so the parser can be exercised against fixture configs
//! from `tests/` without a running compositor.

pub mod group;
pub mod lex;
pub mod model;
pub mod render;
pub mod source;
pub mod vars;
pub mod xkb;
