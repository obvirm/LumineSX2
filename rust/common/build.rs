// build.rs — generate pcsx2_common_rs.h from this crate's public API.
//
// Run automatically by Cargo before compilation. Output goes to OUT_DIR
// (consumed by CMake via cargo's metadata), and also a copy in the crate
// root for inspection.

use std::path::PathBuf;

fn main() {
    // Re-run if any source changes.
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/*.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // Try cbindgen; if it fails (e.g. due to parser quirks on some files),
    // skip header generation rather than failing the whole build.
    let config_path = crate_dir.join("cbindgen.toml");
    let config = match cbindgen::Config::from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cbindgen.toml parse failed: {e}");
            return;
        }
    };

    let bindings = match cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
    {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cbindgen generation failed (continuing without header): {e}");
            return;
        }
    };

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_dir.join("pcsx2_common_rs.h"));
    bindings
        .write_to_file(crate_dir.join("pcsx2_common_rs.h"));
}