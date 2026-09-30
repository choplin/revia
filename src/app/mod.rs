//! Deterministic Multilayer Elm application.

pub mod effect;
pub mod global;
pub mod input;
pub mod mode;
mod root;
#[cfg(test)]
mod scenario;
pub mod view;
pub mod view_state;

pub use effect::{Effect, EffectResult, Outcome};
pub use root::{Model, handle_input, update, view};
