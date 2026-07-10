# Context — PCSX2 common/ Rust Port Analysis

## Project
PCSX2 PS2 emulator. Porting `common/` C++ library to Rust in `rust/common/src/`.

## Architecture
- C++ files in `common/` → Rust files in `rust/common/src/`
- FFI bridge via `pcsx2_capi.cpp` ↔ `rust/common/src/host_sys_ffi.rs`
- Build: Cargo builds `.rlib`, linked via `-Clink-arg` in build.rs
- Target: Windows (MSVC), Vulkan renderer

## Status
- 61 Rust files total in `rust/common/src/`
- 6 agents already analyzed: Image, WinMisc, Redtape, DynamicLibrary, StackWalker, WinHostSys, WinThreads, x86Emitter, SPSC queue
- Agents 1-6 results: `analysis/agent01_image.md` through `analysis/agent06_spsc.md`
- 80 files remaining across 8 groups (A-H)

## Key Rules
- C++ header-only templates → Rust generic traits/structs
- Macros → Rust macros/macro_rules!
- No placeholder code, no TODOs
- Report every missing function
