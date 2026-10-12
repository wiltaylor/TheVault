//! Seam S6: generate a vault of up to 1M notes from a seed, and time the operations the PRD sets
//! targets for (D31, D33, D39). `vault-bench` runs the whole benchmark; `just bench` builds and
//! runs it.

pub mod generate;
pub mod rng;
pub mod stats;
