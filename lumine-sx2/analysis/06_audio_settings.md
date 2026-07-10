# PCSX2 Qt Audio Settings — Deep Analysis

**Source**: `AudioSettingsWidget.h`, `AudioSettingsWidget.cpp`  
**Setting namespace**: `SPU2/Output`

---

## 1. Audio Backend

| Property | Value |
|----------|-------|
| Widget | `QComboBox` (`m_ui.audioBackend`) |
| Setting key | `SPU2/Output` / `Backend` |
| Default | `Pcsx2Config::SPU2Options::DEFAULT_BACKEND` (Cubeb) |
| Enum | `AudioBackend` — Cubeb, SDL, XAudio2, Null |
| Display name | `AudioStream::GetBackendDisplayName()` |

**Behavior**:  
- Changing backend triggers `updateDriverNames()` → cascades to `updateDeviceNames()`
- Backend affects available drivers and output devices

---

## 2. Driver Selection

| Property | Value |
|----------|-------|
| Widget | `QComboBox` (`m_ui.driver`) |
| Setting key | `SPU2/Output` / `DriverName` |
| Source | `AudioStream::GetDriverNames(backend)` |
| Enabled | Only when backend has multiple drivers |

**Behavior**:  
- If backend has no drivers → shows "Default", disabled
- Changing driver triggers `updateDeviceNames()`

---

## 3. Output Device

| Property | Value |
|----------|-------|
| Widget | `QComboBox` (`m_ui.outputDevice`) |
| Setting key | `SPU2/Output` / `DeviceName` |
| Source | `AudioStream::GetOutputDevices(backend, driver)` |
| Enabled | Only when devices available |

**Behavior**:  
- Tracks `m_output_device_latency` (minimum latency frames from device info)
- If current device not found → adds "Unknown Device" entry
- Changing device updates latency label

---

## 4. Buffer Size

| Property | Value |
|----------|-------|
| Widget | `QSlider` (`m_ui.bufferMS`) |
| Setting key | `SPU2/Output` / `BufferMS` |
| Default | `AudioStreamParameters::DEFAULT_BUFFER_MS` |
| Display | `m_ui.bufferMSLabel` — shows `{value} ms` |

**Description**: Determines the buffer size for the time stretcher. Effectively selects average latency — audio is stretched/shrunk to keep buffer within check.

---

## 5. Output Latency

| Property | Value |
|----------|-------|
| Widget | `QSlider` (`m_ui.outputLatencyMS`) |
| Setting key | `SPU2/Output` / `OutputLatencyMS` |
| Default | `AudioStreamParameters::DEFAULT_OUTPUT_LATENCY_MS` |
| Display | `m_ui.outputLatencyLabel` — shows `{value} ms` or "N/A" if minimal |

**Description**: Latency from buffer to host audio output. Can be set lower than target to reduce delay.

---

## 6. Minimal Output Latency (Hidden Feature)

| Property | Value |
|----------|-------|
| Widget | `QCheckBox` (`m_ui.outputLatencyMinimal`) |
| Setting key | `SPU2/Output` / `OutputLatencyMinimal` |
| Default | `false` |

**Behavior**:  
- When checked → disables the Output Latency slider
- Uses minimum device latency instead of manual value
- Shows "N/A" in latency label
- This is a **hidden feature** — many users don't know it exists

---

## 7. Standard Volume

| Property | Value |
|----------|-------|
| Widget | `QSlider` (`m_ui.standardVolume`) |
| Setting key | `SPU2/Output` / `StandardVolume` |
| Default | `100` (percent) |
| Display | `m_ui.standardVolumeLabel` — shows `{value}%` |
| Reset button | `m_ui.resetStandardVolume` |

**Behavior**:  
- For base settings: immediately applies via `g_emu_thread->applySettings()`
- For per-game settings: uses `BindWidgetAndLabelToIntSetting`
- Reset button: resets to 100 (base) or removes per-game override

---

## 8. Fast Forward Volume (Hidden Feature)

| Property | Value |
|----------|-------|
| Widget | `QSlider` (`m_ui.fastForwardVolume`) |
| Setting key | `SPU2/Output` / `FastForwardVolume` |
| Default | `100` (percent) |
| Display | `m_ui.fastForwardVolumeLabel` — shows `{value}%` |
| Reset button | `m_ui.resetFastForwardVolume` |

**Description**: Separate volume control for fast-forward mode. Many users don't know this exists.

---

## 9. Mute All Sound

| Property | Value |
|----------|-------|
| Widget | `QCheckBox` (`m_ui.muted`) |
| Setting key | `SPU2/Output` / `OutputMuted` |
| Default | `false` |

**Behavior**:  
- For base settings: immediately applies
- For per-game settings: uses `BindWidgetToBoolSetting`

---

## 10. Expansion Mode

| Property | Value |
|----------|-------|
| Widget | `QComboBox` (`m_ui.expansionMode`) |
| Setting key | `SPU2/Output` / `ExpansionMode` |
| Default | `AudioStreamParameters::DEFAULT_EXPANSION_MODE` |
| Enum | `AudioExpansionMode` — Disabled, Surround (Dolby Pro Logic II) |
| Settings button | `m_ui.expansionSettings` → opens dialog |

**Behavior**:  
- When mode is Disabled → expansion settings button disabled
- Expansion adds latency (calculated via `GetMSForBufferSize`)

---

## 11. Expansion Settings Dialog (Hidden Feature)

**Trigger**: Click expansion settings button  
**UI file**: `ui_AudioExpansionSettingsDialog.ui`

| Setting | Key | Default | Description |
|---------|-----|---------|-------------|
| Block Size | `ExpandBlockSize` | `DEFAULT_EXPAND_BLOCK_SIZE` | Power-of-2 block size |
| Circular Wrap | `ExpandCircularWrap` | `DEFAULT_EXPAND_CIRCULAR_WRAP` | Circular wrap angle |
| Shift | `ExpandShift` | `DEFAULT_EXPAND_SHIFT` | Channel shift (normalized 0-1) |
| Depth | `ExpandDepth` | `DEFAULT_EXPAND_DEPTH` | Surround depth (normalized 0-1) |
| Focus | `ExpandFocus` | `DEFAULT_EXPAND_FOCUS` | Focus level (normalized 0-1) |
| Center Image | `ExpandCenterImage` | `DEFAULT_EXPAND_CENTER_IMAGE` | Center channel image (normalized 0-1) |
| Front Separation | `ExpandFrontSeparation` | `DEFAULT_EXPAND_FRONT_SEPARATION` | Front channel separation (normalized 0-1) |
| Rear Separation | `ExpandRearSeparation` | `DEFAULT_EXPAND_REAR_SEPARATION` | Rear channel separation (normalized 0-1) |
| Low Cutoff | `ExpandLowCutoff` | `DEFAULT_EXPAND_LOW_CUTOFF` | Low frequency cutoff (Hz) |
| High Cutoff | `ExpandHighCutoff` | `DEFAULT_EXPAND_HIGH_CUTOFF` | High frequency cutoff (Hz) |

**Buttons**: Close, Restore Defaults  
**Restore Defaults**: Sets all to defaults (or removes per-game overrides)

---

## 12. Sync Mode

| Property | Value |
|----------|-------|
| Widget | `QComboBox` (`m_ui.syncMode`) |
| Setting key | `SPU2/Output` / `SyncMode` |
| Default | `Pcsx2Config::SPU2Options::DEFAULT_SYNC_MODE` (TimeStretch) |
| Enum | `SPU2SyncMode` — TimeStretch, Async, None |
| Settings button | `m_ui.stretchSettings` → opens dialog (only enabled for TimeStretch) |

**Description**: When emulation isn't at 100% speed, adjusts audio tempo for better sound during fast-forward/slowdown.

---

## 13. Stretch Settings Dialog (Hidden Feature)

**Trigger**: Click stretch settings button (only active when SyncMode == TimeStretch)  
**UI file**: `ui_AudioStretchSettingsDialog.ui`

| Setting | Key | Default | Description |
|---------|-----|---------|-------------|
| Sequence Length | `StretchSequenceLengthMS` | `DEFAULT_STRETCH_SEQUENCE_LENGTH` | Length of processing sequence (ms) |
| Seek Window | `StretchSeekWindowMS` | `DEFAULT_STRETCH_SEEKWINDOW` | Seek window size (ms) |
| Overlap | `StretchOverlapMS` | `DEFAULT_STRETCH_OVERLAP` | Overlap between frames (ms) |
| Use Quick Seek | `StretchUseQuickSeek` | `DEFAULT_STRETCH_USE_QUICKSEEK` | Enable quick seeking algorithm |
| Use AA Filter | `StretchUseAAFilter` | `DEFAULT_STRETCH_USE_AA_FILTER` | Enable anti-aliasing filter |

**Buttons**: Close, Restore Defaults

---

## 14. Latency Calculation (Dynamic Display)

**Label**: `m_ui.bufferingLabel`  
**Formula**:
```
expand_buffer_ms = GetMSForBufferSize(SAMPLE_RATE, expansion_block_size)
output_latency_ms = minimal ? GetMSForBufferSize(SAMPLE_RATE, device_latency) : config_output_latency_ms

Total = config_buffer_ms + expand_buffer_ms + output_latency_ms
```

**Display variations**:
- With expansion + known latency: `Maximum Latency: X ms (Y ms buffer + Z ms expand + W ms output)`
- Without expansion + known latency: `Maximum Latency: X ms (Y ms buffer + W ms output)`
- With expansion + unknown latency: `Maximum Latency: X ms (Y ms expand, minimum output latency unknown)`
- Without expansion + unknown latency: `Maximum Latency: X ms (minimum output latency unknown)`

---

## 15. Per-Game Settings Handling

- Volume and mute have different code paths for base vs per-game
- Base settings: immediate apply via `g_emu_thread->applySettings()`
- Per-game: uses `BindWidgetAndLabelToIntSetting` with bold font for overridden values
- Reset removes per-game override and reverts to global/inherited value
- Expansion/Stretch dialogs: Restore Defaults removes per-game overrides instead of setting defaults

---

## 16. Setting Keys Summary (SPU2/Output)

| Key | Type | Default | Widget |
|-----|------|---------|--------|
| Backend | enum string | Cubeb | QComboBox |
| DriverName | string | (first available) | QComboBox |
| DeviceName | string | (first available) | QComboBox |
| BufferMS | int | DEFAULT_BUFFER_MS | QSlider |
| OutputLatencyMS | int | DEFAULT_OUTPUT_LATENCY_MS | QSlider |
| OutputLatencyMinimal | bool | false | QCheckBox |
| StandardVolume | int | 100 | QSlider |
| FastForwardVolume | int | 100 | QSlider |
| OutputMuted | bool | false | QCheckBox |
| ExpansionMode | enum string | Disabled | QComboBox |
| SyncMode | enum string | TimeStretch | QComboBox |
| ExpandBlockSize | int | DEFAULT | QSlider |
| ExpandCircularWrap | float | DEFAULT | QSlider |
| ExpandShift | float | DEFAULT | QSlider (normalized) |
| ExpandDepth | float | DEFAULT | QSlider (normalized) |
| ExpandFocus | float | DEFAULT | QSlider (normalized) |
| ExpandCenterImage | float | DEFAULT | QSlider (normalized) |
| ExpandFrontSeparation | float | DEFAULT | QSlider (normalized) |
| ExpandRearSeparation | float | DEFAULT | QSlider (normalized) |
| ExpandLowCutoff | int | DEFAULT | QSlider |
| ExpandHighCutoff | int | DEFAULT | QSlider |
| StretchSequenceLengthMS | int | DEFAULT | QSlider |
| StretchSeekWindowMS | int | DEFAULT | QSlider |
| StretchOverlapMS | int | DEFAULT | QSlider |
| StretchUseQuickSeek | bool | DEFAULT | QCheckBox |
| StretchUseAAFilter | bool | DEFAULT | QCheckBox |

---

## 17. Hidden Features Found

1. **Minimal Output Latency** — checkbox that bypasses manual latency slider, uses device minimum
2. **Fast Forward Volume** — separate volume control for fast-forward mode
3. **Expansion Settings** — 10+ hidden parameters for surround sound expansion (FreeSurround-based)
4. **Stretch Settings** — 5 hidden parameters for time-stretching behavior (SoundTouch-based)
5. **Driver Selection** — per-backend driver selection (hidden when backend has only one driver)
6. **Output Device** — per-driver device selection with latency tracking
7. **Dynamic Latency Display** — real-time calculation showing total latency breakdown
8. **Per-Game Volume Reset** — bold font indicator for overridden values
9. **Block Size Auto-Rounding** — non-power-of-2 block sizes are auto-rounded to next power of 2
