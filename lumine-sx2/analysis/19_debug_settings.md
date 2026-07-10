# PCSX2 Qt — Debug Settings Widget Analysis

## Files Analyzed
- `E:\project\pcsx2\pcsx2-qt\Settings\DebugSettingsWidget.h`
- `E:\project\pcsx2\pcsx2-qt\Settings\DebugSettingsWidget.cpp`

## Overview
Debug settings are organized into **4 sub-tabs** within the Debug settings page:
1. **User Interface** (hidden in per-game settings)
2. **Analysis**
3. **GS** (Graphics Synthesizer dumping)
4. **Logging** (only in PCSX2_DEVBUILD builds)

---

## Tab 1: User Interface

**Hidden in per-game settings** — the entire tab is hidden via `setTabVisible(m_user_interface_tab, false)` when `dialog()->isPerGameSettings()` is true.

| Widget | Setting Key | Type | Default | Description |
|--------|-------------|------|---------|-------------|
| `refreshInterval` | `Debugger/UserInterface/RefreshInterval` | int (QSpinBox) | 1000 | Milliseconds between UI updates reflecting VM state |
| `showOnStartup` | `Debugger/UserInterface/ShowOnStartup` | bool | false | Auto-open debugger window when PCSX2 starts |
| `saveWindowGeometry` | `Debugger/UserInterface/SaveWindowGeometry` | bool | true | Save/restore debugger window position and size on close/reopen |
| `dropIndicator` | `Debugger/UserInterface/DropIndicatorStyle` | enum (string) | "Classic" | Style of dock-window drag indicators. Options: "Classic", "Segmented", "Minimalistic". Requires restart to take effect |

**Hidden behavior**: `refreshInterval` value change triggers `g_debugger_window->updateFromSettings()` — immediately applies the new interval to the running debugger.

**Note**: `dropIndicator` is bound via `BindWidgetToEnumSetting` with a special `DebugUserInterfaceSettingsWidget` class prefix.

---

## Tab 2: Analysis

| Widget | Setting Key | Type | Default | Description |
|--------|-------------|------|---------|-------------|
| `analysisCondition` | `Debugger/Analysis/RunCondition` | enum | `IF_DEBUGGER_IS_OPEN` | When analysis passes run. Options from `Pcsx2Config::DebugAnalysisOptions::RunConditionNames`: Always / If Debugger Is Open / Never |
| `generateSymbolsForIRXExportTables` | `Debugger/Analysis/GenerateSymbolsForIRXExports` | bool | true | Hook IRX module loading/unloading and generate symbols for exported functions on the fly |
| `analysisSettings` | — | embedded widget | — | Container for `DebugAnalysisSettingsWidget` (separate class) — this is a nested settings panel, not a simple toggle |

**Architecture note**: The analysis settings widget is a separate `DebugAnalysisSettingsWidget` class embedded in a QVBoxLayout within the `m_analysis.analysisSettings` placeholder widget. The inner widget likely has its own setting bindings (from `DebugAnalysisSettingsWidget.h/cpp` — not read here).

---

## Tab 3: GS (Graphics Synthesizer Dumping)

**Master toggle**: `dumpGSData` at `EmuCore/GS/DumpGSData` (bool, default false). When unchecked, ALL other GS dump options are **disabled** via `onDrawDumpingChanged()`.

| Widget | Setting Key | Type | Default | Description |
|--------|-------------|------|---------|-------------|
| `dumpGSData` | `EmuCore/GS/DumpGSData` | bool | false | **Master toggle** for all GS data dumping |
| `saveRT` | `EmuCore/GS/SaveRT` | bool | false | Save Render Target data |
| `saveFrame` | `EmuCore/GS/SaveFrame` | bool | false | Save frame data |
| `saveTexture` | `EmuCore/GS/SaveTexture` | bool | false | Save texture data |
| `saveDepth` | `EmuCore/GS/SaveDepth` | bool | false | Save depth buffer data |
| `saveAlpha` | `EmuCore/GS/SaveAlpha` | bool | false | Save alpha channel data |
| `saveInfo` | `EmuCore/GS/SaveInfo` | bool | false | Save info data |
| `saveTransferImages` | `EmuCore/GS/SaveTransferImages` | bool | false | Save transfer images |
| `saveDrawStats` | `EmuCore/GS/SaveDrawStats` | bool | false | Save per-draw statistics |
| `saveFrameStats` | `EmuCore/GS/SaveFrameStats` | bool | false | Save per-frame statistics |
| `saveHWConfig` | `EmuCore/GS/SaveHWConfig` | bool | false | Save HW renderer configuration |
| `saveDrawStart` | `EmuCore/GS/SaveDrawStart` | int | 0 | Starting draw call index for dump range |
| `saveDrawCount` | `EmuCore/GS/SaveDrawCount` | int | 5000 | Number of draw calls to dump |
| `saveFrameStart` | `EmuCore/GS/SaveFrameStart` | int | 0 | Starting frame index for dump range |
| `saveFrameCount` | `EmuCore/GS/SaveFrameCount` | int | 999999 | Number of frames to dump |
| `hwDumpDirectory` | `EmuCore/GS/HWDumpDirectory` | folder path | (empty) | Directory for HW renderer dumps |
| `hwDumpBrowse` | — | button | — | Browse for HW dump directory |
| `hwDumpOpen` | — | button | — | Open HW dump directory in explorer |
| `swDumpDirectory` | `EmuCore/GS/SWDumpDirectory` | folder path | (empty) | Directory for SW renderer dumps |
| `swDumpBrowse` | — | button | — | Browse for SW dump directory |
| `swDumpOpen` | — | button | — | Open SW dump directory in explorer |

**Behavior**: The `onDrawDumpingChanged()` slot reads the effective value of `DumpGSData` and calls `setEnabled()` on every dump control — all 17 controls are disabled when dumping is off, enabled when on. This creates a clear UX where the entire dump section is grayed out unless the master toggle is on.

---

## Tab 4: Logging (DEVBUILD only)

**Only visible in `PCSX2_DEVBUILD` builds** — entire tab hidden in release builds.

**Master toggle**: `chkEnable` at `EmuCore/TraceLog/Enabled` (bool, default false). When unchecked, ALL logging options are disabled via `onLoggingEnableChanged()`. Enabling/disabling also triggers `g_emu_thread->applySettings()` immediately.

### EE (Emotion Engine) Trace Logging

| Widget | Setting Key | Description |
|--------|-------------|-------------|
| `chkEEBIOS` | `EE.bios` | Log SYSCALL and DECI2 activity |
| `chkEEMemory` | `EE.memory` | Log memory access to unknown/unmapped EE memory |
| `chkEER5900` | `EE.r5900` | Log R5900 core instructions (excl. COPs). Requires source modification + interpreter |
| `chkEECOP0` | `EE.cop0` | Log COP0 (MMU, CPU status) instructions |
| `chkEECOP1` | `EE.cop1` | Log COP1 (FPU) instructions |
| `chkEECOP2` | `EE.cop2` | Log COP2 (VU0 Macro mode) instructions |
| `chkEECache` | `EE.cache` | Log EE cache activity |
| `chkEEMMIO` | `EE.knownhw` | Log known MMIO accesses |
| `chkEEUNKNWNMMIO` | `EE.unknownhw` | Log unknown/unimplemented MMIO accesses |
| `chkEEDMARegs` | `EE.dmahw` | Log DMA-related MMIO accesses |
| `chkEEIPU` | `EE.ipu` | Log IPU activity (MMIO, decoding, DMA status) |
| `chkEEGIFTags` | `EE.giftag` | Log GIFtag parsing activity |
| `chkEEVIFCodes` | `EE.vifcode` | Log VIFcode processing (commands, tag style, interrupts) |
| `chkEEMSKPATH3` | `EE.mskpath3` | Log Path3 Masking processing |
| `chkEEMFIFO` | `EE.spr` | Log Scratchpad MFIFO activity |
| `chkEEDMACTRL` | `EE.dmac` | Log DMA transfer activity (stalls, bus arbitration) |
| `chkEECounters` | `EE.counters` | Log EE counter events and register activity |
| `chkEEVIF` | `EE.vif` | Log various VIF/VIFcode processing data |
| `chkEEGIF` | `EE.gif` | Log various GIF/GIFtag parsing data |

### IOP (I/O Processor) Trace Logging

| Widget | Setting Key | Description |
|--------|-------------|-------------|
| `chkIOPBIOS` | `IOP.Bios` | Log SYSCALL and IRX activity |
| `chkIOPMemcards` | `IOP.memcards` | Log memory card activity (reads, writes, erases) |
| `chkIOPR3000A` | `IOP.r3000a` | Log R3000A core instructions (excl. COPs) |
| `chkIOPCOP2` | `IOP.cop2` | Log IOP GPU co-processor instructions |
| `chkIOPMMIO` | `IOP.knownhw` | Log known MMIO accesses |
| `chkIOPUNKNWNMMIO` | `IOP.unknownhw` | Log unknown/unimplemented MMIO accesses |
| `chkIOPDMARegs` | `IOP.dmahw` | Log DMA-related MMIO accesses |
| `chkIOPPad` | `IOP.pad` | Log PAD activity |
| `chkIOPDMACTRL` | `IOP.dmac` | Log DMA transfer activity |
| `chkIOPCounters` | `IOP.counters` | Log IOP counter events and register activity |
| `chkIOPCDVD` | `IOP.cdvd` | Log CDVD hardware activity |
| `chkIOPMDEC` | `IOP.mdec` | Log Motion (FMV) Decoder hardware unit activity |

### MISC

| Widget | Setting Key | Description |
|--------|-------------|-------------|
| `chkEESIF` | `MISC.sif` | Log SIF (EE ↔ IOP) activity |

---

## Key Architectural Patterns

### Conditional Enable/Disable Pattern
Both GS dump and Logging use a master-toggle pattern where:
1. A master `QCheckBox` is bound to a bool setting
2. A `connect()` to `checkStateChanged` triggers a slot
3. The slot reads the effective value and calls `setEnabled()` on all child controls
4. This creates a cascading enable/disable UI

### Per-Game Settings Handling
- User Interface tab is **completely hidden** in per-game settings (`setTabVisible(false)`)
- Analysis, GS, and Logging tabs remain visible for per-game override

### Settings Organization
All settings use the `SettingWidgetBinder` system:
- `BindWidgetToBoolSetting` → bool toggles
- `BindWidgetToIntSetting` → integer spinboxes
- `BindWidgetToEnumSetting` → enum combo boxes with string mapping arrays
- `BindWidgetToFolderSetting` → folder path with browse/open buttons

### Conditional Compilation
The Logging tab is **only compiled in `PCSX2_DEVBUILD` builds**. In release builds, `m_logging_tab` is hidden via `setTabVisible(false)`.

---

## UI Elements Not Exposed in Current Slint UI

All of these are completely missing from our current `settings_debug.slint`:
1. **4 sub-tabs** (UI, Analysis, GS, Logging)
2. **Refresh interval** (integer spinbox for debugger update rate)
3. **Show on startup** toggle
4. **Save window geometry** toggle
5. **Drop indicator style** combo (Classic/Segmented/Minimalistic)
6. **Analysis run condition** combo (Always/If Open/Never)
7. **IRX symbol generation** toggle
8. **Analysis settings** embedded panel
9. **Master GS dump toggle** with cascading enable/disable
10. **20+ individual GS dump toggles** (RT, Frame, Texture, Depth, Alpha, Info, Transfer, DrawStats, FrameStats, HWConfig)
11. **Draw/Frame range controls** (start/count pairs)
12. **HW/SW dump directory** pickers with browse+open buttons
13. **Master logging toggle** with cascading enable
14. **30+ individual trace log toggles** (19 EE + 12 IOP + 1 MISC)
