//! EneeSpec compiler: `enee.yaml` → the compiled app artifact. All SQL is prepared against a scratch
//! SQLite database (feature `sqlite`).
//!
//! Ported from eneeware `crates/enee-spec` at 61bc7a7 (2026-10-07), now owned by r1os (ADR-0005).
//! eneeware's specs it references (docs/specs/enee-spec.md, compiled-app.md) live in that repo; r1os
//! changes are documented in docs/specs/apps.md. Dropped here: the WASM preview build.

mod blocks;
#[cfg(feature = "sqlite")]
pub mod checks;
mod compile;
pub mod compiled;
mod data;
pub mod diag;
pub mod eval;
pub mod icons;
mod lines;
pub mod patch;
pub mod sql;
mod util;
mod value;

#[cfg(feature = "sqlite")]
pub use checks::{CheckResult, Sandbox, run_checks};
pub use compile::{MAX_SOURCE_BYTES, Options, Output, compile};
pub use util::duration_ms;
