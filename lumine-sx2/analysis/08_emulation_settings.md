# Emulation Settings — Deep Analysis

## Source Files
- `pcsx2-qt/Settings/EmulationSettingsWidget.h`
- `pcsx2-qt/Settings/EmulationSettingsWidget.cpp`

---

## 1. Speed Controls

### 1.1 Normal Speed (`Framerate/NominalScalar`, default: `1.0f`)
- Preset speeds: 2%, 10%, 25%, 50%, 75%, 90%, 100%, 110%, 120%, 150%, 175%, 200%, 300%, 400%, 500%, 1000%
- Each preset shows NTSC FPS (×60) and PAL FPS (×50) in the label
- **"Unlimited"** option (value `0.0f`)
- **"Custom"** option — opens `QInputDialog::getDouble` (range 0–5000%, 1 decimal)
- Per-game mode: first item is "Use Global Setting [X%]" (reads global value for display)

### 1.2 Fast-Forward Speed (`Framerate/TurboScalar`, default: `2.0f`)
- Same combo structure as Normal Speed
- Same presets, unlimited, custom

### 1.3 Slow-Motion Speed (`Framerate/SlomoScalar`, default: `0.5f`)
- Same combo structure as Normal Speed
- Same presets, unlimited, custom

### 1.4 Key Implementation Detail
- `initializeSpeedCombo()` builds the combo; `handleSpeedComboChange()` handles selection
- Custom speed dialog: 0–5000%, 1 decimal place
- Cancel on custom dialog reverts to previous value via `QSignalBlocker`

---

## 2. Frame Pacing & Latency

### 2.1 Optimal Frame Pacing (`EmuCore/GS/VsyncQueueSize`)
- Checkbox (tristate in per-game mode: Checked/Unchecked/PartiallyChecked)
- When checked → sets `VsyncQueueSize = 0` (every frame completed before next begins)
- When unchecked → sets `VsyncQueueSize = 2` (DEFAULT_FRAME_LATENCY)
- **Disables** Max Frame Latency spinbox when enabled

### 2.2 Maximum Frame Latency
- Integer spinbox bound to `EmuCore/GS/VsyncQueueSize`
- Default: 2 frames
- Minimum: 0 (when optimal pacing enabled) or 1 (otherwise)
- Disabled when optimal frame pacing is checked

### 2.3 Skip Presenting Duplicate Frames (`EmuCore/GS/SkipDuplicateFrames`, default: `true`)
- Detects idle frames in 25/30fps games and skips presenting (NOT frame skipping)
- Frame still rendered — GPU gets more time
- Smooths frame times near max CPU/GPU utilization
- Helps with frame generation on 25/30fps games
- Can increase input lag

---

## 3. VSync & Host Sync

### 3.1 VSync (`EmuCore/GS/VsyncEnable`, default: `false`)
- Auto-disabled when not running at 100% speed

### 3.2 Sync to Host Refresh Rate (`EmuCore/GS/SyncToHostRefreshRate`, default: `false`)
- Speeds up emulation so guest refresh matches host
- Smoothest animations, potentially <1% speed increase
- Won't take effect if console refresh too far from host
- VRR users should disable

### 3.3 Use Host VSync Timing (`EmuCore/GS/UseVSyncForTiming`, default: `false`)
- **Only enabled when BOTH VSync AND Sync to Host Refresh Rate are enabled**
- Disables PCSX2's internal frame timing, uses host instead
- Smoother frame pacing but **increased input latency**

---

## 4. EE Speedhacks

### 4.1 EE Cycle Rate (`EmuCore/Speedhacks/EECycleRate`, default: `0`)
- Range: -3 to +3
- Higher values = higher internal framerate, more CPU required
- Lower values = less CPU load, lightweight games run full speed
- Per-game mode: first item "Use Global Setting [X%]"

### 4.2 EE Cycle Skipping (`EmuCore/Speedhacks/EECycleSkip`, default: `0`)
- Integer binding
- Makes emulated EE skip cycles
- Helps small subset of games (e.g., Shadow of the Colossus)
- Usually harmful to performance

---

## 5. Threading & I/O

### 5.1 MTVU — Multithreaded VU1 (`EmuCore/Speedhacks/vuThread`, default: `false`)
- Speedup on 4+ core CPUs
- Safe for most games, a few incompatible (may hang)

### 5.2 Thread Pinning (`EmuCore/EnableThreadPinning`, default: `false`)
- Pins specific threads to specific cores
- Helps big.LITTLE CPUs (Intel 12th gen+, AMD)
- Ignores system scheduler

### 5.3 Fast CDVD (`EmuCore/Speedhacks/fastCDVD`, default: `false`)
- Faster disc access, shorter loading times
- **Per-game only** (hidden in global settings)
- Check HDLoader compatibility lists

### 5.4 CDVD Precaching (`EmuCore/CdvdPrecache`, default: `false`)
- Loads disc image into RAM before VM start
- Reduces stutter on slow HDDs
- Significantly increases boot times

---

## 6. Real-Time Clock (Per-Game Only)

### 6.1 Manually Set Real-Time Clock (`EmuCore/ManuallySetRealTimeClock`, default: `false`)
- Checkbox — enables/disables the RTC date picker and locale format toggle
- Only visible in per-game settings (entire `rtcGroup` hidden in global mode)

### 6.2 RTC DateTime Picker
- Binds to 6 separate keys: `EmuCore/RtcYear`, `RtcMonth`, `RtcDay`, `RtcHour`, `RtcMinute`, `RtcSecond`
- Date range: 2000-01-01 to 2099-12-31
- Only applied on PS2 boot; in-game changes have no effect
- Some games require RTC date after their release date

### 6.3 Use System Locale Format (`EmuCore/UseSystemLocaleFormat`, default: `false`)
- Toggle between `yyyy-MM-dd HH:mm:ss` and OS locale short format
- May exclude seconds in locale format

---

## 7. Other

### 7.1 Host Filesystem (`EmuCore/HostFs`, default: `false`)
- Allows games/homebrew to access host filesystem directly

### 7.2 Enable Cheats (`EmuCore/EnableCheats`, default: `false`)
- Auto-loads and applies cheats on game start
- **Global settings only** (hidden in per-game mode — use Cheats panel instead)

---

## 8. Widget Help Texts (Tooltip System)

All settings register help via `dialog()->registerWidgetHelp()` with title, recommended value, and description. This is a **hidden feature** — the help system provides contextual tooltips that appear when hovering over the "?" icon next to each setting.

---

## 9. Per-Game Settings Behavior

- All combos get "Use Global Setting [X%]" as first option
- Optimal Frame Pacing becomes **tristate** (PartiallyChecked = use global)
- RTC group shown only in per-game mode
- FastCDVD shown only in per-game mode
- Cheats hidden in per-game mode
- Cycle Rate gets "Use Global Setting" prefix

---

## 10. Hidden/Non-Obvious Features

| Feature | Details |
|---------|---------|
| Custom speed range | 0% to 5000% (not just presets) |
| Speed label shows FPS | e.g., "200% [120 FPS (NTSC) / 100 FPS (PAL)]" |
| VSync auto-disable | Automatically disabled when not at 100% speed |
| UseVSyncForTiming conditional | Only enabled when BOTH VSync + SyncToHostRefresh are on |
| Optimal pacing tristate | Per-game mode has 3 states (inherit/on/off) |
| RTC date constraints | 2000–2099 range only |
| Thread pinning | Not just "multithreading" — targets P/E cores specifically |
| FastCDVD per-game only | Not available in global settings |

---

## Coverage Status in Current LumineSX2-Ori

| Feature | In LumineSX2? | Notes |
|---------|-----------|-------|
| Normal/FF/Slow speed combos | ✅ Basic | Missing: preset list, unlimited, custom dialog, FPS labels |
| VSync toggle | ✅ | |
| Sync to Host Refresh | ❌ Missing | |
| Use VSync for Timing | ❌ Missing | |
| Skip Duplicate Frames | ❌ Missing | |
| Optimal Frame Pacing | ✅ Basic | Missing: tristate, max frame latency spinbox |
| Max Frame Latency | ❌ Missing | Spinbox for VsyncQueueSize |
| EE Cycle Rate | ❌ Missing | -3 to +3 range |
| EE Cycle Skipping | ❌ Missing | |
| MTVU | ❌ Missing | |
| Thread Pinning | ❌ Missing | |
| Fast CDVD | ❌ Missing | |
| CDVD Precache | ❌ Missing | |
| RTC (per-game) | ❌ Missing | 6-key datetime binding |
| System Locale Format | ❌ Missing | |
| Host Filesystem | ✅ | |
| Cheats toggle | ❌ Missing | |
| Widget help system | ❌ Missing | Tooltip/hover help |

