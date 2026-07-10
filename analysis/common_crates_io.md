# Crate.io Equivalents — External C/C++ Library Calls di `common/`

Untuk setiap file C++ yang **manggil library eksternal/third-party C++**, dicarikan Rust crate dari crates.io.

---

## 1. `common/Image.cpp` — Image I/O (RGBA8Image)

### External C/C++ Libraries Dipanggil

| Library | Include | Fungsi |
|---------|---------|--------|
| **libjpeg** | `<jpeglib.h>` | `jpeg_create_decompress()`, `jpeg_read_header()`, `jpeg_start_decompress()`, `jpeg_read_scanlines()`, `jpeg_finish_decompress()`, `jpeg_destroy_decompress()` + encode counterparts |
| **libpng** | `<png.h>` | `png_create_read_struct()`, `png_create_info_struct()`, `png_init_io()`, `png_read_image()`, `png_write_image()`, `png_destroy_read_struct()`, dll |
| **libwebp** | `<webp/decode.h>` | `WebPGetInfo()`, `WebPDecodeRGBA()`, `WebPDecodeRGB()` |
| **libwebp** | `<webp/encode.h>` | `WebPEncodeRGBA()`, `WebPEncodeLosslessRGBA()` |

### Rust Crate.io Equivalents

| Library | Rust Crate | Command | Notes |
|---------|-----------|---------|-------|
| libjpeg | **`image`** | `cargo add image` | Paling populer, support JPEG/PNG/WebP via feature flags. Bisa load/save file |
| libjpeg | **`jpeg-decoder`** + **`jpeg-encoder`** | `cargo add jpeg-decoder jpeg-encoder` | Kalau butuh kontrol lebih |
| libjpeg | **`turbojpeg`** | `cargo add turbojpeg` | Yang paling cepat (bind ke libturbojpeg C) |
| libpng | **`png`** | `cargo add png` | Pure Rust PNG encoder/decoder |
| libpng | **`image`** | (already counted) | Fitur `png` via default |
| libwebp | **`webp-rs`** | `cargo add webp-rs` | Rust bind ke libwebp C |
| libwebp | **`webp-rust`** | `cargo add webp-rust` | Pure Rust (decode only, partial encode) |
| libwebp | **`image`** | (already counted) | Fitur `webp` via feature flag |

**Rekomendasi:** Pakai `image` crate aja (fitur `jpeg`, `png`, `webp`). Satu crate buat semua format.

---

## 2. `common/RedtapeWilCom.h` — Windows COM Wrappers

### External C/C++ Libraries Dipanggil

| Library | Include | Fungsi |
|---------|---------|--------|
| **WIL** (Windows Implementation Library) | `<wil/com.h>` | `wil::com_ptr<T>`, `wil::com_ptr_nothrow<T>`, `wil::unique_couninitialize_call`, `wil::unique_hmodule`, WIL COM smart pointers & RAII wrappers |

### Rust Crate.io Equivalents

| Library | Rust Crate | Command | Notes |
|---------|-----------|---------|-------|
| WIL com_ptr | **`windows`** | `cargo add windows` | `windows::Win32::System::Com` punya `CoInitializeEx`, `CoUninitialize`, dll |
| WIL com_ptr | **`com-rs`** / **`intercom`** | `cargo add com` | COM abstraction di Rust |
| WIL unique_couninitialize | **`windows`** | (sama) | `CoUninitialize` via `windows` crate + Drop guard manual |
| WIL unique_hmodule | **`windows`** | (sama) | `LoadLibraryExW` / `FreeLibrary` via `windows` crate |

**Rekomendasi:** Pakai `windows` crate langsung (`windows = { version = "0.61", features = ["Win32_System_Com", "Win32_System_LibraryLoader"] }`). COM smart pointer tidak terlalu diperlukan di Rust karena ownership system sudah handle RAII.

---

## 3. `common/Windows/WinMisc.cpp` — Windows Utilities

### External C/C++ Libraries Dipanggil

| Library | Include | Fungsi |
|---------|---------|--------|
| **Win32 MMSystem** | `<mmsystem.h>` | `timeGetTime()`, multimedia timers |
| **Win32 TimeAPI** | `<timeapi.h>` | `timeBeginPeriod()`, `timeEndPeriod()` — high-res timer |
| **Win32 VersionHelpers** | `<VersionHelpers.h>` | `IsWindowsVersionOrGreater()`, OS version check |
| **Win32 Kernel32** | (via Windows.h) | `SetThreadExecutionState()`, `GetPhysicallyInstalledSystemMemory()`, `GetTickCount64()` |
| **Win32 User32** | (via Windows.h) | `SetCursorPos()`, `GetCursorPos()` |

### Rust Crate.io Equivalents

| Win32 API | Rust Crate | Notes |
|-----------|-----------|-------|
| `timeGetTime`, `timeBeginPeriod` | **`windows`** = `{ features: ["Devices_Sensors", "System_Power"] }` | Atau `std::time` |
| `SetThreadExecutionState` | **`windows`** = `{ features: ["System_Power"] }` | ES_DISPLAY_REQUIRED dll |
| `GetPhysicallyInstalledSystemMemory` | **`windows`** = `{ features: ["System_SystemInformation"] }` | `GlobalMemoryStatusEx` |
| `SetCursorPos` | **`windows`** = `{ features: ["UI_Input"] }` | Atau `enigo` crate |
| `IsWindowsVersionOrGreater` | **`windows`** = `{ features: ["System_SystemInformation"] }` | `RtlGetVersion` |
| Sleep/Threading | **`std::thread`** | Built-in |

**Rekomendasi:** Semua via `windows` crate + `std::thread`.

---

## 4. `common/StackWalker.h` — Stack Tracing

### External C/C++ Libraries Dipanggil

| Library | Include | Fungsi |
|---------|---------|--------|
| **DbgHelp** (Win32) | `<windows.h>` + `dbghelp.dll` | `StackWalk64()`, `SymGetModuleBase64()`, `SymFromAddr()`, `SymGetLineFromAddr64()`, `SymInitialize()`, `SymCleanup()` |
| **Win32** | `RedtapeWindows.h` | Process/thread handles |

### Rust Crate.io Equivalents

| Library | Rust Crate | Command | Notes |
|---------|-----------|---------|-------|
| DbgHelp | **`backtrace`** | `cargo add backtrace` | Stack unwinding + symbol resolution. Cross-platform |
| DbgHelp | **`addr2line`** | `cargo add addr2line` | Address → file:line resolution |
| DbgHelp | **`windows`** | (sama) | `DbgHelp` functions via `windows` crate features |

**Rekomendasi:** `backtrace` crate (built-in, maintained, cross-platform). Tidak perlu DbgHelp.

---

## 5. Threading (`common/Threading.h`)

### External C/C++ Libraries Dipanggil

| Library | Include | Fungsi |
|---------|---------|--------|
| **POSIX** | `<semaphore.h>` | `sem_init()`, `sem_wait()`, `sem_post()`, `sem_destroy()` (Linux) |
| **macOS** | `<mach/semaphore.h>` | `semaphore_create()`, `semaphore_wait()`, `semaphore_signal()` (macOS) |
| **Win32** | (via Windows.h) | `CreateSemaphore()`, `WaitForSingleObject()`, `ReleaseSemaphore()` (Windows) |
| **C++ STL** | `<atomic>`, `<functional>`, `<thread>` | `std::atomic`, `std::function`, `std::thread` |

### Rust Crate.io Equivalents

| Library | Rust Crate | Notes |
|---------|-----------|-------|
| Semaphore (POSIX) | **`std::sync::Semaphore`** | (di-nightly) atau `tokio::sync::Semaphore` |
| Semaphore (Win32) | **`std::sync::Semaphore`** | Sama |
| Atomic ops | **`std::sync::atomic`** | Built-in |
| Thread | **`std::thread`** | Built-in |
| Mutex | **`std::sync::Mutex`** | Built-in |
| ConditionVariable | **`std::sync::Condvar`** | Built-in |
| Thread pool | **`threadpool`**, **`rayon`** | Cargo tambahan |

**Rekomendasi:** Semua built-in `std::sync`. Semaphore via `tokio::sync::Semaphore` atau custom pakai `Condvar` + `Mutex`.

---

## 6. `common/SettingsWrapper.h` — Settings Serialization

### External C/C++ Libraries Dipanggil

**TIDAK ADA!** SettingsWrapper.h cuma pake `SettingsInterface.h` (internal PCSX2) via template + macro. Ini pure C++ pattern, tidak manggil library eksternal.

### Rust Equivalent

| Feature | Rust Standard | Notes |
|---------|--------------|-------|
| Template/macro → serialize | **Trait + derive** | `serde::Serialize`/`Deserialize` pattern |
| Entry() pattern | **Custom trait** | `SettingsInterface` trait sudah ada di `settings_interface.rs` |
| EnumEntry() | **serde** + `strum` | `strum::EnumString` + `strum::Display` |

**Rekomendasi:** `serde` + `serde_yaml`/`serde_json` buat serialization. SettingsWrapper macros tidak perlu di-Rust — pake trait-based approach yang udah ada.

---

## 7. `MRCHelpers.h` — MRC Resource

### External C/C++ Libraries Dipanggil

**TIDAK ADA.** Cuma `#include <cstddef>` dan `#include <utility>`. Ini helper trivially small.

### Rust Equivalent
- `std::convert`, `std::mem` — built-in. Tidak perlu crate.

---

## 8. `RedtapeWindows.h` — Windows Header Wrapper

### External C/C++ Libraries Dipanggil

| Library | Include |
|---------|---------|
| **Win32 SDK** | `<Windows.h>` (dengan WIN32_LEAN_AND_MEAN, NOMINMAX) |

### Rust Equivalent
- **`windows`** crate — Win32 API bindings yang comprehensive.

---

## 9. `x86Emitter` (`common/emitter/`)

### External C/C++ Libraries Dipanggil

**TIDAK ADA.** x86Emitter adalah library internal PCSX2 milik sendiri. Tidak manggil xbyak atau library eksternal lain. File-file di `common/emitter/` adalah implementasi x86 assembler buatan sendiri.

### Rust Equivalent

| Component | Rust Crate | Notes |
|-----------|-----------|-------|
| Full x86 assembler | **`iced-x86`** | `cargo add iced-x86` — The BEST Rust x86 library. Disassembler + assembler + encoder + decoder + formatter. 15K+ stars |
| JIT codegen | **`iced-x86` CodeAssembler** | `iced-x86::CodeAssembler` — buat dynamic code generation (ganti x86Emitter) |
| Instruction encoding | **`iced-x86`** | `Instruction::encode()` → bytecode |
| CPUID detection | **`raw-cpuid`** | CPU feature detection |

**Rekomendasi:** `iced-x86` crate adalah satu-satunya yang perlu untuk ganti 50+ file `common/emitter/*`. Crate ini punya:
- `Decoder` — x86 instruction decoder
- `Encoder` — x86 instruction encoder
- `Formatter` — instruction formatting
- `CodeAssembler` — dynamic code generation (JIT)
- Support: x86 (16/32/64-bit), x87, MMX, SSE, AVX, AVX-512, FPU, VEX, EVEX, MVEX

---

## 10. `DynamicLibrary.h` — Dynamic Library Loading

### External C/C++ Libraries Dipanggil

**TIDAK ADA langsung di header.** Implementasi di `.cpp` panggil:
- Win32: `LoadLibraryExW()`, `GetProcAddress()`, `FreeLibrary()`
- POSIX: `dlopen()`, `dlsym()`, `dlclose()`

### Rust Crate.io Equivalents

| API | Rust Crate | Notes |
|-----|-----------|-------|
| LoadLibrary/dlsym | **`libloading`** | `cargo add libloading` — Cross-platform dynamic library loading. Recommended! |
| Win32 version | **`windows`** | `LoadLibraryExW` dll |

**Rekomendasi:** `libloading` crate (0.8+). Simpel, cross-platform.

---

## RINGKASAN — Semua Crate yang Dibutuhkan

| # | Crate | Command | Untuk |
|---|-------|---------|-------|
| 1 | **`image`** | `cargo add image` | JPEG + PNG + WebP I/O (ganti libjpeg, libpng, libwebp) |
| 2 | **`windows`** | `cargo add windows` | Semua Win32 API (COM, time, memory, dbghelp, registry) |
| 3 | **`libloading`** | `cargo add libloading` | Dynamic library loading (ganti LoadLibrary/dlsym) |
| 4 | **`backtrace`** | `cargo add backtrace` | Stack tracing (ganti DbgHelp/StackWalker) |
| 5 | **`iced-x86`** | `cargo add iced-x86` | x86 assembler + disassembler (ganti x86Emitter + Zydis + xbyak) |
| 6 | **`raw-cpuid`** | `cargo add raw-cpuid` | CPU feature detection |
| 7 | **`serde`** | `cargo add serde` | Settings serialization (ganti SettingsWrapper) |
| 8 | **`serde_yaml`** | `cargo add serde_yaml` | YAML parsing (ganti rapidyaml) |

**Yang SUDAH built-in Rust (no crate needed):**
| Fitur | Rust std | Ganti |
|-------|----------|-------|
| Thread | `std::thread` | Threading.h |
| Mutex | `std::sync::Mutex` | Threading.h |
| Condvar | `std::sync::Condvar` | Threading.h |
| Semaphore | `tokio::sync::Semaphore` / custom | Threading.h |
| Atomic | `std::sync::atomic` | Threading.h |
| Sleep | `std::thread::sleep` | WinMisc.cpp |
| Time | `std::time` | WinMisc.cpp timeAPI |
| File I/O | `std::fs`, `std::io` | FileSystem.h |
| HEader only | — | MRCHelpers.h, RedtapeWindows.h |
