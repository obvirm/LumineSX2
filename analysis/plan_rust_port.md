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
| Modul | File C++ | Rust Crate | Prioritas | Status |
|-------|----------|-----------|-----------|--------|
| **CDVD Base** | `CDVD/CDVD.cpp` + `CDVDcommon.cpp` | custom | 1 | 🔲 belum (CDVD.cpp masih C++) |
| **ISO Reader** | `CDVD/IsoReader.cpp`, `FlatFileReader.cpp` | custom (`isofs.rs` + `iso_reader.rs`) | 1 | ✅ **SELESAI** (Rust 100%) |
| **ChdFileReader** | `CDVD/ChdFileReader.cpp` | `chd` crate | 2 | ✅ **SELESAI** (`chd_reader.rs`) |
| **CsoFileReader** | `CDVD/CsoFileReader.cpp` | `flate2` + `lz4_flex` | 2 | ✅ **SELESAI** (`cso_reader.rs`) |
| **GzippedFileReader** | `CDVD/GzippedFileReader.cpp` | `flate2` | 2 | ✅ **SELESAI** (`gz_reader.rs`) |
| **BlockdumpFileReader** | `CDVD/BlockdumpFileReader.cpp` | custom | 2 | ✅ **SELESAI** (`blockdump_reader.rs`) |
| **ThreadedFileReader** | `CDVD/ThreadedFileReader.cpp` | `std::thread` | 2 | 🔲 belum (tetap C++, by design) |
| **Game Database** | `GameDatabase.cpp` | `serde_yaml` | 2 | 🔲 |
| **Game List** | `GameList.cpp` | custom | 2 | 🔲 (UI scan ada di lib.rs) |

> **CDVD reader layer = 100% Rust** (commit `40f740a8f`). Semua format
> (.iso/.chd/.cso/.zso/.dump/.gz) dibuka via `CreateRustFileReader()` di
> `InputIsoFile.cpp`. C++ reader asli sudah lepas dari build. Sisa C++ di
> `pcsx2/CDVD/`: `CDVD.cpp` (disc control/command), `ThreadedFileReader`
> (threading, sengaja tetap C++), `Ps1CD.cpp`, `CDVDdiscThread`, `IOCtlSrc`.

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

---

## UI Porting Qt -> Slint (audit )

### Gap 1: Settings wiring (DONE 2026-07-13)
- **Sebelum**: 7 key aja yg tersimpan (renderer/fastboot/audio backend dll). Sisanya UI doang.
- **Sesudah**: 274 field settings di 15 halaman settings di-bind two-way ke 
  properties, lalu  tulis/ baca ke PCSX2 ini (paritas 1:1 dgn Qt).
- Pola:  parse  -> tambah prop + two-way binding
  di situs instansiasi (Slint global tdk diekspor oleh slint-build 1.16, jadi pakai
  MainWindow props + ).
- Test:  PASS (boot + emulation loop + settings round-trip).
- Windows-only di-skip (auto-updater, setup wizard, VC runtime, online cover download)
  karena target Android (Redmi).

### Gap 2: Dialogs
- [ ] About window (INFO:  di  - versi, kontributor)
- [ ] Log viewer ( - 11 channel)
- [ ] Hotkey settings ()
- [ ] Memory card create/convert dialogs

### Gap 3: Game list parity
- [x] Scan folder (iso/cso/chd/gz/bin/elf) + favorite + region filter
- [ ] Cover download (SKIP - online, Android offline)
- [ ] Drag-drop, context menu (rename/delete/properties)

---

## UI Porting Qt -> Slint (audit )

### Gap 1: Settings wiring � DONE
274 field ter-wiring ke core (paritas 1:1 Qt). Lihat commit sebelumnya.

### Gap 2: Dialogs � DONE (About + Log viewer)
-  (page 16): versi LumineSX2 + versi PCSX2 core (
  dari +) + link eksternal (buka browser via ).
-  (page 17): stream log core ke UI. Core log di-forward lewat
   ->  -> ring buffer Rust (, 2000 baris)
  -> dipoll 500ms ke . Tombol Bersihkan -> .
- C++ bridge:  +  di
  , di-build ulang jadi .
- Test: debug build OK, exe jalan,  PASS (no regression).

### Gap 2 lanjutan (DONE: Hotkey settings):
- [x] Hotkey settings (Tombol Pintasan) - parity dengan Qt HotkeySettingsWidget + InputBindingWidget. Page 21.
  - Enumerasi hotkey via pcsx2_get_hotkey_list() (InputManager::GetHotkeyList).
  - Baca/set/clear binding via pcsx2_get/set/clear_hotkey_binding() (INI section [Hotkeys], string-list).
  - Tangkap tombol: pcsx2_capture_hotkey_begin/poll/cancel() pakai InputInterceptHook internal.
  - C++ bridge (pcsx2_capi.{h,cpp}) + Rust API (pcsx2_capi.rs) + UI (hotkey_settings.slint) + wiring lib.rs.
  - Test: examples/hotkey_test.rs PASS - enumerate 64 hotkey, set/get/clear round-trip OK, capture begin/poll/cancel no panic.
  - Commit 0f89f3c6c (pushed).
- [ ] Memory card create/convert dialogs - nyambung ke SIO/Memcard/MemoryCardFile.

### Gap 2: Dialogs - DONE (About + Log viewer + Hotkey settings)
- About (page 16): versi LumineSX2 + versi PCSX2 core (pcsx2_get_version_string() dari BuildVersion::GitRev+GitDate) + link eksternal (buka browser via cmd start).
- Log viewer (page 17): stream log core ke UI. Core log di-forward lewat Console::SetHostOutputLevel -> cb_log -> ring buffer Rust (2000 baris) -> dipoll 500ms ke root.log-lines. Tombol Bersihkan -> clear_log_buffer().
- Hotkey settings (page 21): lihat Gap 2 lanjutan di atas.
- C++ bridge: pcsx2_register_log_callback() + pcsx2_get_version_string() + pcsx2_*_hotkey_*() di pcsx2_capi.{h,cpp}, di-build ulang jadi build/capi/pcsx2_capi.lib.
- Test: debug build OK, exe jalan, hotkey_test PASS (no regression).

### Gap 3: Game list parity
- [x] Scan folder (iso/cso/chd/gz/bin/elf) + favorite + region filter
- [ ] Cover download (SKIP - online, Android offline)
- [ ] Drag-drop, context menu (rename/delete/properties)
