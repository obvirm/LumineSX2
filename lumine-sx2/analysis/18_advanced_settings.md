# Advanced Settings - Deep Analysis

Source: `pcsx2-qt/Settings/AdvancedSettingsWidget.h`, `AdvancedSettingsWidget.cpp`, `AdvancedSettingsWidget.ui`

## Overview
Advanced system-level settings organized into 6 major sections. These are the "power user" knobs that can break games if misconfigured. The page starts with a disclaimer warning.

---

## Section 1: EmotionEngine (MIPS-IV)

### Combo Boxes
| Widget | Setting Path | Key | Options | Default |
|--------|-------------|-----|---------|---------|
| eeRoundingMode | EmuCore/CPU | FPU.Roundmode | Nearest, Negative, Positive, Chop/Zero (Default) | Chop/Zero |
| eeDivRoundingMode | EmuCore/CPU | FPUDiv.Roundmode | Nearest (Default), Negative, Positive, Chop/Zero | Nearest |
| eeClampMode | EmuCore/CPU/Recompiler | fpuOverflow, fpuExtraOverflow, fpuFullMode | None, Normal (Default), Extra + Preserve Sign, Full | Normal |

### Checkboxes
| Widget | Setting Path | Key | Default | Description |
|--------|-------------|-----|---------|-------------|
| eeRecompiler | EmuCore/CPU/Recompiler | EnableEE | true | JIT binary translation MIPS-IV → x86 |
| eeCache | EmuCore/CPU/Recompiler | EnableEECache | false | Interpreter-only, diagnostic use |
| eeINTCSpinDetection | EmuCore/Speedhacks | IntcStat | true | Huge speedup, no side effects |
| eeWaitLoopDetection | EmuCore/Speedhacks | WaitLoop | true | Moderate speedup, no side effects |
| eeFastmem | EmuCore/CPU/Recompiler | EnableFastmem | true | Backpatching to avoid register flushing |
| pauseOnTLBMiss | EmuCore/CPU/Recompiler | PauseOnTLBMiss | false | Pauses VM on TLB miss instead of ignoring |
| extraMemory | EmuCore/CPU | ExtraMemory | false | Exposes 128MB EE + 8MB IOP RAM (Dev Console) |

### Hidden: Clamping Mode Implementation
Clamping mode is NOT a simple int — it's stored as 3 separate booleans:
- `fpuOverflow` (None→Normal boundary)
- `fpuExtraOverflow` (Normal→Extra boundary)
- `fpuFullMode` (Extra→Full boundary)

Per-game settings insert a "Use Global Setting" option at index 0.

---

## Section 2: Vector Units (VU)

### Combo Boxes
| Widget | Setting Path | Keys | Options | Default |
|--------|-------------|------|---------|---------|
| vu0RoundingMode | EmuCore/CPU | VU0.Roundmode | Nearest, Negative, Positive, Chop/Zero (Default) | Chop/Zero |
| vu0ClampMode | EmuCore/CPU/Recompiler | vu0Overflow, vu0ExtraOverflow, vu0SignOverflow | None, Normal (Default), Extra, Extra + Preserve Sign | Normal |
| vu1RoundingMode | EmuCore/CPU | VU1.Roundmode | Nearest, Negative, Positive, Chop/Zero (Default) | Chop/Zero |
| vu1ClampMode | EmuCore/CPU/Recompiler | vu1Overflow, vu1ExtraOverflow, vu1SignOverflow | None, Normal (Default), Extra, Extra + Preserve Sign | Normal |

### Checkboxes
| Widget | Setting Path | Key | Default | Description |
|--------|-------------|-----|---------|-------------|
| vu0Recompiler | EmuCore/CPU/Recompiler | EnableVU0 | true | VU0 Micro Mode recompiler |
| vu1Recompiler | EmuCore/CPU/Recompiler | EnableVU1 | true | VU1 recompiler |
| vuFlagHack | EmuCore/Speedhacks | vuFlagHack | true | mVU flag hack - good speedup |
| instantVU1 | EmuCore/Speedhacks | vu1Instant | true | Runs VU1 instantly |

### Hidden: VU Clamping Same 3-Bool Pattern
Same as EE clamping — each VU unit uses 3 booleans to encode 4 levels.

---

## Section 3: I/O Processor (IOP, MIPS-I)

| Widget | Setting Path | Key | Default | Description |
|--------|-------------|-----|---------|-------------|
| iopRecompiler | EmuCore/CPU/Recompiler | EnableIOP | true | JIT binary translation MIPS-I → x86 |

---

## Section 4: Game Settings

| Widget | Setting Path | Key | Default | Description |
|--------|-------------|-----|---------|-------------|
| gameFixes | EmuCore | EnableGameFixes | true | Auto-loads fixes for known problematic games |
| patches | EmuCore | EnablePatches | true | Auto-loads compatibility patches |

---

## Section 5: Savestate Settings

### Combo Boxes
| Widget | Setting Path | Key | Options | Default |
|--------|-------------|-----|---------|---------|
| savestateCompressionMethod | EmuCore | SavestateCompressionType | Uncompressed, Deflate, Zstandard | Zstandard |
| savestateCompressionLevel | EmuCore | SavestateCompressionRatio | Low (Fast), Medium (Recommended), High, Very High (Slow, Not Recommended) | Medium |

### Checkboxes
| Widget | Setting Path | Key | Default | Description |
|--------|-------------|-----|---------|-------------|
| backupSaveStates | EmuCore | BackupSavestate | true | Creates .backup copy of existing savestate |
| saveStateOnShutdown | EmuCore | SaveStateOnShutdown | false | Auto-saves state when powering down |

### Hidden: Compression Level Disabled When Uncompressed
`onSavestateCompressionTypeChanged()` disables `savestateCompressionLevel` when method is Uncompressed.

### Hidden: SavestateCompressionMethod Enum
```cpp
SavestateCompressionMethod::Uncompressed  // = 0
SavestateCompressionMethod::Deflate       // = 1
SavestateCompressionMethod::Zstandard     // = 2
```

### Hidden: SavestateCompressionLevel Enum
```cpp
SavestateCompressionLevel::Low    // = 0
SavestateCompressionLevel::Medium // = 1
SavestateCompressionLevel::High   // = 2
SavestateCompressionLevel::VeryHigh // = 3
```

---

## Section 6: PINE Settings

| Widget | Setting Path | Key | Default | Description |
|--------|-------------|-----|---------|-------------|
| pineEnable | EmuCore | EnablePINE | false | Enable PINE IPC server |
| pineSlot | EmuCore | PINESlot | 28011 | IPC socket port number |

### Hidden: What is PINE?
PINE (PCSX2 Inter-process Networking Environment) is an IPC mechanism that allows external tools to communicate with the running emulator (read/write memory, controller input, etc.). Used by tools like Cheat Engine integrations and automation scripts.

---

## Missing from Current LumineSX2 Implementation

| Feature | Status | Notes |
|---------|--------|-------|
| EE Recompiler toggle | ❌ Missing | Critical - disabling breaks most games |
| EE Cache toggle | ❌ Missing | Diagnostic only |
| INTC Spin Detection | ❌ Missing | Important speedhack |
| Wait Loop Detection | ❌ Missing | Important speedhack |
| Fast Memory Access | ❌ Missing | Performance feature |
| Pause on TLB Miss | ❌ Missing | Debug feature |
| Extended RAM (Dev Console) | ❌ Missing | Debug/dev feature |
| EE Division Rounding Mode | ❌ Missing | Different from regular rounding |
| VU0 Recompiler toggle | ❌ Missing | Critical |
| VU1 Recompiler toggle | ❌ Missing | Critical |
| mVU Flag Hack | ❌ Missing | Speedhack |
| Instant VU1 | ❌ Missing | Speedhack |
| IOP Recompiler toggle | ❌ Missing | Critical |
| Game Fixes toggle | ❌ Missing | Already in GameFixesSettingsView as detailed toggles |
| Patches toggle | ❌ Missing | Already in PatchesSettingsView |
| Savestate Compression Method | ❌ Missing | 3 options: Uncompressed/Deflate/Zstandard |
| Savestate Compression Level | ❌ Missing | 4 levels with dynamic disable |
| Backup Savestates | ❌ Missing | .backup copies |
| Save State On Shutdown | ❌ Missing | Auto-save on exit |
| PINE Enable/Slot | ❌ Missing | IPC for external tools |
| Disclaimer label | ❌ Missing | Warning text at top |
| Per-game "Use Global Setting" override | ❌ Missing | Per-game settings pattern |

### What Current LumineSX2 Has (Partially)
- EE/VU Clamping Mode: ✅ Has 4 options but WRONG implementation (uses single int, real PCSX2 uses 3 bools)
- EE/VU Rounding Mode: ✅ Has 4 options matching PCSX2
- Savestate Compression: ❌ Only has generic toggle, not method+level
- Host Filesystem toggle: ❌ Present in LumineSX2 but not in real PCSX2 AdvancedSettings (this is a real PCSX2 setting but lives elsewhere)
- Extra Memory / Extended RAM: ❌ Missing

---

## Settings Binding Architecture
All settings use `SettingWidgetBinder` which automatically:
1. Reads current value from INI/game settings on widget creation
2. Connects widget signals to write back to settings
3. Handles per-game "use global setting" override pattern
4. Supports `SettingsInterface` for both global and per-game config

The `dialog()->registerWidgetHelp()` calls provide tooltip/hover help text for each setting.
