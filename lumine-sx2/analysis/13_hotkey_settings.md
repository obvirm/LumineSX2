# PCSX2 Qt Hotkey Settings — Deep Analysis

## Source Files
- `pcsx2-qt/Settings/HotkeySettingsWidget.h` / `.cpp`
- `pcsx2/Input/InputManager.h` / `.cpp`
- `pcsx2/Hotkeys.cpp`
- `pcsx2/GS/GS.cpp`
- `pcsx2-qt/QtHost.cpp`

## Architecture

### HotkeyInfo Struct
```cpp
struct HotkeyInfo {
    const char* name;          // Internal ID
    const char* category;      // Display category (translatable)
    const char* display_name;  // Display name (translatable)
    void (*handler)(s32 pressed); // Callback: 1=all pressed, 0=released, -1=cancelled
};
```

### Three Hotkey Lists
1. `g_common_hotkeys` — Core emulator hotkeys (Hotkeys.cpp)
2. `g_gs_hotkeys` — Graphics/GS hotkeys (GS/GS.cpp)
3. `g_host_hotkeys` — Platform-specific hotkeys (QtHost.cpp: **EMPTY** for Qt)

### InputManager::GetHotkeyList()
Merges all 3 lists into one vector, sorted alphabetically by display_name.

---

## ALL Hotkey Categories & Actions

### Category: Navigation
| ID | Display Name | Handler |
|----|-------------|---------|
| ToggleFullscreen | Toggle Fullscreen | `Host::SetFullscreen()` |
| OpenPauseMenu | Open Pause Menu | `FullscreenUI::OpenPauseMenu()` |
| OpenAchievementsList | Open Achievements List | `FullscreenUI::OpenAchievementsWindow()` |
| OpenLeaderboardsList | Open Leaderboards List | `FullscreenUI::OpenLeaderboardsWindow()` |

### Category: Speed
| ID | Display Name | Handler |
|----|-------------|---------|
| TogglePause | Toggle Pause | `VMManager::SetPaused()` |
| FrameAdvance | Frame Advance | `VMManager::FrameAdvance(1)` |
| ToggleFrameLimit | Toggle Frame Limit | Toggle Nominal/Unlimited |
| ToggleTurbo | Toggle Turbo / Fast Forward | Toggle Nominal/Turbo |
| HoldTurbo | Turbo / Fast Forward (Hold) | Hold = Turbo, release = restore |
| ToggleSlowMotion | Toggle Slow Motion | Toggle Nominal/Slomo |
| IncreaseSpeed | Increase Target Speed | +0.1 speed |
| DecreaseSpeed | Decrease Target Speed | -0.1 speed |

### Category: System
| ID | Display Name | Handler |
|----|-------------|---------|
| ShutdownVM | Shut Down Virtual Machine | `Host::RequestVMShutdown()` |
| ResetVM | Reset Virtual Machine | `VMManager::RequestReset()` |
| ReloadPatches | Reload Patches | `VMManager::ReloadPatches()` |
| SwapMemCards | Swap Memory Cards | `FileMcd_Swap()` |
| InputRecToggleMode | Toggle Input Recording Mode | `g_InputRecording.getControls().toggleRecordMode()` |
| ToggleMouseLock | Toggle Mouse Lock | `Host::SetMouseLock()` |

### Category: Save States
| ID | Display Name | Handler |
|----|-------------|---------|
| PreviousSaveStateSlot | Select Previous Save Slot | `SaveStateSelectorUI::SelectPreviousSlot()` |
| NextSaveStateSlot | Select Next Save Slot | `SaveStateSelectorUI::SelectNextSlot()` |
| SaveStateToSlot | Save State To Selected Slot | `SaveStateSelectorUI::SaveCurrentSlot()` |
| LoadStateFromSlot | Load State From Selected Slot | `SaveStateSelectorUI::LoadCurrentSlot()` |
| LoadBackupStateFromSlot | Load Backup State From Selected Slot | `SaveStateSelectorUI::LoadCurrentBackupSlot()` |
| SaveStateAndSelectNextSlot | Save State and Select Next Slot | Save + Next |
| SelectNextSlotAndSaveState | Select Next Slot and Save State | Next + Save |
| SaveStateToSlot1..10 | Save State To Slot 1..10 | Direct slot save |
| LoadStateFromSlot1..10 | Load State From Slot 1..10 | Direct slot load |

### Category: Audio
| ID | Display Name | Handler |
|----|-------------|---------|
| Mute | Toggle Mute | `HotkeyToggleMute()` |
| IncreaseVolume | Increase Volume | +5 volume |
| DecreaseVolume | Decrease Volume | -5 volume |

### Category: Graphics
| ID | Display Name | Handler |
|----|-------------|---------|
| Screenshot | Save Screenshot | `GSQueueSnapshot()` |
| ToggleVideoCapture | Toggle Video Capture | Start/stop capture |
| GSDumpSingleFrame | Save Single Frame GS Dump | `GSQueueSnapshot(..., 1)` |
| GSDumpMultiFrame | Save Multi Frame GS Dump | Start/stop multi-frame dump |
| ToggleSoftwareRendering | Toggle Software Rendering | `MTGS::ToggleSoftwareRendering()` |
| IncreaseUpscaleMultiplier | Increase Upscale Multiplier | +1x |
| DecreaseUpscaleMultiplier | Decrease Upscale Multiplier | -1x |
| ToggleOSD | Toggle On-Screen Display | Toggle OSD visibility |
| CycleAspectRatio | Cycle Aspect Ratio | Cycle through all AR types |
| ToggleMipmapMode | Toggle Hardware Mipmapping | Toggle HW mipmaps |
| CycleInterlaceMode | Cycle Deinterlace Mode | Cycle 10 modes (Auto, Off, Weave T/B, Bob T/B, Blend T/B, Adaptive T/B) |
| CycleTVShader | Cycle TV Shader | Cycle 8 shaders (None, Scanline, Diagonal, Triangular, Wave, Lottes CRT, 4xRGSS, NxAGSS) |
| CycleBlendingAccuracy | Cycle Blending Accuracy | Cycle 6 levels (Min, Basic, Med, High, Full, Max) |
| ToggleTextureDumping | Toggle Texture Dumping | Toggle dump |
| ToggleTextureReplacements | Toggle Texture Replacements | Toggle load |
| ReloadTextureReplacements | Reload Texture Replacements | Reload map + purge cache |

---

## HotkeySettingsWidget UI Structure

### Layout
```
QScrollArea
  └── QVBoxLayout (m_layout)
      ├── Horizontal Line (separator)
      ├── QLabel (Category Name: "Navigation", bold/large)
      ├── QGridLayout (2 columns: label + binding widget)
      │   ├── QLabel "Toggle Fullscreen"
      │   └── InputBindingWidget (multi-bind button)
      ├── Horizontal Line (separator)
      ├── QLabel (Category Name: "Speed")
      ├── QGridLayout
      │   ├── QLabel "Toggle Pause"
      │   └── InputBindingWidget
      ... (repeated for all categories)
```

### Key Behaviors
1. **Category auto-creation**: Categories created on first encounter; order follows `GetHotkeyList()` alphabetical sort
2. **Multi-binding**: Each hotkey can have MULTIPLE bindings (InputBindingWidget supports multiple keys)
3. **Profile-aware**: Bindings read/write from current controller profile (global or per-game)
4. **Separator lines**: Horizontal rule (12px height) before each category
5. **Category headers**: QLabel with larger/bold font

### ControllerSettingsWindow Integration
- Categories: GlobalSettings → HotkeySettings (index 1)
- Profile system: supports global + per-game profiles
- Restore defaults: `onRestoreDefaultsClicked()` clears all bindings
- Profile CRUD: New, Apply, Rename, Delete

---

## Hidden/Advanced Features

1. **Hold vs Toggle**: HoldTurbo is the ONLY hold-type hotkey; all others are toggle
2. **Chord cancellation**: handler receives -1 when a longer chord takes over
3. **Slot-specific save/load**: Direct hotkeys for slots 1-10 (bypass selector UI)
4. **Backup state loading**: LoadBackupStateFromSlot loads the _backup save
5. **Combined actions**: SaveStateAndSelectNextSlot / SelectNextSlotAndSaveState
6. **GS Dump multi-frame**: Press-and-hold captures until release
7. **Video capture**: Full AV capture with sync (waits for GS thread)
8. **Input Recording**: Toggle mode hotkey for TAS input recording
9. **Mouse lock toggle**: Global setting, not per-game
10. **Host hotkeys empty**: Qt platform defines no extra hotkeys; all are core/GS

## Total Hotkey Count
- Navigation: 4
- Speed: 8
- System: 6
- Save States: 5 + 20 (10 save + 10 load) = 25
- Audio: 3
- Graphics: 16
- **TOTAL: 62 hotkeys**
