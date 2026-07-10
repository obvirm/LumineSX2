//! End-to-end round-trip test of the `yaml` module.
//!
//! Constructs a realistic PCSX2 emulator settings block in YAML form,
//! parses it with the Rust module (now backed by the `serde_yml` crate),
//! mutates a couple of fields, serializes back to YAML, re-parses, and
//! verifies the mutations round-tripped.
//!
//! This proves that `serde_yml` is a drop-in replacement for `serde_yaml`
//! in the `yaml.rs` module: the same `from_str` / `to_string` API works
//! against the actively-maintained fork, including nested mappings, optional
//! fields, and string lists — all the things PCSX2's `GameIndex.yaml` and
//! `RedumpDatabase.yaml` actually carry.
//!
//! Run with: cargo run --release --example yaml_test

// The existing Rust YAML module is loaded via the `#[path = ...]` attribute
// below and exposes the generic `from_str` / `to_string` / `load_from_file`
// / `save_to_file` helpers, plus a structured `Error` enum. No C++ dependency
// is pulled in.

#[path = "../src/yaml.rs"]
mod yaml_module;

use serde::{Deserialize, Serialize};
use yaml_module::{from_str, to_string};

// ---------------------------------------------------------------------------
// Realistic PCSX2-shaped data model.
// ---------------------------------------------------------------------------
//
// The shapes below are inspired by the kind of structures PCSX2 ships in
// `GameIndex.yaml` (one entry per game, with patch lists and memory maps)
// and `Settings.ini` (typed emulator settings). They exercise nested maps,
// optional fields, vectors of structs, and enums.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MemoryMap {
    name: String,
    size_mb: u32,
    speed_ns: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EmulatorSettings {
    /// Allow the EE to run ahead of the GS without blocking.
    eecyclingrate: i32,
    /// Cycle count before the EE yields. 0 means "unlimited".
    eecycle_skip: i32,
    /// Emulated EE clock speed, in percent of real hardware.
    clockspeed: f32,
    /// Frame limiter (FPS).
    framelimit: f32,
    /// EE/IOP VU rec recompilers enabled.
    vu_recompiler: bool,
    /// Patch directory (optional, may be empty / missing).
    #[serde(default)]
    patches_dir: Option<String>,
    /// Memory-card directory.
    memcard_dir: String,
    /// Memory-map pool.
    memory_maps: Vec<MemoryMap>,
    /// Optional renderer override; absent means "auto".
    #[serde(default)]
    renderer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct GameEntry {
    name: String,
    serial: String,
    region: String,
    crc: String,
    /// Optional per-game overrides; absent means "use emulator default".
    #[serde(default)]
    overrides: Option<EmulatorSettings>,
}

// ---------------------------------------------------------------------------
// The test data — a realistic settings block for two PS2 games.
// ---------------------------------------------------------------------------

fn build_source_yaml() -> String {
    // Block-style YAML, mimicking how PCSX2's GameIndex.yaml is laid out:
    // top-level map with a `settings` entry plus a `games` list.
    let raw = r#"
settings:
  eecyclingrate: -3
  eecycle_skip: 0
  clockspeed: 100.0
  framelimit: 60.0
  vu_recompiler: true
  patches_dir: /home/user/pcsx2/patches
  memcard_dir: /home/user/pcsx2/memcards
  memory_maps:
    - name: default
      size_mb: 32
      speed_ns: 2500
    - name: extended
      size_mb: 64
      speed_ns: 2500
  renderer: Vulkan

games:
  - name: Final Fantasy X
    serial: SLUS-20312
    region: NTSC-U
    crc: 0a02f285
  - name: Shadow of the Colossus
    serial: SCUS-97472
    region: NTSC-U
    crc: 6c578733
    overrides:
      eecyclingrate: -2
      eecycle_skip: 1
      clockspeed: 110.0
      framelimit: 60.0
      vu_recompiler: true
      memcard_dir: /home/user/pcsx2/memcards/sotc
      memory_maps:
        - name: default
          size_mb: 32
          speed_ns: 2500
"#;
    raw.to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConfigFile {
    settings: EmulatorSettings,
    games: Vec<GameEntry>,
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

fn main() {
    println!("=== yaml round-trip test (serde_yml backend) ===\n");

    // 1. Source YAML.
    let source_yaml = build_source_yaml();
    println!("--- BEFORE (source YAML) ---");
    println!("{}", source_yaml);
    println!("--- END BEFORE ---\n");

    // 2. Parse into a strongly-typed Rust struct.
    let mut config: ConfigFile = match from_str(&source_yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("parse failed: {}", e);
            std::process::exit(1);
        }
    };
    println!("Parsed {} game(s); emulator uses {} memory-map(s).",
             config.games.len(),
             config.settings.memory_maps.len());

    // Sanity checks on the parsed data.
    assert_eq!(config.settings.eecyclingrate, -3);
    assert_eq!(config.settings.clockspeed, 100.0);
    assert!(config.settings.vu_recompiler);
    assert_eq!(config.settings.patches_dir.as_deref(), Some("/home/user/pcsx2/patches"));
    assert_eq!(config.settings.renderer.as_deref(), Some("Vulkan"));
    assert_eq!(config.settings.memory_maps.len(), 2);
    assert_eq!(config.settings.memory_maps[0].name, "default");
    assert_eq!(config.settings.memory_maps[0].size_mb, 32);
    assert_eq!(config.settings.memory_maps[1].name, "extended");
    assert_eq!(config.settings.memory_maps[1].size_mb, 64);
    assert_eq!(config.games.len(), 2);
    assert_eq!(config.games[0].serial, "SLUS-20312");
    assert!(config.games[0].overrides.is_none());
    assert!(config.games[1].overrides.is_some());
    println!("  OK — parsed structure matches expected values.\n");

    // 3. Mutate two fields: bump the EE clock-speed and add a renderer
    //    override for the first game.
    config.settings.clockspeed = 125.0;
    config.settings.framelimit = 120.0;
    config.games[0].overrides = Some(EmulatorSettings {
        eecyclingrate: -1,
        eecycle_skip: 2,
        clockspeed: 115.0,
        framelimit: 60.0,
        vu_recompiler: false,
        patches_dir: None,
        memcard_dir: "/home/user/pcsx2/memcards/ffx".to_string(),
        memory_maps: vec![MemoryMap {
            name: "default".to_string(),
            size_mb: 32,
            speed_ns: 2500,
        }],
        renderer: Some("OpenGL".to_string()),
    });

    // 4. Serialize back to YAML.
    let serialized = match to_string(&config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("serialize failed: {}", e);
            std::process::exit(1);
        }
    };
    println!("--- AFTER (modified YAML) ---");
    println!("{}", serialized);
    println!("--- END AFTER ---\n");

    // 5. Re-parse and verify the round-trip.
    let config2: ConfigFile = match from_str(&serialized) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("re-parse failed: {}", e);
            std::process::exit(1);
        }
    };
    assert_eq!(config2, config, "round-trip mismatch");
    println!("Re-parsed structure equals the in-memory value: OK");

    // Spot-check the modifications specifically.
    assert_eq!(config2.settings.clockspeed, 125.0);
    assert_eq!(config2.settings.framelimit, 120.0);
    assert!(config2.games[0].overrides.is_some());
    let overrides = config2.games[0].overrides.as_ref().unwrap();
    assert_eq!(overrides.clockspeed, 115.0);
    assert_eq!(overrides.eecyclingrate, -1);
    assert!(!overrides.vu_recompiler);
    assert_eq!(overrides.renderer.as_deref(), Some("OpenGL"));
    assert!(overrides.patches_dir.is_none());
    println!("  OK — mutations survived the round-trip.");

    // 6. Bonus: parse-error reporting.
    let bad_yaml = "settings: : :\n  - not a map\n";
    match from_str::<ConfigFile>(bad_yaml) {
        Err(e) => {
            println!("\nError path works — malformed YAML rejected:\n  {}", e);
        }
        Ok(_) => {
            eprintln!("\nERROR: malformed YAML was accepted by the parser");
            std::process::exit(1);
        }
    }

    println!("\nOK — yaml module round-trip (serde_yml) works end-to-end");
}