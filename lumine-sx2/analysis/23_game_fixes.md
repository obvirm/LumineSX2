# Game Fix Settings Widget — Deep Analysis

**Source**: `pcsx2-qt/Settings/GameFixSettingsWidget.cpp` + `.h` + `.ui`

## Summary

Single `QGroupBox` named "Game Fixes" containing **18 QCheckBoxes**. All are boolean toggles bound to `EmuCore/Gamefixes` settings section. All default to `false` (unchecked). All are auto-applied per-game when configured via Game Properties.

## Complete Game Fix List (18 total)

### UI Order (from .ui tab order)

| # | Widget Name | Display Name | Setting Key | Default | Description / Affected Games |
|---|-------------|-------------|-------------|---------|------------------------------|
| 1 | `FpuMulHack` | FPU Multiply Hack | `FpuMulHack` | false | For **Tales of Destiny**. Fixes FPU multiplication issues. |
| 2 | `SoftwareRendererFMVHack` | Use Software Renderer For FMVs | `SoftwareRendererFMVHack` | false | Needed for games with complex FMV rendering. Switches to SW renderer during FMVs. |
| 3 | `SkipMPEGHack` | Skip MPEG Hack | `SkipMPEGHack` | false | Skips videos/FMVs to avoid game hanging/freezes. |
| 4 | `GoemonTlbHack` | Preload TLB Hack | `GoemonTlbHack` | false | To avoid TLB miss on **Goemon** (series). |
| 5 | `EETimingHack` | EE Timing Hack | `EETimingHack` | false | General-purpose timing hack. Affects: **Digital Devil Saga**, **SSX**. |
| 6 | `InstantDMAHack` | Instant DMA Hack | `InstantDMAHack` | false | Fixes cache emulation problems. Affects: **Fire Pro Wrestling Z**. |
| 7 | `OPHFlagHack` | OPH Flag Hack | `OPHFlagHack` | false | Output PatH flag in GIF_STAT register. Affects: **Bleach Blade Battlers**, **Growlanser II & III**, **Wizardry**. |
| 8 | `GIFFIFOHack` | Emulate GIF FIFO | `GIFFIFOHack` | false | GS Interface FIFO emulation. Correct but slower. Affects: **FIFA Street 2**. |
| 9 | `DMABusyHack` | DMA Busy Hack | `DMABusyHack` | false | Affects: **Mana Khemia 1**, **Metal Saga**, **Pilot Down Behind Enemy Lines**. |
| 10 | `VIF1StallHack` | Delay VIF1 Stalls | `VIF1StallHack` | false | VU Interface stall delay. For **SOCOM 2 HUD** and **Spy Hunter loading hang**. |
| 11 | `VIFFIFOHack` | Emulate VIF FIFO | `VIFFIFOHack` | false | Simulate VIF1 FIFO read ahead. Affects: **Test Drive Unlimited**, **Transformers**. |
| 12 | `FullVU0SyncHack` | Full VU0 Synchronization | `FullVU0SyncHack` | false | Forces tight VU0 sync on every COP2 instruction. |
| 13 | `IbitHack` | VU I Bit Hack | `IbitHack` | false | Avoids constant recompilation. Affects: **Scarface The World is Yours**, **Crash Tag Team Racing**. |
| 14 | `VuAddSubHack` | VU Add Hack | `VuAddSubHack` | false | For Tri-Ace games: **Star Ocean 3**, **Radiata Stories**, **Valkyrie Profile 2**. |
| 15 | `VUOverflowHack` | VU Overflow Hack | `VUOverflowHack` | false | Checks for float overflows. For **Superman Returns**. |
| 16 | `VUSyncHack` | VU Sync | `VUSyncHack` | false | Run VUs behind EE. Avoids sync problems when reading/writing VU registers. M-Bit game support. |
| 17 | `XgKickHack` | VU XGKick Sync | `XgKickHack` | false | Accurate timing for VU XGKick instructions (slower). |
| 18 | `BlitInternalFPSHack` | Force Blit Internal FPS Detection | `BlitInternalFPSHack` | false | Alternative method to calculate internal FPS via blit detection. Avoids false FPS readings. |

## Setting Section

All keys are under: **`EmuCore/Gamefixes`**

All are `bool` type with default `false`.

## Widget Details

- **Widget type**: `QCheckBox` (simple toggle)
- **Layout**: Single `QVBoxLayout` inside a `QGroupBox` titled "Game Fixes"
- **No grouping/category subdivisions** — all 18 are in one flat list
- **Tab order** matches the UI order above (1-18)
- **Help system**: Each checkbox has `registerWidgetHelp()` with title, default state, and description
- **No "reset to defaults" button** — relies on parent dialog's reset mechanism
- **No per-game override indicator** — visual feedback comes from the SettingsWindow framework

## Binding Pattern

```cpp
SettingWidgetBinder::BindWidgetToBoolSetting(sif, m_ui.WidgetName, "EmuCore/Gamefixes", "KeyName", false);
// All use: BindWidgetToBoolSetting, all default false
```

## Help Registration Pattern

```cpp
dialog()->registerWidgetHelp(m_ui.WidgetName, tr("Title"), tr("Unchecked"), tr("Description text"));
// "Unchecked" is always the default state string
```

## Technical Categories

### CPU/EE Hacks (2)
- **FpuMulHack** — FPU multiply fix (Tales of Destiny)
- **EETimingHack** — General EE timing adjustment (DDS, SSX)

### DMA/Transfer Hacks (4)
- **InstantDMAHack** — Cache emulation fix (Fire Pro Wrestling Z)
- **DMABusyHack** — DMA busy flag fix (Mana Khemia, Metal Saga)
- **GIFFIFOHack** — GIF FIFO emulation (FIFA Street 2)
- **BlitInternalFPSHack** — Blit-based FPS detection

### VU (Vector Unit) Hacks (7)
- **VuAddSubHack** — VU add/sub precision (Tri-Ace games)
- **IbitHack** — VU I-bit recompilation (Scarface, Crash Tag Team)
- **VUOverflowHack** — Float overflow check (Superman Returns)
- **VUSyncHack** — VU-EE sync delay (M-Bit games)
- **FullVU0SyncHack** — VU0 tight sync per COP2
- **XgKickHack** — XGKick accurate timing

### VIF (VU Interface) Hacks (2)
- **VIF1StallHack** — VIF1 stall delay (SOCOM 2, Spy Hunter)
- **VIFFIFOHack** — VIF1 FIFO read-ahead (TDU, Transformers)

### Video/Graphics Hacks (3)
- **SoftwareRendererFMVHack** — SW renderer for FMVs
- **SkipMPEGHack** — Skip MPEG/FMV playback
- **OPHFlagHack** — Output PatH flag (Bleach, Growlanser, Wizardry)

### Memory Hacks (1)
- **GoemonTlbHack** — TLB preload (Goemon series)

## Current LumineSX2 vs Qt: Gap Analysis

### Current LumineSX2 has 13 hacks (settings_graphics.slint lines 150-164)

| LumineSX2 Name | In Qt? | Notes |
|---|---|---|
| FPU Multiply Hack | ✅ | Correct |
| FPU Negative Div Hack | ❌ FABRICATED | Not in PCSX2 Qt. Doesn't exist in the codebase. |
| DivX Hack | ❌ FABRICATED | Not in PCSX2 Qt. Doesn't exist in the codebase. |
| XGKick Hack | ✅ | Correct (XgKickHack) |
| IPU Wait Hack | ❌ FABRICATED | Not in PCSX2 Qt. Doesn't exist in the codebase. |
| EE Timing Hack | ✅ | Correct (EETimingHack) |
| Skip MPEG Hack | ✅ | Correct (SkipMPEGHack) |
| OPH Flag Hack | ✅ | Correct (OPHFlagHack) |
| DMA Busy Hack | ✅ | Correct (DMABusyHack) |
| VIF FIFO Hack | ✅ | Correct (VIFFIFOHack) |
| VIF1 Stall Hack | ✅ | Correct (VIF1StallHack) |
| GIF FIFO Hack | ✅ | Correct (GIFFIFOHack) |
| Goemon TLB Hack | ✅ | Correct (GoemonTlbHack) |

### Missing from LumineSX2 (exist in Qt, 7 hacks)

| Qt Key | Display Name | Qt Description |
|---|---|---|
| `SoftwareRendererFMVHack` | Use Software Renderer For FMVs | SW renderer during FMVs |
| `InstantDMAHack` | Instant DMA Hack | Cache emulation fix |
| `VuAddSubHack` | VU Add Hack | Tri-Ace games (Star Ocean 3, VP2) |
| `IbitHack` | VU I Bit Hack | Avoids recompilation (Scarface) |
| `VUOverflowHack` | VU Overflow Hack | Float overflow check (Superman Returns) |
| `VUSyncHack` | VU Sync | VU-EE sync delay |
| `FullVU0SyncHack` | Full VU0 Sync | Tight VU0 sync per COP2 |
| `BlitInternalFPSHack` | Force Blit Internal FPS Detection | Blit-based FPS detection |

### Fabricated in LumineSX2 (3 hacks that DON'T EXIST in PCSX2)

| LumineSX2 Name | Reality |
|---|---|
| FPU Negative Div Hack | ❌ No such setting in PCSX2. Should be removed. |
| DivX Hack | ❌ No such setting in PCSX2. Should be removed. |
| IPU Wait Hack | ❌ No such setting in PCSX2. Should be removed. |

### Correct LumineSX2 should have 18 hacks, currently has 10 real + 3 fake = need to remove 3, add 8
