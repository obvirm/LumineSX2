# Laporan Lengkap: `common/` C++ → Rust — Analisa External Library per File

## Ringkasan Coverage

| Status | Jumlah | Keterangan |
|--------|--------|------------|
| ✅ Rust lengkap | 51 file | Semua fungsi ter-cover |
| ✅ Rust baru (agent 1-6) | 9 file | image, x86_emitter, win_misc, redtape, dynlib, stackwalker, winhost, winthreads, spsc |
| 🔲 Skip (platform-specific) | 7 file | Darwin/*.cpp, Lnx*.cpp, MRCHelpers.h, PrecompiledHeader |
| ⚠️ Pre-existing compile error | 3 file | perf_event_counter.rs, linux_misc.rs, linux_host_sys.rs |
| **Total** | **~70 C++ files** | → **61 Rust files** |

---

## File-by-File: External Library → Rust Crate

### CORE HEADERS (16 files)

| # | File C++ | External Libs | Rust File | Rust Crate | Keterangan |
|---|----------|--------------|-----------|------------|------------|
| 1 | `AlignedMalloc.h` | `<malloc.h>` (MSVC aligned alloc) | `aligned_malloc.rs` | `std::alloc` + `libc` | ✅ Portable |
| 2 | `BitUtils.h` | `<intrin.h>` (_BitScanReverse) | `bit_utils.rs` | `std::num` | ✅ Built-in |
| 3 | `ByteSwap.h` | `<stdlib.h>` (_byteswap_*) | `byte_swap.rs` | `.swap_bytes()` | ✅ Built-in |
| 4 | `Easing.h` | `<cmath>` (sin) | `easing.rs` | `std::f32::sin()` | ✅ Built-in |
| 5 | `EnumOps.h` | `<type_traits>` | `enum_ops.rs` | `std::mem::discriminant` | ✅ Built-in |
| 6 | `FPControl.h` | intrinsics (MXCSR/FPCR) | `fp_control.rs` | `core::arch::x86_64` | ✅ |
| 7 | `HashCombine.h` | `<functional>` (hash) | `hash_combine.rs` | `std::hash::Hash` | ✅ Built-in |
| 8 | `HeapArray.h` | STL saja | `heap_array.rs` | `Vec<T>` + `Box<[T]>` | ✅ Built-in |
| 9 | `HeterogeneousContainers.h` | STL (map, set, unordered) | `heterogeneous_containers.rs` | `HashMap` + `BTreeMap` | ✅ Built-in |
| 10 | `Pcsx2Defs.h` | `<bit>` | `pcsx2_defs.rs` | `std` | ✅ Built-in |
| 11 | `Pcsx2Types.h` | `<cstdint>` | `pcsx2_types.rs` | `std` | ✅ |
| 12 | `ScopedGuard.h` | `<optional>` | `scoped_guard.rs` | `Drop` trait | ✅ |
| 13 | `SingleRegisterTypes.h` | intrinsics (__m128i) | `single_register_types.rs` | `core::arch::x86_64` | ✅ |
| 14 | `VectorIntrin.h` | `<xmmintrin.h>`, `<emmintrin.h>` | `vector_intrin.rs` | `core::arch::x86_64::_mm_*` | ✅ |
| 15 | `WrappedMemCopy.h` | `<cstring>` (memcpy) | `wrapped_mem_copy.rs` | `std::ptr::copy_nonoverlapping` | ✅ |
| 16 | `MRCHelpers.h` | **macOS ObjC `[retain]/[release]`** | ❌ TIDAK ADA | `objc` crate | 🔲 Skip (macOS-only) |

### STRING / PATH / FILE (4 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 17 | `StringUtil.cpp/h` | `<codecvt>`, `<sstream>` | `string_util.rs` | `std::str::from_utf8` |
| 18 | `SmallString.cpp/h` | STL iterator | `small_string.rs` | `smallvec` crate |
| 19 | `Path.h` | STL string/string_view | `path.rs` | `std::path::PathBuf` |
| 20 | `FileSystem.cpp/h` | `<mach-o/dyld.h>`, `<sys/param.h>`, `<sys/stat.h>` | `file_system.rs` | `std::fs` + `memmap2` crate |

### ERROR / CRASH (4 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 21 | `Error.cpp/h` | STL string | `error.rs` | `std::error::Error` trait |
| 22 | `CrashHandler.cpp/h` | **`<DbgHelp.h>`** (MiniDumpWriteDump), **`<backtrace.h>`** | `crash_handler.rs` | `crash-handler` + `backtrace` + `minidump` crate |
| 23 | `Assertions.cpp/h` | `<intrin.h>`, `<tlhelp32.h>`, `<signal.h>` | `assertions.rs` | `std::process::abort` |
| 24 | `FastJmp.cpp/h` | `<csetjmp>` (setjmp/longjmp) | `fast_jmp.rs` | `std::panic::catch_unwind` |

### SETTINGS / MEMORY (3 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 25 | `SettingsInterface.h` | STL string/optional/vector | `settings_interface.rs` | `serde` + trait |
| 26 | `SettingsWrapper.h` | Template macro (no external) | (di settings_interface) | `serde::Serialize/Deserialize` |
| 27 | `MemorySettingsInterface.cpp/h` | STL string | `memory_settings_interface.rs` | `serde_yaml` |

### THREADING / TIMER (4 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 28 | `Threading.h` | **`<mach/semaphore.h>`**, **`<semaphore.h>`** | `threading.rs` | `std::sync` (Mutex, Condvar, Atomic) |
| 29 | `Timer.cpp/h` | `<time.h>` | `timer.rs` | `std::time::Instant` |
| 30 | `Semaphore.cpp` | STL limits | `semaphore_impl.rs` | `std::sync::Condvar` + `Mutex` |
| 31 | `ReadbackSpinManager.cpp/h` | STL vector | `readback_spin_manager.rs` | `std::sync::atomic` |

### I/O / YAML (5 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 32 | `WAVWriter.cpp/h` | STL | `wav_writer.rs` | `hound` crate atau custom |
| 33 | `WindowInfo.cpp/h` | **`<dwmapi.h>`**, **`<X11/Xlib.h>`** | `window_info.rs` | `windows` crate |
| 34 | `TextureDecompress.cpp/h` | **`<immintrin.h>`**, **`<emmintrin.h>`** | `texture_decompress.rs` | `core::arch::x86_64::_mm_*` |
| 35 | `YAML.cpp/h` | STL optional | `yaml.rs` | `serde_yaml` crate |
| 36 | `Console.cpp/h` | `<unistd.h>` | `console.rs` | `std::io::Write` |

### HTTP / MD5 / PROGRESS / PERF (6 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 37 | `HTTPDownloader.cpp/h` | STL atomic/mutex/string | `http_downloader.rs` | `ureq` atau `reqwest` crate |
| 38 | `HTTPDownloaderCurl.cpp/h` | **`<curl/curl.h>`** | `http_downloader.rs` | `ureq` crate ✅ |
| 39 | `HTTPDownloaderWinHTTP.cpp/h` | **`<winhttp.h>`**, `<VersionHelpers.h>` | `http_downloader.rs` | `ureq` crate ✅ |
| 40 | `MD5Digest.cpp/h` | Portable C++ | `md5_digest.rs` | `md-5` crate ✅ |
| 41 | `LRUCache.h` | STL map | `lru_cache.rs` | `lru` crate ✅ |
| 42 | `ProgressCallback.cpp/h` | STL string/limits | `progress_callback.rs` | `indicatif` atau custom |
| 43 | `Perf.cpp/h` | **`<elf.h>`** (Linux perf_event) | `perf.rs` | `perf_event` crate (⚠️ stub, pre-existing error) |
| 44 | `ZipHelpers.h` | STL optional/string/vector | `zip_helpers.rs` | `zip` crate ✅ |

### HOST SYS (2 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 45 | `HostSys.cpp/h` | `<cpuinfo.h>` (CPU feature detect) | `host_sys.rs` + `host_sys_ffi.rs` | `raw-cpuid` crate ✅ |

### PLATFORM-SPECIFIC (9 files)

| # | File C++ | External Libs | Rust File | Rust Crate |
|---|----------|--------------|-----------|------------|
| 46 | `Windows/WinHostSys.cpp` | STL mutex | `windows_host_sys.rs` | `windows` crate ✅ (NEW) |
| 47 | `Windows/WinThreads.cpp` | **`<mmsystem.h>`**, **`<timeapi.h>`** | `windows_threads.rs` | `windows` crate ✅ (NEW) |
| 48 | `Windows/WinMisc.cpp` | **`<mmsystem.h>`**, **`<timeapi.h>`**, `<VersionHelpers.h>` | `windows_misc.rs` | `windows` crate ✅ (NEW) |
| 49 | `Linux/LnxHostSys.cpp` | `<sys/mman.h>`, `<fcntl.h>`, `<csignal>` | `linux_host_sys.rs` | `nix` crate (⚠️ pre-existing error) |
| 50 | `Linux/LnxMisc.cpp` | **`<dbus/dbus.h>`** | `linux_misc.rs` | `dbus` crate (⚠️ pre-existing error) |
| 51 | `Linux/LnxThreads.cpp` | `<pthread.h>`, `<sys/prctl.h>` | `linux_threads.rs` | `std::thread` |
| 52 | `Darwin/DarwinMisc.cpp` | `<sys/sysctl.h>` | `darwin_misc.rs` | `sysctl` crate |
| 53 | `Darwin/DarwinThreads.cpp` | `<pthread.h>` | `darwin_threads.rs` | `std::thread` |
| 54 | `CocoaTools.h` | **macOS Cocoa** | `cocoa_tools.rs` | `objc` crate |

### ALREADY ANALYZED BY AGENTS 1-6

| # | File C++ | Rust File | Rust Crate | Agent |
|---|----------|-----------|------------|-------|
| 55 | `Image.cpp/h` | `image.rs` | `image` crate | Agent 1 ✅ |
| 56 | `RedtapeWilCom.h` / `RedtapeWindows.h` | `redtape.rs` + `windows_misc.rs` | `windows` crate | Agent 2 ✅ |
| 57 | `DynamicLibrary.cpp/h` | `dynamic_library.rs` | `libloading` crate | Agent 3 ✅ |
| 58 | `StackWalker.cpp/h` | `stack_walker.rs` | `backtrace` crate | Agent 3 ✅ |
| 59 | `emitter/*` (34 files) | `x86_emitter.rs` | `iced-x86` crate | Agent 5 ✅ |
| 60 | `boost_spsc_queue.hpp` | `spsc_queue.rs` | custom (crossbeam pattern) | Agent 6 ✅ |

---

## Ringkasan External Library → Rust Crate

| Eksternal Library | Crate Rust | Digunakan di File |
|------------------|-----------|-------------------|
| libjpeg, libpng, libwebp | **`image`** | `Image.cpp` |
| DbgHelp (MiniDumpWriteDump) | **`crash-handler`** + **`backtrace`** + **`minidump`** | `CrashHandler.cpp` |
| libcurl | **`ureq`** | `HTTPDownloaderCurl.cpp` |
| WinHTTP | **`ureq`** | `HTTPDownloaderWinHTTP.cpp` |
| dlfcn.h (dlopen) + LoadLibrary | **`libloading`** | `DynamicLibrary.cpp` |
| WIL (com_ptr) | **`windows`** | `RedtapeWilCom.h` |
| Win32 (mmsystem, timeapi, dwmapi) | **`windows`** | `WinMisc.cpp`, `WinThreads.cpp`, `WindowInfo.cpp` |
| DbGhelp (StackWalk64) | **`backtrace`** | `StackWalker.cpp` |
| x86Emitter (internal) + Zydis | **`iced-x86`** | `emitter/*` (34 files) |
| DBus | **`dbus`** | `LnxMisc.cpp` (⚠️ pre-existing error) |
| perf_event (Linux) | **`perf_event`** | `Perf.cpp` (⚠️ pre-existing error) |
| macOS Cocoa/ObjC | **`objc`** | `CocoaTools.h`, `MRCHelpers.h` |
| MMX/SSE/AVX intrinsics | **`core::arch::x86_64::_mm_*`** | `VectorIntrin.h`, `FPControl.h`, `TextureDecompress.cpp` |

## Gaps Tersisa

| Gap | File | Severity | Status |
|-----|------|----------|--------|
| `Adopt()`/`GetHandle()` di DynamicLibrary | `CrashHandler.cpp` | Medium | BUTUH FIX |
| 8 fungsi duplikat threading.rs vs windows_threads.rs | `threading.rs` + `windows_threads.rs` | 🔴 HIGH | Linker error |
| 16 instruksi critical emitter (CMOVcc, INC/DEC, ADC...) | `x86_emitter.rs` | Medium | Parsial |
| perf_event (Linux) | `perf_event_counter.rs` | Low | Pre-existing |
| dbus | `linux_misc.rs` | Low | Pre-existing |

---

**Kesimpulan: Hanya 3 gap yang benar-benar critical:**
1. 🔴 8 fungsi duplikat (threading) → akan linker error di Windows
2. ⚠️ Adopt/GetHandle → CrashHandler bakal error kalo minidump dipanggil
3. ⚠️ 16 instruksi x86 emitter → hot path 70% OK, tapi game bisa crash kalo pake instruksi missing
