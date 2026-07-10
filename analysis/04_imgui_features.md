# Analisis External Library Calls — ImGui & Features (4 files)

Dari: `pcsx2/pcsx2/ImGui/ImGuiManager.cpp`, `ImGuiFullscreen.cpp`, `VMManager.cpp`, `SaveState.cpp`

---

## 1. ImGuiManager.cpp (1442 baris)

### External Libraries Dipanggil

| Library | Include | Fungsi/Calls |
|---------|---------|-------------|
| **imgui** (3rdparty) | `<imgui.h>`, `<imgui_internal.h>` | `ImGui::CreateContext()`, `ImGui::DestroyContext()`, `ImGui::GetIO()`, `ImGui::NewFrame()`, `ImGui::EndFrame()`, `ImGui::GetStyle()`, `ImGui::GetBackgroundDrawList()`, `ImGui::GetForegroundDrawList()`, `ImGui::GetCurrentContext()`, `ImGui::GetCurrentWindowRead()`, `ImGui::GetPlatformIO()`, `ImGui::GetIO().Fonts->AddFontFromMemoryTTF()`, `ImGui::GetIO().AddInputCharactersUTF8()`, `ImGui::GetIO().AddMouseButtonEvent()`, `ImGui::GetIO().AddMouseWheelEvent()`, `ImGui::GetIO().AddKeyAnalogEvent()`, `ImGuiFreeTypeLoaderFlags_LoadColor`, dll. |
| **imgui_freetype** (3rdparty) | `<imgui_freetype.h>` | `ImGuiFreeType::BuildFontAtlas()` — font rasterizer via FreeType |
| **FreeType2** | `<ft2build.h>`, `FT_FREETYPE_H`, `FT_MODULE_H` | `FT_Init_FreeType()`, `FT_New_Memory_Face()`, `FT_Done_Face()`, `FT_Err_Ok` — font loading engine |
| **fmt** (3rdparty) | `<fmt/format.h>` | `fmt::format()` — string formatting |
| **common/Image** | `"common/Image.h"` | `RGBA8Image` type — image buffer |
| **common internal** | `"common/FileSystem.h"`, `"common/Console.h"`, `"common/StringUtil.h"`, `"common/Path.h"`, `"common/Timer.h"` | Utility functions |

### Fungsi Utama
- `SetFonts()` — init FreeType, load TTF fonts, build ImGui font atlas
- `NewFrame()` / `EndFrame()` / `RenderOSD()` — ImGui frame lifecycle
- Input handling: mouse, keyboard, gamepad → ImGui IO

### Ketergantungan
- **imgui** ~410 KB (C++) — rendering + event system
- **imgui_freetype** — font rasterizer plugin
- **FreeType2** ~1 MB — font loading
- **fmt** — formatting
- GSDevice untuk render backend

### Rust Equivalent
- Slint built-in — no need for imgui
- fontdb + rustybuzz — font loading & shaping
- `std::fmt` — built-in formatting

---

## 2. ImGuiFullscreen.cpp (3568 baris)

### External Libraries Dipanggil

| Library | Include | Fungsi/Calls |
|---------|---------|-------------|
| **imgui** (3rdparty) | `<imgui_internal.h>`, `<imgui_stdlib.h>` | `ImGui::PushStyleVar()`, `PopStyleVar()`, `GetIO()`, GetDisplaySize, dll — ribuan calls untuk fullscreen UI |
| **plutosvg** (3rdparty) | `<plutosvg.h>` | `plutosvg_document_load_from_data()`, `plutosvg_document_get_width()`, `plutosvg_document_get_height()`, `plutosvg_document_render()`, `plutosvg_document_destroy()` — SVG parsing & rendering |
| **plutovg** (3rdparty) | `<plutovg.h>` | `plutovg_surface_create_for_data()`, `plutovg_canvas_create()`, `plutovg_canvas_scale()`, `plutovg_canvas_translate()`, `plutovg_canvas_destroy()`, `plutovg_surface_destroy()`, `plutovg_convert_argb_to_rgba()` — 2D vector rendering |
| **fmt** (3rdparty) | `<fmt/format.h>` | `fmt::format()` |
| **common internal** | `common/Image.h`, `common/Console.h`, `common/Easing.h`, `common/LRUCache.h`, `common/FileSystem.h`, `common/Path.h` | Utility (image buffer, cache, filesystem) |

### Fungsi Utama
- `LoadSvgTextureImage()` — load SVG → render to raster via plutosvg + plutovg → RGBA8Image
- `LoadTextureImage()` — load PNG/JPG → RGBA8Image
- `UploadTexture()` — upload raster image → GSTexture (GPU)
- Fullscreen UI (game list, achievements, settings, dll)
- Texture cache, SVG data cache

### Ketergantungan
- **plutosvg** — pure C SVG parser/renderer (~200KB)
- **plutovg** — pure C 2D vector renderer (~100KB)
- **imgui** — UI framework
- GSTexture (via GSDevice)

### Rust Equivalent
- **usvg** crate + **tiny-skia** crate → ganti plutosvg + plutovg
- Slitn built-in Image → ganti GSTexture
- No need for imgui fullscreen (Slitn handles UI)

---

## 3. VMManager.cpp (3845 baris)

### External Libraries Dipanggil

| Library | Include | Fungsi/Calls |
|---------|---------|-------------|
| **fmt** (3rdparty) | `<fmt/format.h>` | `fmt::format()` — ~60+ calls untuk string formatting log messages, error messages, paths |
| **cpuinfo** (3rdparty) | `"cpuinfo.h"` | `cpuinfo_initialize()`, `cpuinfo_has_x86_sse4_1()`, `cpuinfo_has_x86_avx2()`, `cpuinfo_has_arm_neon()` — CPU feature detection |
| **discord-rpc** (3rdparty) | `"discord_rpc.h"` | `Discord_Initialize()`, `Discord_Shutdown()`, `Discord_UpdatePresence()`, `Discord_ClearPresence()`, `Discord_RunCallbacks()` — Discord Rich Presence |
| **IconsFontAwesome** | `"IconsFontAwesome.h"` | `ICON_FA_*` — icon constants untuk OSD messages |
| **common internal** | `common/*.h` | `Path::Combine()`, `Console.WriteLn()`, `FileSystem`, `StringUtil`, `Threading`, `Timer`, `SettingsWrapper`, `Error`, `RedtapeWilCom.h` |
| **Win32** (conditional) | `<objbase.h>`, `<timeapi.h>`, `<powrprof.h>`, `<wil/com.h>`, `<dxgi.h>` | `CoInitializeEx()`, `timeBeginPeriod()`, `SetThreadExecutionState()`, WIL COM wrappers, DXGI |

### Fungsi Utama (External)
- `PerformEarlyHardwareChecks()` — `cpuinfo_initialize()`, SSE4.1/AVX2 check
- `EnsureCPUInfoInitialized()` — `cpuinfo_initialize()`
- `InitializeDiscordPresence()` — `Discord_Initialize()`, setup handlers
- `ShutdownDiscordPresence()` — `Discord_ClearPresence()`, `Discord_Shutdown()`
- `PollDiscordPresence()` — `Discord_RunCallbacks()`
- `UpdateDiscordPresence()` — `Discord_UpdatePresence()` dengan status game
- Ribuan `fmt::format()` calls untuk logging dan OSD

### Ketergantungan
- **cpuinfo** — ~100KB, cross-platform CPU detection
- **discord-rpc** — ~200KB, Discord SDK
- **fmt** — string formatting
- **Win32 WIL** — Windows COM wrappers

### Rust Equivalent
- `cpuinfo` → `raw-cpuid` crate + `is_x86_feature_detected!()` macro
- `discord-rpc` → `discord-rich-presence` crate
- `fmt` → `std::fmt`
- WIL → `windows` crate

---

## 4. SaveState.cpp (1211 baris)

### External Libraries Dipanggil

| Library | Include | Fungsi/Calls |
|---------|---------|-------------|
| **libpng** (3rdparty) | `<png.h>` | `png_create_write_struct()`, `png_create_info_struct()`, `png_destroy_write_struct()`, `png_set_write_fn()`, `png_set_compression_level()`, `png_set_IHDR()`, `png_write_info()`, `png_write_row()`, `png_write_end()`, `png_create_read_struct()`, `png_set_read_fn()`, `png_read_info()`, `png_read_image()`, `png_destroy_read_struct()`, `png_jmpbuf()` — screenshot save/load |
| **libzip** (3rdparty) | `"common/ZipHelpers.h"` (wrapper) | `zip_open()`, `zip_close()`, `zip_source_write()`, `zip_*` — save state compression (ZIP container) |
| **ZSTD** (via libzip) | indirect | `ZIP_CM_ZSTD` — Zstandard compression method via libzip |
| **fmt** (3rdparty) | `<fmt/format.h>` | `fmt::format()` — ~20+ error messages |
| **common internal** | `common/Error.h`, `common/FileSystem.h`, `common/Path.h`, `common/StringUtil.h`, `common/ZipHelpers.h` | Utility |
| **IconsFontAwesome** | `"IconsFontAwesome.h"` | Icon constants |

### Fungsi Utama (External)
- `SaveState_CompressScreenshot()` — `png_create_write_struct()`, `png_write_*` — encode RGBA → PNG
- `SaveState_DecompressScreenshot()` — `png_create_read_struct()`, `png_read_*` — decode PNG → RGBA
- Zip save state — `zip_source_write()`, `zip_set_file_compression()`, `ZIP_CM_ZSTD`
- Load/save state dengan ArchiveEntryList

### Ketergantungan
- **libpng** ~200KB — PNG encoding/decoding
- **libzip** ~100KB — ZIP container format
- **zstd** (via libzip) — compression
- **fmt** — formatting

### Rust Equivalent
- `png` crate — pure Rust PNG encode/decode
- `zip` crate — pure Rust ZIP handling
- `flate2` / `zstd` crate — compression

---

## Ringkasan Dependency Graph

```
ImGuiManager.cpp
  ├── imgui (C++) ───→ Slint (Rust)
  ├── imgui_freetype ──→ fontdb + rustybuzz
  ├── FreeType2 ───────→ rustybuzz (ttf-parser)
  └── fmt ─────────────→ std::fmt

ImGuiFullscreen.cpp
  ├── imgui ───────────→ Slint
  ├── plutosvg ────────→ usvg
  ├── plutovg ─────────→ tiny-skia
  └── fmt ─────────────→ std::fmt

VMManager.cpp
  ├── cpuinfo ─────────→ raw-cpuid
  ├── discord-rpc ─────→ discord-rich-presence
  ├── Win32 WIL ───────→ windows crate
  └── fmt ─────────────→ std::fmt

SaveState.cpp
  ├── libpng ──────────→ png crate
  ├── libzip + zstd ───→ zip crate + zstd crate
  └── fmt ─────────────→ std::fmt
```

## Kompleksitas Porting

| File | LOC | External Deps | Rust Equiv | Kesulitan |
|------|-----|---------------|------------|-----------|
| ImGuiManager.cpp | ~1442 | 4 libs | Slint built-in | **Rendah** — Slint handle UI native |
| ImGuiFullscreen.cpp | ~3568 | 4 libs | Slint + usvg + tiny-skia | **Sedang** — perlu rewrite UI logic ke Slint |
| VMManager.cpp | ~3845 | 3 libs + Win32 | raw-cpuid + discord crate | **Tinggi** — logika emulasi kompleks, bukan library replacement |
| SaveState.cpp | ~1211 | 3 libs | png + zip crate | **Rendah** — straightforward library swap |

**Total external library calls:** ~100+ distinct API calls dari 10+ library eksternal.

### Catatan Arsitektur
- **VMManager.cpp** is the HEART of the emulator — 3845 lines of core emulation logic with relatively few external deps (mostly cpuinfo + discord + fmt). Porting this = porting the emulator itself.
- **ImGuiFullscreen.cpp** has the most external SVG/image calls via plutosvg + plutovg. In Rust/Slint, Slint's native SVG support replaces this entirely.
- **SaveState.cpp** is clean — straightforward library swap.

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "Analyzed all 4 files: ImGuiManager.cpp (1442 LOC, 4 external libs), ImGuiFullscreen.cpp (3568 LOC, 4 external libs), VMManager.cpp (3845 LOC, 3 external libs + Win32), SaveState.cpp (1211 LOC, 3 external libs). Found ~100+ external API calls across 10+ libraries."
    }
  ],
  "changedFiles": [
    "E:\\project\\pcsx2\\analysis\\04_imgui_features.md"
  ],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "grep + read tools on 4 source files",
      "result": "passed",
      "summary": "Extracted all #include directives and external API calls for each file"
    }
  ],
  "validationOutput": [
    "Report written to E:\\project\\pcsx2\\analysis\\04_imgui_features.md"
  ],
  "residualRisks": [
    "VMManager.cpp is 3845 lines of core emulation logic — library deps are simple but the PORTING of the logic itself is the hard part",
    "ImGuiFullscreen.cpp has 3568 LOC of fullscreen UI code that needs Slint rewrite, not just library swap"
  ],
  "noStagedFiles": true,
  "diffSummary": "Created analysis report for 4 files covering imgui, freetype, plutosvg, plutovg, cpuinfo, discord-rpc, libpng, libzip, zstd, fmt external calls",
  "reviewFindings": [
    "no blockers: All external dependencies have Rust equivalents identified"
  ],
  "manualNotes": "VMManager.cpp is the priority target for Rust port — it's the emulator core. SaveState.cpp is easiest to port (just swap png/zip/zstd crates). ImGui UI files get replaced by Slint, no need to port."
}
```
