# CDVD: C++ vs Rust Port Comparison (Deep Analysis)

**Date:** 2026-07-12
**Scope:** `pcsx2/CDVD/` (C++ original) vs `pcsx2/rust/cdvd/src/` (Rust port)
**Method:** LOC + role audit of every file, plus boot validation with ISO/CHD/CSO.

---

## 1. Size Overview

| Side | LOC | Files |
|------|-----|-------|
| C++ `pcsx2/CDVD/` (all) | **10,372** | 40 |
| Rust `pcsx2/rust/cdvd/src/` | **691** | 6 |
| C++ readers replaced by Rust (see §2) | 925 | 4 |

The Rust crate is **6.7%** of the C++ CDVD LOC. But it does NOT replace the whole module — it replaces only the **on-disk format readers** (the `ThreadedFileReader` subclasses that read bytes off a file). Everything else in CDVD is still C++.

---

## 2. What Rust HAS Ported (✅ DONE)

Four on-disk image-format readers. All verified by unit tests + real boot (ISO/CHD/CSO) and boot test (Black USA).

| Format | C++ original | Rust | Notes |
|--------|-------------|------|-------|
| Raw ISO (sector read) | `FlatFileReader.cpp` 91 + raw part of `IsoReader` | `iso_reader.rs` 56 | Raw 2048-byte sector I/O |
| CHD | `ChdFileReader.cpp` 451 (**+ libchdr C lib**) | `chd_reader.rs` 135 | Pure-Rust `chd` crate; **removed C libchdr dependency** |
| CSO / ZSO | `CsoFileReader.cpp` 263 | `cso_reader.rs` 212 | zlib (CSO) + LZ4 (ZSO) via `flate2`/`lz4_flex` |
| Blockdump | `BlockdumpFileReader.cpp` 120 | `blockdump_reader.rs` 118 | Debug sparse dump format |
| **Subtotal** | **925** | **521** (4 readers) + 41 trait + 129 FFI | ~44% less LOC, no C deps |

**Build status:** `ChdFileReader.cpp`, `CsoFileReader.cpp`, `BlockdumpFileReader.cpp` are **already removed from `pcsx2/CMakeLists.txt`** (not compiled). `FlatFileReader.cpp` is still listed in CMake but is **dead code** (never instantiated — `InputIsoFile.cpp` routes every format to `RustFileReader`). Headers (`ChdFileReader.h` etc.) remain in the headers list but are unused.

**FFI bridge (intentional, stays C++):** `RustFileReader.cpp` (105 LOC) + `CDVD_Rust.h` (24) + `InputIsoFile.cpp` (250, now a dispatcher). This thin shim calls the Rust `pcsx2_cdvd_*` functions.

---

## 3. What Rust Has NOT Ported (❌ still C++) — "yang belum apa-apa"

### 3A. Logically portable later (good Rust candidates, part of CDVD)
| Component | C++ LOC | File(s) | Why not done yet |
|-----------|---------|---------|------------------|
| **ISO9660 filesystem parser** | 253 | `IsoReader.cpp` / `.h` | Directory walk, PVD, `SYSTEM.CNF` parse, file locate. **Still actively used by `CDVD.cpp` for disc detection** (the `Failed to get ELF name` warning path goes C++ IsoReader → `DoCDVDreadSector` → RustFileReader → Rust `iso_reader`). Pure logic, no C deps → ideal next CDVD sub-port. |
| **.gz reader** | 204 + 442 (`zlib_indexed.h`) = **646** | `GzippedFileReader.cpp`, `zlib_indexed.h` | gzip random-access index (zran). Only format still routed to C++ (`InputIsoFile` returns `GzippedFileReader` for `.gz`). Portable with `flate2` but the indexed-seek logic is the tricky part. |

### 3B. Emulation / platform layer (belongs to later "Core emulation" phase, NOT CDVD-reader work)
| Component | C++ LOC | File(s) | Role |
|-----------|---------|---------|------|
| **CDVD core peripheral** | 2,727 | `CDVD.cpp` | Command processing, NVRAM, `DoCDVDreadSector` dispatch, disc-type detect. The bulk of CDVD. |
| PS1 CD | 956 | `Ps1CD.cpp` | PS1 audio/data reading. |
| CDVD common | 518 | `CDVDcommon.cpp` / `.h` | Shared helpers. |
| Disc reader base | 474 | `CDVDdiscReader.cpp` / `.h` | Base class for readers. |
| Threaded reader wrapper | 366 | `ThreadedFileReader.cpp` / `.h` | Async read queue. **Kept on C++ by design** — `RustFileReader` subclasses it and calls Rust synchronously; threading stays C++. |
| ISO disc reader (high-level) | 346 | `CDVDisoReader.cpp` | Ties `IsoReader` + `ThreadedFileReader`. |
| Disc thread | 344 | `CDVDdiscThread.cpp` | Background read thread. |
| Physical drive IOCTL | 913 (×6 files) | `IOCtlSrc.cpp` (Win/Linux/macOS variants) | Real optical drives. **Not relevant to file-based emulation / Android** — skip. |
| ISO hasher | 156 | `IsoHasher.cpp` / `.h` | Hash calc. |
| Output ISO | 103 | `OutputIsoFile.cpp` | Writing disc images. |
| Drive utility | 189 (×3) | `DriveUtility.cpp` | Physical drive helpers. |
| Headers / defs | ~600 | `CDVD.h`, `CDVD_internal.h`, `IsoFileFormats.h`, etc. | Types & constants. |

---

## 4. Functional Coverage Matrix (boot-critical path)

| Capability | C++ path | Rust path | Status |
|------------|----------|-----------|--------|
| Read raw ISO sector | `FlatFileReader` | `iso_reader.rs` | ✅ replaced |
| Read CHD | `ChdFileReader`+libchdr | `chd_reader.rs` | ✅ replaced + boots |
| Read CSO/ZSO | `CsoFileReader` | `cso_reader.rs` | ✅ replaced + boots |
| Read blockdump | `BlockdumpFileReader` | `blockdump_reader.rs` | ✅ replaced |
| Read .gz | `GzippedFileReader` | — | ❌ still C++ |
| ISO9660 file lookup (SYSTEM.CNF) | `IsoReader` | — | ❌ still C++ (calls Rust for raw sectors) |
| Disc info / ELF CRC | `CDVD.cpp` + `IsoReader` | — | ❌ still C++ |
| CDVD command emulation | `CDVD.cpp` | — | ❌ still C++ (core) |
| Threaded async reads | `ThreadedFileReader` | — | ❌ still C++ (by design) |
| Physical optical drive | `IOCtlSrc` | — | ❌ not ported (skip for Android) |

---

## 5. Conclusions

1. **CDVD file-I/O layer = DONE.** The 4 on-disk readers are fully Rust, verified by unit tests (CHD/CSO bytes == ISO bytes) and by booting a real game from ISO, CHD, and CSO.
2. **Rust covers ~521 LOC of reader logic that was 925 LOC C++** (44% smaller) and **removed the C `libchdr` dependency** entirely. Pure-Rust crates only (`chd`, `flate2`, `lz4_flex`).
3. **The other ~9,800 LOC of CDVD is NOT ported** and is mostly emulation/platform code (CDVD.cpp core, PS1, threading, physical drives) that belongs to the later "Core emulation" phase — not the CDVD reader task.
4. **Two genuinely CDVD-scoped gaps remain** if we want CDVD 100% Rust:
   - `IsoReader` (ISO9660 parser, 253 LOC) — pure logic, ideal next step.
   - `GzippedFileReader` (646 LOC) — needs gzip random-access indexing in Rust.
5. **Cleanup opportunity:** `FlatFileReader.cpp` is compiled but dead. Can be dropped from `CMakeLists.txt` to fully remove the last unused C++ reader. (Not required for correctness.)

---

## 6. Recommendation

- CDVD readers are **complete and validated** — commit them.
- Do **not** expand CDVD scope now; move to the next planned module (Audio/SPU2 per the port plan).
- If a future pass wants CDVD 100% Rust, port `IsoReader` first (small, pure, high-value), then `GzippedFileReader`.
