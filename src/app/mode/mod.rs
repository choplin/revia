//! Persistent mode-local programs coordinated by the application root.

pub mod composer;
pub mod help;
pub mod review;
pub mod rollup;

/// The single closed registry of interaction modes.
///
/// Mode-local events and effects remain owned by their modules. Root envelopes
/// only transport those values through the application loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActiveMode {
    #[default]
    Review,
    Composer,
    Help,
    Rollup,
}
