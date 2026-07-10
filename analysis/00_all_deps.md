# Analisis Komprehensif External Dependencies — `pcsx2/pcsx2/`

> Dihasilkan: 2026-06-28
> Lingkup: Semua file `.cpp` / `.h` di `E:\project\pcsx2\pcsx2\` + `common/` (yang di-link)
> Metode: Grep `#include`, API calls, COM, platform-specific, embedded third-party code

---

## 📊 Ringkasan

| Kategori | Jumlah |
|----------|--------|
| Header eksternal (`<...>`) | ~80 unique |
| Quoted include ke eksternal | ~30 |
| COM interfaces digunakan | 40+ |
| Win32 API calls | 60+ |
| POSIX/Linux API calls | 20+ |
| Embedded third-party files | ~20 file |
| **Total library eksternal (unik)** | **~50+** |

---

## 1. THIRD-PARTY LIBRARIES (dari 3rdparty/)

| # | Library | Lokasi di 3rdparty/ | Dipakai di pcsx2/ | Rust Crate |
|---|---------|--------------------|-------------------|------------|
| 1 | **Vulkan SDK** | `3rdparty/vulkan/` | `GS/Renderers/Vulkan/*` (17 file) | `ash` |
| 2 | **VMA** | `3rdparty/include/vk_mem_alloc.h` | `VKLoader.h` | `gpu-allocator` |
| 3 | **shaderc** | `3rdparty/shaderc/` | `VKShaderCache.cpp` | `shaderc-sys` |
| 4 | **D3D11** | Windows SDK | `GS/Renderers/DX11/*` (skip) | skip |
| 5 | **D3D12** | Windows SDK | `GS/Renderers/DX12/*` (skip) | skip |
| 6 | **OpenGL + glad** | `3rdparty/glad/` | `GS/Renderers/OpenGL/*` (skip) | skip |
| 7 | **FFmpeg** | `3rdparty/ffmpeg/` | `GS/GSCapture.cpp` | `ffmpeg-next` / skip |
| 8 | **libpng** | vcpkg | `GSPng.cpp`, `SaveState.cpp`, `GSTextureReplacementLoaders.cpp`, `common/Image.cpp` | `image` / `png` |
| 9 | **libjpeg** | vcpkg | `common/Image.cpp`, `GSTextureReplacementLoaders.cpp` | `image` / `jpeg` |
| 10 | **libwebp** | vcpkg | `common/Image.cpp` | `image` / `webp` |
| 11 | **zlib** | vcpkg | `CDVD/CsoFileReader.cpp`, `GSUtil.cpp`, `GSPng.cpp`, `iR3000A.cpp` (debug), `VMManager.cpp` | `flate2` |
| 12 | **lz4** | vcpkg | `CDVD/CsoFileReader.cpp` | `lz4_flex` |
| 13 | **LZMA SDK** (7z) | vcpkg | `GS/GSLzma.cpp`, `GS/GSDump.cpp` | `xz2` / `lzma-rs` |
| 14 | **zstd** | vcpkg | `GS/GSLzma.cpp`, `GS/GSDump.cpp`, `SaveState.cpp` | `zstd` |
| 15 | **libchdr** | `3rdparty/libchdr/` | `CDVD/ChdFileReader.cpp` | `chd` crate |
| 16 | **cpuinfo** | `3rdparty/cpuinfo/` | `GS/MultiISA.cpp`, `VMManager.cpp` | `raw-cpuid` |
| 17 | **imgui** | `3rdparty/imgui/` | `ImGui/*`, `GSDevice.cpp` (common/) | Slint built-in |
| 18 | **imgui_freetype** | `3rdparty/imgui/` | `ImGuiManager.cpp` | Slint built-in |
| 19 | **FreeType** | vcpkg | `ImGuiManager.cpp` (via imgui_freetype) | `fontdb` + `rustybuzz` |
| 20 | **plutosvg** | `3rdparty/plutosvg/` | `ImGuiFullscreen.cpp` | `usvg` |
| 21 | **plutovg** | `3rdparty/plutovg/` | `ImGuiFullscreen.cpp` | `tiny-skia` |
| 22 | **fmt** | `3rdparty/fmt/` | **65+ file** (hampir semua .cpp) | `std::fmt` |
| 23 | **SoundTouch** | `3rdparty/soundtouch/` | `Host/AudioStream.cpp` | `rubato` |
| 24 | **FreeSurround** | `3rdparty/freesurround/` | `Host/AudioStream.cpp` | `dasp` / custom |
| 25 | **cubeb** | `3rdparty/cubeb/` | `Host/CubebAudioStream.cpp`, `USB/usb-mic/audiodev-cubeb.cpp` | `cpal` |
| 26 | **SDL3** | `3rdparty/SDL/` | `Input/SDLInputSource.*`, `Host/SDLAudioStream.cpp` | `gilrs` + `cpal` |
| 27 | **discord-rpc** | `3rdparty/discord-rpc/` | `VMManager.cpp` | `discord-rich-presence` |
| 28 | **rcheevos** | `3rdparty/rcheevos/` | `Achievements.cpp` | `rcheevos` (Rust bind) |
| 29 | **libzip** | `3rdparty/libzip/` | `SaveState.cpp` (via common/ZipHelpers.h) | `zip` crate |
| 30 | **rapidyaml (ryml)** | vcpkg (`find_package`) | `GameDatabase.cpp`, `SIO/Memcard/MemoryCardFolder.cpp` | `serde_yaml` / `yaml-rust` |
| 31 | **xbyak** | `3rdparty/xbyak/` | `GS/Renderers/SW/*` (codegen) | `iced-x86` |
| 32 | **x86Emitter** | `common/emitter/` (internal) | `x86/iR3000A.cpp`, `iR5900.cpp`, `microVU.cpp`, `iFPU.cpp` | `iced-x86` |
| 33 | **vixl** | `3rdparty/vixl/` | `arm64/AsmHelpers.*` | `iced-x86` |
| 34 | **Zydis** | `3rdparty/zydis/` | `x86/iR3000A.cpp` (#ifdef DUMP_BLOCKS) | `iced-x86` |
| 35 | **ccc** | `3rdparty/ccc/` | `DebugTools/SymbolImporter.cpp`, `SymbolGuardian.h` | `goblin` (ELF) |
| 36 | **demangle** | `3rdparty/demangler/` | `DebugTools/SymbolImporter.cpp`, `SymbolGuardian.h` | `rustc-demangle` |
| 37 | **libpcap** | vcpkg / OS | `DEV9/pcap_io.cpp`, `Win32/pcap_io_win32.cpp` | `pcap` crate |
| 38 | **libbacktrace** | vcpkg (optional) | `common/` | `backtrace` crate |
| 39 | **xxhash** | `common/xxhash.h` | `GS/GSXXH.cpp` | `xxhash-rust` |
| 40 | **XInput** | Windows SDK | `Input/XInputSource.*` | `gilrs` |
| 41 | **DirectInput** | Windows SDK | `Input/DInputSource.*` | `gilrs` |
| 42 | **DirectShow** | Windows SDK | `USB/usb-eyetoy/cam-windows.cpp` | skip (USB item) |
| 43 | **WIL** (Windows Implement. Lib) | `3rdparty/winwil/` | `GS/DX11/DX12/*`, `Input/DInputSource.cpp` | `windows` crate |

## 2. EMBEDDED THIRD-PARTY CODE (di dalam pcsx2/pcsx2/)

| Kode | Lokasi | File | Baris | Catatan |
|------|--------|------|-------|---------|
| **QEMU USB** | `USB/qemu-usb/` | 12 file: `bus.cpp core.cpp desc.cpp desc.h hid.cpp hid.h input-keymap*.cpp queue.h qusb.h usb-ohci.cpp USBinternal.h` | ~3K | Fork dari QEMU untuk USB controller emulation |
| **jo_mpeg** | `USB/usb-eyetoy/jo_mpeg.cpp` | 1 file | ~300 | Minimal MPEG1 encoder |
| **cpuinfo** | via `#include "cpuinfo.h"` | 1 header | ~100K | Sebenarnya dari 3rdparty/cpuinfo/ |
| **xxhash** | via `#include "xxhash.h"` | 1 header | ~5K | Public domain hash |
| **vk_mem_alloc.h** | via `#include "vk_mem_alloc.h"` | 1 header + 1 .cpp | ~15K | Vulkan Memory Allocator |
| **zlib_indexed.h** | `CDVD/zlib_indexed.h` | 1 header | ~300 | Random access zlib |

## 3. COMMON LIBRARY EXTERNAL DEPS (di-link via `common.lib`)

| Library | CMake | Dipakai oleh pcsx2/ |
|---------|-------|---------------------|
| **ryml::ryml** (rapidyaml) | `find_package(ryml REQUIRED)` | GameDatabase, MemoryCardFolder |
| **libbacktrace** | optional | Error handling |
| **Vtune** | optional | Profiling |
| **winhttp** / **curl** | via `common/HTTPDownloader.*` | Achievements (download) |
| **ntdll** (Win32) | system | Kernel calls |
| **ws2_32** (Win32) | system | Sockets |
| **bcrypt** (Win32) | system | Cryptography |
| **advapi32** (Win32) | system | Registry/security |
| **userenv** (Win32) | system | User environment |
| **version** (Win32) | system | Version info |
| **fmt** | re-exported | String formatting |

## 4. PLATFORM-SPECIFIC API CALLS

### 4.1 Win32 API — COM

| Interface/API | File | Fungsi |
|---------------|------|--------|
| `ID3D11Device`, `ID3D11DeviceContext` | DX11/* | D3D11 rendering (skip) |
| `ID3D12Device`, `ID3D12GraphicsCommandList` | DX12/* | D3D12 rendering (skip) |
| `IDXGIFactory1/5`, `IDXGIAdapter1`, `IDXGISwapChain1` | DX11/*, DX12/* | DXGI swap chains (skip) |
| `IDirectInput8W`, `IDirectInputDevice8W` | Input/DInputSource.* | DirectInput controllers |
| `INetCfg*` | DEV9/Win32/tap-win32.cpp | Network configuration |
| `IMediaControl`, `ISampleGrabberCB` | USB/usb-eyetoy/cam-windows.cpp | DirectShow webcam |
| `IID_IDirectInput8W` | Input/DInputSource.* | DirectInput GUID |
| `CoCreateInstance` + `CLSID_*` | DEV9, USB/eyetoy | COM object creation |
| `CoInitializeEx` / `CoUninitialize` | CubebAudioStream, tap-win32, pcsx2_capi.cpp | COM initialization |

### 4.2 Win32 API — Dynamic Loading

| API | File | Fungsi |
|-----|------|--------|
| `LoadLibrary("wpcap.dll")` | DEV9/Win32/pcap_io_win32.cpp | Load npcap/wpcap |
| `GetProcAddress` | DEV9, DX12, GLContextWGL | Resolve function pointer |
| `GetModuleHandle("d3d12.dll")` | GSDevice12.cpp | Check D3D12 availability |
| `dlopen`/`dlsym`/`dlclose` | GLContextEGLWayland.cpp | Wayland EGL loading |

### 4.3 Win32 API — Registry

| API | File | Fungsi |
|-----|------|--------|
| `RegOpenKeyEx` | DEV9/tap-win32.cpp, D3D.cpp | Network adapter, DirectX version |
| `RegQueryValueEx` | DEV9/tap-win32.cpp, D3D.cpp | Adapter config, driver version |
| `RegGetValueW` | D3D.cpp | DirectX adapter LUID |

### 4.4 Win32 API — File/IO

| API | File | Fungsi |
|-----|------|--------|
| `CreateFileA` / `CreateFileW` | CDVD/Windows/*, DEV9/* | CD/DVD drive access, TAP device |
| `DeviceIoControl` | CDVD/Windows/*, DEV9/* | CD-ROM IOCTL, TAP IOCTL |
| `ReadFile` / `WriteFile` | DEV9/*, CDVD/* | Async I/O |
| `SetFilePointerEx` / `SetEndOfFile` | ATA/HddCreate.cpp | Sparse HDD image creation |
| `GetFileSizeEx` / `FindFirstFile` | Various | File enumeration |

### 4.5 Win32 API — Threading/Sync

| API | File | Fungsi |
|-----|------|--------|
| `CreateThread` | MTGS.cpp, various | Worker threads |
| `CreateEvent` / `SetEvent` / `ResetEvent` | CDVD/*, MTGS/* | Event synchronization |
| `WaitForSingleObject` / `WaitForMultipleObjects` | CDVD/*, GS/* | Thread sync |
| `InitializeCriticalSection` | Various | Critical sections |
| `SetThreadPriority` / `SetThreadAffinityMask` | MTGS/*, MTVU/* | Thread affinity |

### 4.6 Win32 API — Memory

| API | File | Fungsi |
|-----|------|--------|
| `VirtualAlloc` / `VirtualFree` | Memory.cpp, vtlb.cpp | Large memory allocations |
| `_aligned_malloc` / `_aligned_free` | GS/*.cpp, Gif_Unit.h | SIMD-aligned allocations |
| `HeapAlloc` / `HeapFree` | Various | Heap management |

### 4.7 Win32 API — Windows/Message

| API | File | Fungsi |
|-----|------|--------|
| `CreateWindowEx` | pcsx2_capi.cpp | GS HWND window |
| `ShowWindow` / `SetWindowPos` | pcsx2_capi.cpp | Window visibility |
| `PeekMessage` / `GetMessage` / `DispatchMessage` | pcsx2_capi.cpp, Host.cpp | Message pump |
| `GetClientRect` | pcsx2_capi.cpp | Window size |
| `PostMessage` | pcsx2_capi.cpp | Inter-thread messaging |

### 4.8 SIMD / CPU Intrinsics (Compiler-dependent)

| API | File | Fungsi |
|-----|------|--------|
| `_mm_*` (SSE) | GS/*, x86/*, common/* | SIMD vector math |
| `_mm256_*` (AVX2) | GS/*, x86/* | AVX2 operations |
| `_mm512_*` (AVX-512) | GS/* (conditional) | AVX-512 ops |
| `__cpuid` / `__cpuidex` | GS/*, VMManager.cpp | CPU feature detection |
| `_Interlocked*` | GS/*, MTGS/* | Atomic operations |
| `__has_builtin` | GS/GSVector4i_arm64.h | Clang builtin check |

### 4.9 Linux / POSIX API

| API | File | Fungsi |
|-----|------|--------|
| `ioctl` | CDVD/Linux/*, DEV9/Linux/* | CD-ROM control, TAP config |
| `mmap` / `munmap` | Memory.cpp, vtlb.cpp | Large memory maps |
| `pthread_create` / `pthread_mutex_*` | Various | Threading (Linux) |
| `sched_setaffinity` | MTVU.cpp | Thread affinity (Linux) |
| `clock_gettime` | PerformanceMetrics.cpp | High-res timing |
| `AF_UNIX` socket | PINE.cpp | PINE debugger protocol |
| `dlopen` / `dlsym` / `dlclose` | GLContextEGLWayland.cpp | Wayland EGL |
| `getifaddrs` / `freeifaddrs` | DEV9/Linux/* | Network interface list |

### 4.10 macOS / Apple API

| API | File | Fungsi |
|-----|------|--------|
| `<AppKit/AppKit.h>` | (via cmake) | Window system |
| `<Metal/Metal.h>` | GS/Renderers/Metal/* | Metal rendering |
| `<IOKit/*>` | CDVD/Darwin/* | CD/DVD access |
| `<CoreFoundation/*>` | Various | System services |

## 5. C++ STANDARD LIBRARY FEATURES DIGUNAKAN

| Feature | Penggunaan |
|---------|------------|
| `std::thread` | CDVD threading, debug import, DEV9 I/O |
| `std::mutex` / `std::lock_guard` | Synchronization everywhere |
| `std::shared_mutex` | Read-write locks |
| `std::atomic` | Lock-free state, refcounts |
| `std::optional` | Return values (C++17) |
| `std::variant` | Type-safe unions |
| `std::string_view` | String references (C++17) |
| `std::span` | Array views (C++20) |
| `std::chrono::steady_clock` | Timing (DEV9) |
| `std::function` | Callbacks |
| `std::shared_ptr` / `std::unique_ptr` | Smart pointers |
| `std::filesystem` | **TIDAK** dipakai (pake Path::Combine dll) |
| `std::regex` | **TIDAK** dipakai (manual parsing) |
| `std::random` | **TIDAK** dipakai |

## 6. MASTER TABLE — SEMUA EXTERNAL DEPENDENCY

| # | Library | Tipe | Rust Crate | Kritikal? | Complexity |
|---|---------|------|-----------|-----------|------------|
| 1 | **Vulkan SDK** | GPU API | `ash` | ✅ YA (renderer) | 🔴 Tinggi |
| 2 | **VMA** | GPU memory | `gpu-allocator` | ✅ YA | 🟡 Sedang |
| 3 | **shaderc** | Shader compile | `shaderc-sys` / naga | ✅ YA | 🟡 Sedang |
| 4 | **D3D11** | GPU (skip) | skip | ❌ Skip | 🟢 skip |
| 5 | **D3D12** | GPU (skip) | skip | ❌ Skip | 🟢 skip |
| 6 | **OpenGL** | GPU (skip) | skip | ❌ Skip | 🟢 skip |
| 7 | **FFmpeg** | Video cap | `ffmpeg-next` | ❌ Optional | 🟡 Sedang |
| 8 | **libpng** | Image I/O | `image` / `png` | ✅ YA (savestate) | 🟢 Rendah |
| 9 | **libjpeg** | Image I/O | `image` / `jpeg` | ❌ Optional | 🟢 Rendah |
| 10 | **libwebp** | Image I/O | `image` / `webp` | ❌ Optional | 🟢 Rendah |
| 11 | **zlib** | Compress | `flate2` | ✅ YA (CDVD) | 🟢 Rendah |
| 12 | **lz4** | Compress | `lz4_flex` | ❌ Minor | 🟢 Rendah |
| 13 | **LZMA SDK** | Compress | `xz2` / `lzma-rs` | ❌ Minor | 🟢 Rendah |
| 14 | **zstd** | Compress | `zstd` | ❌ Minor | 🟢 Rendah |
| 15 | **libchdr** | Disc format | `chd` | ❌ Minor | 🟡 Sedang |
| 16 | **cpuinfo** | CPU detect | `raw-cpuid` | ✅ YA | 🟢 Rendah |
| 17 | **imgui** | Debug UI | Slint | ✅ YA (ganti) | 🟢 Rendah |
| 18 | **FreeType** | Fonts | `fontdb`+`rustybuzz` | ✅ YA | 🟢 Rendah |
| 19 | **plutosvg** | SVG | `usvg` | ✅ YA | 🟢 Rendah |
| 20 | **plutovg** | 2D vector | `tiny-skia` | ✅ YA | 🟢 Rendah |
| 21 | **fmt** | Format | `std::fmt` | ✅ YA (built-in) | 🟢 Rendah |
| 22 | **SoundTouch** | Audio DSP | `rubato` | ❌ Minor | 🟡 Sedang |
| 23 | **FreeSurround** | Audio DSP | `dasp` / custom | ❌ Minor | 🟡 Sedang |
| 24 | **cubeb** | Audio out | `cpal` | ✅ YA | 🟢 Rendah |
| 25 | **SDL3** | Input/Audio | `gilrs`+`cpal` | ✅ YA | 🟡 Sedang |
| 26 | **discord-rpc** | Discord | `discord-rich-presence` | ❌ Optional | 🟢 Rendah |
| 27 | **rcheevos** | Achieve | `rcheevos` (bind) | ❌ Optional | 🟡 Sedang |
| 28 | **libzip** | ZIP | `zip` crate | ❌ Minor | 🟢 Rendah |
| 29 | **rapidyaml** | YAML | `serde_yaml` | ✅ YA (GameIndex) | 🟢 Rendah |
| 30 | **x86Emitter** | JIT codegen | `iced-x86` | 🔴 KRITIKAL | 🔴 Tinggi |
| 31 | **xbyak** | JIT codegen | `iced-x86` | ❌ Minor (SW) | 🟡 Sedang |
| 32 | **vixl** | ARM64 JIT | `iced-x86` | ❌ ARM64 only | 🟡 Sedang |
| 33 | **Zydis** | Disasm | `iced-x86` | ❌ Debug only | 🟢 Rendah |
| 34 | **ccc** | ELF/analysis | `goblin` | ❌ Debug only | 🟡 Sedang |
| 35 | **demangle** | Demangle | `rustc-demangle` | ❌ Debug only | 🟢 Rendah |
| 36 | **libpcap** | Network | `pcap` crate | ❌ DEV9 only | 🟡 Sedang |
| 37 | **xxhash** | Hashing | `xxhash-rust` | ❌ Minor | 🟢 Rendah |
| 38 | **XInput** | Controller | `gilrs` | ✅ YA | 🟢 Rendah |
| 39 | **DirectInput** | Controller | `gilrs` | ✅ YA | 🟢 Rendah |
| 40 | **WIL** | Win32 COM | `windows` crate | ✅ YA | 🟢 Rendah |
| 41 | **libbacktrace** | Backtrace | `backtrace` crate | ❌ Optional | 🟢 Rendah |
| 42 | **Vtune** | Profiling | skip | ❌ Optional | 🟢 skip |
| 43 | **~~QEMU USB~~** | USB emu | skip/custom | ❌ Skip (USB) | 🟡 Sedang |
| 44 | **~~jo_mpeg~~** | MPEG enc | skip | ❌ Skip (USB) | 🟢 Rendah |
| 45 | **WinHTTP** | HTTP | `reqwest` | ✅ YA (Achieve) | 🟢 Rendah |
| 46 | **fmt/ranges** | Format | `std::fmt` | ❌ Minor | 🟢 Rendah |
| 47 | **fmt/chrono** | Format | `std::fmt` | ❌ Minor | 🟢 Rendah |
| 48 | **glad** | GL loader | skip | ❌ Skip | 🟢 skip |
| 49 | **D3D12MemAlloc** | GPU mem | skip | ❌ Skip | 🟢 skip |
| 50 | **WinPixEventRuntime** | GPU debug | skip | ❌ Optional | 🟢 skip |
| 51 | **DirectShow** | Webcam | `windows` crate | ❌ Skip (USB) | 🟡 Sedang |
| 52 | **ntdll/ws2_32/bcrypt/...** | Win32 system | `windows` crate | ✅ YA | 🟢 Rendah |

## 7. TOTAL BARIS KODE EKSTERNAL (estimasi)

| Library | Perkiraan LOC |
|---------|--------------|
| Vulkan SDK headers | ~500K (not compiled, just declarations) |
| imgui | ~150K |
| fmt | ~50K |
| cpuinfo | ~100K |
| vixl | ~200K |
| xbyak | ~50K |
| zydis | ~200K |
| ccc | ~100K |
| libchdr | ~50K |
| **Total compiled third-party** | **~900K LOC+** |
| **PCSX2 core (pcsx2/pcsx2/ saja)** | **~150K LOC** |

## 8. KESIMPULAN UNTUK RUST PORT

### Modul KRITIKAL (wajib di-port)
1. **Vulkan Renderer** → 17 file, 150+ API calls via `ash`
2. **JIT/Recompiler** → x86Emitter + xbyak → `iced-x86`
3. **VMManager** → 3845 line core logic (sedikit external deps)
4. **CDVD** → zlib + chd, port dengan `flate2` + `chd`
5. **Audio** → cubeb/SDL/SoundTouch → `cpal` + `rubato`
6. **Input** → XInput/DInput/SDL → `gilrs`
7. **YAML** → rapidyaml → `serde_yaml`
8. **HTTP** → WinHTTP → `reqwest`

### Modul BISA SKIP (porting tahap 2+)
- D3D11/D3D12/OpenGL renderer → Vulkan-only
- GS Software Renderer (xbyak) → pake HW renderer
- FFmpeg capture → tambah nanti
- DEV9 network → optional
- USB devices → optional
- Achievements → optional
- Discord → optional

### Library BUILT-IN Rust (no external crate needed)
- fmt → `std::fmt`
- threading → `std::thread`
- memory → `std::alloc`
- filesystem → `std::fs`, `std::path`
- chrono → `std::time`
- C++ exception handling → Rust panic/Result

### Rust CRATES yang dibutuhkan

```
# Wajib
ash, gpu-allocator, naga (shader), 
winapi / windows (Win32 FFI),
gilrs (input), cpal (audio),
iced-x86 (JIT/disassembler),
serde_yaml (GameIndex.yaml),
reqwest (HTTP),
flate2, zstd, lz4_flex (compression),
image / png (textures),
usvg + tiny-skia (SVG/vector),
fontdb + rustybuzz (fonts),
backtrace,
xxhash-rust,
zip

# Optional
discord-rich-presence,
rcheevos,
pcap,
rubato (audio DSP),
chd (disc format),
goblin (ELF parsing),
rustc-demangle
```

**Total ~25 crate wajib + ~7 optional**
