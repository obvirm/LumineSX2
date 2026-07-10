# Rencana Porting Rust — Prioritas 1

## Target: Game bisa jalan di Slint UI (Windows + Vulkan)

---

## Modul & Urutan Porting

### Fase 1A: Foundation (dependencies dulu)
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **Memory management** | `Memory.cpp`, `vtlb.cpp`, `GSAlignedClass.h` | `std::alloc`, custom | 1 |
| **Config/Settings** | `Config.h`, `Pcsx2Config.cpp`, `INISettingsInterface.cpp` | `ini` crate | 1 |
| **State Wrapper** | `StateWrapper.cpp` | custom serialize | 1 |
| **Performance Metrics** | `PerformanceMetrics.cpp` | custom | 1 |

### Fase 1B: CPU Core (recompiler)
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **EE Recompiler** | `x86/ix86-32/iR5900.cpp` + tables | `iced-x86` | 1 |
| **VU Recompiler** | `x86/microVU.cpp` + microVU_*.inl | `iced-x86` | 1 |
| **IOP Recompiler** | `x86/iR3000A.cpp` + tables | `iced-x86` | 2 |
| **COP0/COP2/FPU** | `COP0.cpp`, `COP2.cpp`, `FPU.cpp` | custom | 2 |
| **Interpreter** | `Interpreter.cpp` | custom | 3 |
| **MMI** | `MMI.cpp` | custom | 3 |
| **IPU** | `IPU/` (12 file) | custom | 3 |

### Fase 1C: Graphics (Vulkan-only)
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **GS Base** | `GS/GS.cpp`, `GS.h` | `ash` | 1 |
| **Vulkan Renderer** | `GS/Renderers/Vulkan/*` (16 file) | `ash` + `gpu-allocator` | 1 |
| **HW Renderer** | `GS/Renderers/HW/*` (11 file) | custom | 1 |
| **GS State** | `GS/GSState.cpp` | custom | 1 |
| **GS Local Memory** | `GS/GSLocalMemory.cpp` | custom | 1 |
| **GS Vector** | `GS/GSVector*.cpp` | `wide` crate | 1 |
| **GS Util** | `GS/GSUtil.cpp` | custom | 2 |
| **Texture Cache** | `GS/GSTextureCache.cpp` | custom | 2 |
| **Png/Lzma/Zstd** | `GS/GSPng.cpp`, `GSLzma.cpp` | `image`, `xz2`, `zstd` | 3 |

### Fase 1D: Disc & CDVD
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **CDVD Base** | `CDVD/CDVD.cpp` + `CDVDcommon.cpp` | custom | 1 |
| **ISO Reader** | `CDVD/IsoReader.cpp`, `FlatFileReader.cpp` | custom | 1 |
| **ChdFileReader** | `CDVD/ChdFileReader.cpp` | `chd` crate | 2 |
| **CsoFileReader** | `CDVD/CsoFileReader.cpp` | `flate2` + `lz4_flex` | 2 |
| **ThreadedFileReader** | `CDVD/ThreadedFileReader.cpp` | `std::thread` | 2 |
| **Game Database** | `GameDatabase.cpp` | `serde_yaml` | 2 |
| **Game List** | `GameList.cpp` | custom | 2 |

### Fase 1E: Audio
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **Audio Stream** | `Host/AudioStream.cpp` | `cpal` + `rubato` | 1 |
| **Cubeb/SDL** | `Host/CubebAudioStream.cpp`, `SDLAudioStream.cpp` | `cpal` | 1 |
| **SPU2** | `SPU2/*` (19 file) | custom | 2 |

### Fase 1F: Input
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **Input Manager** | `Input/InputManager.cpp` | `gilrs` | 1 |
| **SDL Input** | `Input/SDLInputSource.cpp` | `gilrs` | 1 |
| **XInput/DInput** | `Input/XInputSource.cpp`, `DInputSource.cpp` | `gilrs` | 1 |
| **Pad** | `SIO/Pad/*` (15 file) | custom | 2 |

### Fase 1G: Core Loop
| Modul | File C++ | Rust Crate | Prioritas |
|-------|----------|-----------|-----------|
| **VMManager** | `VMManager.cpp` | custom | 1 |
| **Host** | `Host.cpp` | custom (host.rs) | 1 |
| **MTGS** | `MTGS.cpp` | custom | 1 |
| **MTVU** | `MTVU.cpp` | custom | 2 |

---

## Status Sekarang

### Sudah di-Rust
- `host.rs` — 2700 line Rust rewrite of QtHost.cpp ✅
- `pcsx2_capi.cpp` — C FFI bridge ✅
- `pcsx2_capi.rs` — Rust FFI bindings ✅
- `debug_backend.rs` + `debug_controller.rs` — debugger ✅
- `rust/common/` — sebagian common library ✅

### Masih C++
- Semua isi `pcsx2/pcsx2/` (core emulation)
- `pcsx2/GS/` (renderer)
- `pcsx2/CDVD/` (disc)
- DLL

---

## Strategi Porting

1. **Per modul, bukan per file**: Satu modul Rust = 1 package dengan beberapa file
2. **FFI bridge selama transisi**: Panggil C++ yang belum di-port via `extern "C"`
3. **Test tiap fase**: Game harus tetap jalan setelah tiap fase
4. **Prioritas render path**: Vulkan HW renderer dulu, SW renderer skip
