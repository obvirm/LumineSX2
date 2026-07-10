// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/YAML.{h,cpp}`.
//!
//! The original C++ code is a thin wrapper over
//! [`rapidyaml`](https://github.com/biojppm/rapidyaml) that parses YAML
//! strings into a [`ryml::Tree`] while recovering from parse errors via
//! `setjmp`/`longjmp` (the project disables C++ exceptions).
//!
//! This module mirrors the public surface area of the C++ header —
//! [`parse`], [`serialize`], [`load_from_file`] and [`save_to_file`] — but
//! the underlying rapidyaml calls are stubbed out with
//! [`unimplemented!()`] because the file is part of a translation
//! skeleton, not a fully functional YAML port.

use std::fs;
use std::io;
use std::path::Path;

/// Opaque YAML document value.
///
/// In the C++ code this is a `ryml::Tree`; here it is a placeholder
/// struct that stands in for the parsed document until a real rapidyaml
/// binding is wired up.
#[derive(Debug, Default, Clone)]
pub struct Yaml {
    _private: (),
}

impl Yaml {
    /// Construct an empty YAML document.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parse a YAML string into a [`Yaml`] document.
///
/// This is the Rust equivalent of `Pcsx2Yaml::Parse` from the C++ side
/// (or, more directly, of the internal `ParseYAMLFromString` helper).
/// The original code recovers from parse errors via `setjmp`/`longjmp`;
/// in idiomatic Rust that becomes a [`Result`] whose `Err` variant
/// carries the error description.
pub fn parse(s: &str) -> Result<Yaml, String> {
    let _ = s;
    unimplemented!("rapidyaml parse stub")
}

/// Serialize a [`Yaml`] document into a YAML string.
///
/// Mirrors `Pcsx2Yaml::Serialize` from the C++ code.
pub fn serialize(y: &Yaml) -> String {
    let _ = y;
    unimplemented!("rapidyaml serialize stub")
}

/// Load a YAML document from the file at `path`.
///
/// On the C++ side the file is read into memory and then handed to
/// `ParseYAMLFromString`; this Rust wrapper preserves the public
/// `Result<Yaml, std::io::Error>` signature so that the call sites in
/// the rest of the port can be translated one-for-one.
pub fn load_from_file(path: &Path) -> Result<Yaml, io::Error> {
    let _ = fs::read_to_string(path);
    unimplemented!("rapidyaml load_from_file stub")
}

/// Write a YAML document to the file at `path`.
///
/// The C++ counterpart is a `ryml::emit` into a `std::ofstream`; here
/// it is stubbed out alongside the rest of the rapidyaml surface.
pub fn save_to_file(path: &Path, yaml: &Yaml) -> Result<(), io::Error> {
    let _ = (fs::write(path, b""), yaml);
    unimplemented!("rapidyaml save_to_file stub")
}
