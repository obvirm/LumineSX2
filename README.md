# LumineSX2

> **PlayStation 2 Emulator — Android-First Rewrite**

[![Platform](https://img.shields.io/badge/platform-Windows%20(desktop)-orange?style=flat-square)]()
[![Platform](https://img.shields.io/badge/platform-Android%20(target)-green?style=flat-square)]()
[![Language](https://img.shields.io/badge/language-Rust%20+%20C++-blue?style=flat-square)]()
[![Status](https://img.shields.io/badge/status-EXPERIMENTAL%20-red?style=flat-square)]()


https://github.com/user-attachments/assets/aee8e5b8-e06a-4c46-a358-efdfcfdcffe0


---

## ⚠️ PERINGATAN EKSPERIMENTAL

**LumineSX2 masih dalam tahap pengembangan awal.**

- Saat ini **hanya bisa dijalankan di Windows (Desktop)**
- **Belum support Android** — ini adalah target utama
- Masih dalam proses **rewrite dari C++ ke Rust**
- **Belum stabil** — banyak fitur 

yang belum/tidak berfungsi

**JANGAN digunakan untuk bermain game secara production.**

---

## Visi & Misi

### Visi
Membangun emulator PlayStation 2 yang modern, ringan, dan portabel untuk platform **Android** (terutama device Redmi/Xiaomi), dengan arsitektur Rust yang aman dan performa tinggi.

### Misi
1. **Rewrite modul-modul inti PCSX2 ke Rust** — satu per satu, dimulai dari CDVD
2. **UI modern dengan Slint** — Material You dark theme, touch-friendly untuk mobile
3. **Vulkan renderer** — kompatibel dengan GPU mobile (ARM Mali, Adreno, dll)
4. **Android-first** — target utama adalah HP Redmi/Xiaomi dengan harga terjangkau
5. **Open source** — berbasis PCSX2 (GPLv3), dikembangkan secara transparan

---

## Status Porting ke Rust

| Modul | Status | Keterangan |
|-------|--------|------------|
| `common/` | ✅ Sebagian | StringUtil, SettingsWrapper, Threading |
| `CDVD/` | ✅ Parsial | ISO reader sudah jalan, CHD/CSO stubbed |
| `GS/` | ❌ Belum | Graphics synthesizer (prioritas rendah) |
| `SPU2/` | ❌ Belum | Audio processing |
| `PAD/` | ❌ Belum | Input/controller |
| `Core/EE` | ❌ Belum | Emotion Engine (prioritas rendah) |

---

## Build (Windows)

### Prerequisites
- [Rust](https://rustup.rs/) (stable toolchain)
- [Visual Studio 2022](https://visualstudio.microsoft.com/) (C++ build tools)
- [Vulkan SDK](https://vulkan.lunarg.com/) (untuk renderer)

### Build Commands

```powershell
# Build Rust UI + link PCSX2 core
cd lumine-sx2
cargo build --features pcsx2-core --release

# Output: target/release/lumine-sx2.exe
```

### Run

```powershell
# Boot game
.\target\release\lumine-sx2.exe --boot "E:\path\to\game.iso"

# With BIOS path
.\target\release\lumine-sx2.exe --bios "E:\path\to\bios" --boot "E:\path\to\game.iso"
```

---

## Arsitektur

```
┌─────────────────────────────────────┐
│         LumineSX2 (Rust UI)         │
│          Slint + Material You       │
├─────────────────────────────────────┤
│     C FFI Bridge (pcsx2_capi)       │
├─────────────────────────────────────┤
│      PCSX2 Core (C++ Static Lib)    │
│   VMManager | GS | SPU2 | PAD | EE  │
├─────────────────────────────────────┤
│     Rust Modules (gradually ported) │
│   CDVD ✅ | common ✅ | ...         │
└─────────────────────────────────────┘
```

---

## Perbedaan dengan PCSX2 Original

| Aspek | PCSX2 Original | LumineSX2 |
|-------|----------------|-----------|
| UI Framework | Qt6 | Slint (Rust) |
| Target Platform | Windows/Linux/Mac | Android (primary) |
| Bahasa UI | C++ | Rust |
| Renderer | Vulkan/D3D11/D3D12/GL | Vulkan (fokus) |
| Status | Stable | Experimental |

---

## Credits

LumineSX2 dibangun di atas [PCSX2](https://github.com/PCSX2/pcsx2) — emulator PS2 yang telah dikembangkan selama 20+ tahun. Terima kasih kepada seluruh kontributor PCSX2.

---

## License

[GNU General Public License v3.0](https://www.gnu.org/licenses/gpl-3.0.html) — mengikuti license PCSX2 upstream.
