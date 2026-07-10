# Analisis: `common/` C++ vs `rust/common/` Rust

## Ringkasan

| Aspek | C++ `common/` | Rust `rust/common/src/` | Status |
|-------|---------------|------------------------|--------|
| Total file | ~100+ (.cpp/.h) | 59 .rs | |
| Implementasi | ✅ Lengkap | ⚠️ Sebagian | Ada gap |
| Digunakan oleh | Semua file di pcsx2/ | via FFI + _rust_shim | Transisi |

---

## ✅ SUDAH di-Rust (lengkap)

| Modul | File Rust | Catatan |
|-------|-----------|---------|
| BitUtils | `bit_utils.rs` | ✅ |
| ByteSwap | `byte_swap.rs` | ✅ |
| Easing | `easing.rs` | ✅ |
| EnumOps | `enum_ops.rs` | ✅ |
| HashCombine | `hash_combine.rs` | ✅ |
| HeapArray | `heap_array.rs` | ✅ |
| HeterogeneousContainers | `heterogeneous_containers.rs` | ✅ |
| LRUCache | `lru_cache.rs` | ✅ |
| MD5Digest | `md5_digest.rs` | ✅ |
| Pcsx2Defs | `pcsx2_defs.rs` | ✅ |
| Pcsx2Types | `pcsx2_types.rs` | ✅ |
| ScopedGuard | `scoped_guard.rs` | ✅ |
| SingleRegisterTypes | `single_register_types.rs` | ✅ |
| SmallString | `small_string.rs` | ✅ |
| VectorIntrin | `vector_intrin.rs` | ✅ |
| WrappedMemCopy | `wrapped_mem_copy.rs` | ✅ |
| Assertions | `assertions.rs` | ✅ |
| AlignedMalloc | `aligned_malloc.rs` | ✅ |
| FastJmp | `fast_jmp.rs` | ✅ |
| CrashHandler | `crash_handler.rs` | ✅ |
| Error | `error.rs` | ✅ |
| ProgressCallback | `progress_callback.rs` | ✅ |
| ReadbackSpinManager | `readback_spin_manager.rs` | ✅ |
| Threading | `threading.rs` | ✅ |
| FileSystem | `file_system.rs` | ✅ |
| Path | `path.rs` | ✅ |
| StringUtil | `string_util.rs` | ✅ |
| HostSys (cross-platform) | `host_sys.rs`, `host_sys_ffi.rs` | ✅ |
| Console (logging) | `console.rs` | ✅ |
| MemorySettingsInterface | `memory_settings_interface.rs` | ✅ |
| SettingsInterface trait | `settings_interface.rs` | ✅ |
| MemoryInterface | `memory_interface.rs` | ✅ |
| HTTPDownloader | `http_downloader.rs` | ✅ |
| Image | `image.rs` | ✅ |
| TextureDecompress | `texture_decompress.rs` | ✅ |
| WAVWriter | `wav_writer.rs` | ✅ |
| WindowInfo | `window_info.rs` | ✅ |
| YAML | `yaml.rs` | ✅ |
| ZipHelpers | `zip_helpers.rs` | ✅ |
| FPControl | `fp_control.rs` | ✅ |
| Perf | `perf.rs` | ✅ |
| Timer | `timer.rs` | ✅ |
| CPUFeatures | `cpu_features.rs` | ✅ (x86_64 only) |
| CocoaTools | `cocoa_tools.rs` | ✅ (macOS) |
| DarwinMisc | `darwin_misc.rs` | ✅ (macOS) |
| DarwinThreads | `darwin_threads.rs` | ✅ (macOS) |
| LinuxHostSys | `linux_host_sys.rs` | ✅ (Linux) |
| LinuxMisc | `linux_misc.rs` | ✅ (Linux) |
| LinuxThreads | `linux_threads.rs` | ✅ (Linux) |

---

## ⚠️ ADA di Rust tapi DISABLED (tidak dikompilasi)

| Modul | File Rust | Alasan |
|-------|-----------|--------|
| **windows_host_sys** | `windows_host_sys.rs` | Disabled — windows-sys 0.61 API path mismatches. Diganti `host_sys_ffi.rs` |
| **windows_threads** | `windows_threads.rs` | Disabled — windows-sys 0.61 API mismatches remain |
| **dynamic_library** | `dynamic_library.rs` | Disabled — uses Win32 APIs not in pinned libc |
| **stack_walker** | `stack_walker.rs` | Disabled — uses `windows` crate not pinned |
| **semaphore_impl** | `semaphore_impl.rs` | Disabled — duplicate FFI symbols dengan `threading.rs` |

---

## ❌ BELUM ADA di Rust (hanya C++)

| Modul | File C++ | Dampak | Prioritas |
|-------|----------|--------|-----------|
| **WinMisc** | `common/Windows/WinMisc.cpp` | Windows utility functions (GetTickFrequency, GetPhysicalMemory, dll) → di-shim di `_shim_extras.cpp` | 🟡 Sedang |
| **RedtapeWilCom** | `common/RedtapeWilCom.h` | Windows COM wrappers (wil::com_ptr, wil::unique_couninitialize_call) — penting buat COM interop | 🟡 Sedang |
| **RedtapeWindows** | `common/RedtapeWindows.h` | Windows helper macros dan type aliases | 🟢 Rendah |
| **MRCHelpers** | `common/MRCHelpers.h` | MRC resource helpers | 🟢 Rendah |
| **RGBA8Image** (di Image.h) | `common/Image.h` | `RGBA8Image` type — dipakai di SaveState, GS, dll. Di-shim sbg stub doang | 🔴 TINGGI |
| **x86Emitter** (full) | `common/emitter/*` (50+ file) | JIT x86 assembler — `x86_emitter.rs` cuma proof-of-concept | 🔴 KRITIKAL |

---

## 🔶 DI-SHIM (C++ fallback, Rust belum implement penuh)

Fungsi-fungsi ini ada di `common/_rust_shim/_shim_extras.cpp` sebagai stub:

| Fungsi C++ | Status Rust | Keterangan |
|------------|-------------|------------|
| `GetTickFrequency()` | Panggil `pcsx2_timer_get_tick_frequency()` via FFI | ✅ Terhubung |
| `GetCPUTicks()` | Panggil `pcsx2_timer_get_cpu_ticks()` via FFI | ✅ Terhubung |
| `GetPhysicalMemory()` | Panggil `pcsx2_host_physical_memory()` via FFI | ✅ Terhubung |
| `GetAvailablePhysicalMemory()` | Fallback ke GetPhysicalMemory | ⚠️ Stub |
| `GetCPUInfo()` | Return `CPUInfo{}` static | ⚠️ Stub (gsrunner not use) |
| `ShortSpin()` | Return 0 | ⚠️ Stub |
| `AbortWithMessage()` | Panggil `std::abort()` | ⚠️ Stub |
| `GetOSVersionString()` | Return "PCSX2-shim" | ⚠️ Stub |
| `Common::InhibitScreensaver()` | Return false | ⚠️ Stub |
| `Common::PlaySoundAsync()` | Return false | ⚠️ Stub |
| `Common::SetMousePosition()` | No-op | ⚠️ Stub |
| `PageFaultHandler::Install()` | Return false | ⚠️ Stub |
| `ProgressCallback::*` | Method stubs | ⚠️ Stub |
| `RGBA8Image::*` | Method stubs | ⚠️ Stub |
| `SmallStringBase::operator=` | Implementasi manual | ✅ OK |

Juga ada shim terpisah untuk:
- `_shim_md5digest.cpp` — fallback MD5
- `_shim_settings.cpp` — fallback settings
- `_shim_texturedecompress.cpp` — fallback texture decompress
- `_shim_yaml.cpp` — fallback YAML parsing
- `_shim_readbackspinmanager.cpp` — fallback readback
- `_shim_perf.cpp` — fallback perf
- dll (total 20+ shim files)

---

## 📊 Kesimpulan

### Yang PERLU dilengkapi untuk Windows target:

| Priority | Item | Action |
|----------|------|--------|
| 🔴 1 | **x86Emitter** (x86_emitter.rs) | Masih "proof-of-concept". Perlu full JIT assembler via `iced-x86` |
| 🔴 2 | **RGBA8Image** | Dipakai SaveState, GS, dll. Image.rs mungkin perlu dilengkapi |
| 🔴 3 | **windows_host_sys.rs** | Enable + fix windows-sys API mismatches |
| 🟡 4 | **windows_threads.rs** | Enable + fix windows-sys API mismatches |
| 🟡 5 | **dynamic_library.rs** | Enable + fix Win32 APIs |
| 🟡 6 | **stack_walker.rs** | Enable (debug/crash reporting) |
| 🟡 7 | **WinMisc.rs** | Port `Windows/WinMisc.cpp` |
| 🟢 8 | **Shim stubs** | Implementasi beneran di Rust |
| 🟢 9 | **RedtapeWilCom** | Port ke `windows` crate |
| 🟢 10 | **SettingsWrapper macros** | Verifikasi sudah cover di Rust |

### Yang SUDAH OK:
- Semua leaf utility (bit, hash, heap, string, path, file, dll)
- Console/logging, threading, error handling
- Settings interface, memory interface
- HTTP downloader, YAML, ZIP, image
- Timer, perf, FP control
- Platform-specific (macOS, Linux) — lengkap

### Total gap:
- **~11 item** belum di-Rust (dari ~70+ total)
- **~20+ shim stub** yang masih C++ fallback
- **x86 emitter** sebagai gap terbesar (50+ file di `common/emitter/`)
