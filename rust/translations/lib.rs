//! PCSX2 Rust Translations - master entry point
//!
//! Structural/stub translation. Method bodies are `unimplemented!()`.
#![allow(
    non_camel_case_types, non_snake_case, non_upper_case_globals,
    dead_code, unused_imports, unused_variables, unused_mut, unused_assignments,
    static_mut_refs, missing_docs, clippy::all,
)]

pub mod common;
pub mod pcsx2;
pub mod pcsx2_gsrunner;
pub mod pcsx2_qt;
pub mod tests;
pub mod thirdparty;
pub mod updater;
