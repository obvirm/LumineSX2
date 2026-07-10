# Debugger & JIT Module — External Library Analysis

## Files Analysed

### 1. `x86/iR3000A.cpp` — IOP (R3000A) Recompiler

**Lokasi:** `E:\project\pcsx2\pcsx2\x86\iR3000A.cpp`

**External dependencies:**

| Library | Header | Conditional? | Usage |
|---------|--------|-------------|-------|
| **Zydis** | `<Zydis/Zydis.h>` | `#ifdef DUMP_BLOCKS` | x86-64 disassembler for debug traces |
| **Zycore** | `<Zycore/Format.h>`, `<Zycore/Status.h>` | `#ifdef DUMP_BLOCKS` | Zydis core utilities |
| **zlib** | `<zlib.h>` | `#ifdef TRACE_BLOCKS` | Block trace compression |
| **x86Emitter** (internal) | `common/emitter/x86emitter.h` | always | Custom PCSX2 x86 JIT emitter (NOT xbyak) |

**Catatan:**
- Zydis dan zlib **hanya aktif** kalau `DUMP_BLOCKS`/`TRACE_BLOCKS` di-`#define` — defaultnya **tidak aktif**.
- Recompiler pake `x86Emitter` milik PCSX2 sendiri, **bukan xbyak**.
- `x86Emitter` adalah custom emitter internal di `common/emitter/`.

---

### 2. `x86/ix86-32/iR5900.cpp` — EE (R5900) Recompiler

**Lokasi:** `E:\project\pcsx2\pcsx2\x86\ix86-32\iR5900.cpp`

**External dependencies:**

| Library | Header | Usage |
|---------|--------|-------|
| **x86Emitter** (internal) | `common/emitter/x86emitter.h` (via iR5900.h) | EE recompiler JIT |

**Catatan:**
- Tidak panggil library eksternal langsung — semua emitter via `x86Emitter` internal.
- File ini adalah **heartbeat PS2 emulation** — recompiler main CPU (EE).

---

### 3. `x86/microVU.cpp` — VU (Vector Unit) Recompiler

**Lokasi:** `E:\project\pcsx2\pcsx2\x86\microVU.cpp`

**External dependencies:**

| Library | Header | Conditional? | Usage |
|---------|--------|-------------|-------|
| **zlib** | `<zlib.h>` | `#if 0` (dead code) | VU state dump |
| **x86Emitter** (internal) | `common/emitter/x86emitter.h` | always | VU JIT compilation |

**Catatan:**
- `<zlib.h>` di dalam `#if 0` — **mati total**, tidak aktif.
- microVU pake internal emitter juga.

---

### 4. `arm64/AsmHelpers.h` + `.cpp` — ARM64 JIT Helpers

**Lokasi:** `E:\project\pcsx2\pcsx2\arm64\AsmHelpers.h`

**External dependencies:**

| Library | Header | Usage |
|---------|--------|-------|
| **vixl** | `<vixl/aarch64/constants-aarch64.h>` | ARM64 constant definitions |
| **vixl** | `<vixl/aarch64/macro-assembler-aarch64.h>` | ARM64 macro assembler |

**Catatan:**
- vixl = `3rdparty/vixl` — ARM64 assembler library dari Google (V8 JavaScript engine).
- Dipakai untuk ARM64 JIT backend (dynamic code generation).
- Juga dipakai di `arm64/Vif_UnpackNEON.h` dan file ARM64 lainnya.

---

### 5. `GS/Renderers/SW/GSDrawScanlineCodeGenerator.*` — GS SW Renderer

**Lokasi:** `E:\project\pcsx2\pcsx2\GS\Renderers\SW\`

**External dependencies:**

| Library | Header | Usage |
|---------|--------|-------|
| **xbyak** | `xbyak/xbyak.h`, `xbyak/xbyak_util.h` | Dynamic codegen untuk scanline drawing |

**Catatan:**
- xbyak dipakai **hanya di GS Software Renderer**, bukan di EE/VU recompiler.
- `GSNewCodeGenerator.h` adalah wrapper PCSX2 di atas xbyak.
- File terkait: `GSDrawScanlineCodeGenerator.all.cpp/.h`, `GSSetupPrimCodeGenerator.all.cpp/.h`.

---

### 6. `DebugTools/SymbolImporter.cpp` — Symbol Import

**Lokasi:** `E:\project\pcsx2\pcsx2\DebugTools\SymbolImporter.cpp`

**External dependencies:**

| Library | Header | Usage |
|---------|--------|-------|
| **ccc** | `<ccc/ast.h>` | C/C++ AST parsing |
| **ccc** | `<ccc/elf.h>` | ELF binary parsing |
| **ccc** | `<ccc/importer_flags.h>` | Import configuration |
| **ccc** | `<ccc/symbol_file.h>` | Symbol file I/O |
| **demangle** | `<demangle.h>` | C++ symbol demangling |

---

### 7. `DebugTools/SymbolGuardian.h` — Symbol Cache

**Lokasi:** `E:\project\pcsx2\pcsx2\DebugTools\SymbolGuardian.h`

**External dependencies:**

| Library | Header | Usage |
|---------|--------|-------|
| **ccc** | `<ccc/ast.h>` | AST types for symbol DB |
| **ccc** | `<ccc/symbol_database.h>` | Symbol database queries |
| **ccc** | `<ccc/symbol_file.h>` | Symbol file reading |

---

## Ringkasan Library Eksternal

| Library | Lokasi 3rdparty | Dipakai di File | Fungsi |
|---------|----------------|-----------------|--------|
| **x86Emitter** | `common/emitter/` (internal) | iR3000A.cpp, iR5900.cpp, microVU.cpp, iFPU.cpp, dll | x86 JIT codegen — **bukan xbyak**, punya PCSX2 sendiri |
| **xbyak** | `3rdparty/xbyak/` | GS/Renderers/SW/* | GS SW Renderer dynamic codegen (SSE/AVX) |
| **vixl** | `3rdparty/vixl/` | arm64/AsmHelpers.h, arm64/* | ARM64 JIT codegen |
| **Zydis** | `3rdparty/zydis/` | iR3000A.cpp (`#ifdef DUMP_BLOCKS`) | x86 disassembler (opsional) |
| **ccc** | `3rdparty/ccc/` | DebugTools/SymbolImporter.cpp, SymbolGuardian.h | C/C++ code analysis untuk symbol import |
| **demangle** | `3rdparty/demangler/` | DebugTools/SymbolImporter.cpp | C++ name demangling |
| **zlib** | `3rdparty/include/` | iR3000A.cpp (opsional), microVU.cpp (dead code) | Compression (tidak aktif) |

## Implikasi untuk Rust Port

1. **x86Emitter → Rust JIT crate**:
   - Butuh `iced-x86` (disassembler) + custom assembler
   - Atau port x86Emitter ke Rust (~10K lines)
   - **Ini bagian TERBERAT** dari rewrite

2. **xbyak → Rust JIT**:
   - GS SW Renderer punya dynamic codegen
   - Kalau target Vulkan, GS HW Renderer mungkin cukup — SW Renderer bisa skip

3. **vixl → Rust ARM64**:
   - `iced-x86` support ARM64 juga
   - Atau `arm64` crate

4. **ccc → Rust**:
   - ELF parsing bisa pake `goblin` crate
   - AST parsing bisa pake `tree-sitter`

5. **demangle → Rust**:
   - `rustc-demangle` crate

6. **Zydis → Rust**:
   - `iced-x86` crate (disassemble)

7. **Kesimpulan**: JIT/Recompiler adalah **bagian paling kompleks** untuk port ke Rust karena dynamic code generation.
