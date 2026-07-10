# Hidden Features Discovered from Translation (.ts) Files

## Source Files Analyzed
- `pcsx2-qt_en.ts` (27,073 lines)
- `pcsx2-qt_id-ID.ts` (25,847 lines)
- Total: 175 contexts, 5,014 messages

## 1. FullscreenUI — 347 strings (COMPLETELY MISSED)

There is a FULL Big Picture-style UI built into PCSX2 Qt that was NOT covered by the 50-agent .cpp/.h analysis or the .ui analysis. This is a full-screen gamepad-navigable overlay system with:
- Game browser grid
- Settings panels
- Save state management
- Achievement viewer
- Controller configuration
- All managed via gamepad input

This is a massive feature that needs its own dedicated analysis.

## 2. Controller Macros (Detailed)

From `ControllerMacroEditWidget` (6 strings):
- **Multi-button triggers**: "Select the buttons which you want to trigger with this macro. All buttons are activated concurrently."
- **Pressure sensitivity**: "For buttons which are pressure sensitive, this slider controls how much force will be simulated when the macro is active."
- **Chord triggers**: "Select the trigger to activate this macro. This can be a single button, or combination of buttons (chord). Shift-click for multiple triggers."
- **Toggle frame interval**: "Macro will toggle every N frames."
- **No repeat**: "Macro will not repeat."
- **Custom interval**: "Macro will toggle buttons every %1 frames."

## 3. Input Vibration Binding

From `InputVibrationBindingWidget`:
- There is a separate binding widget specifically for vibration/rumble, independent from regular button bindings.

## 4. Missing Graphics Features

### Display
- **"Apply Widescreen Patches"** — Toggle inside Display settings tab
- **"Apply No-Interlacing Patches"** — Toggle inside Display settings tab  
- **"FMV Aspect Ratio Override"** — Combo box with "Fit to Window / Fullscreen"
- **"Fullscreen Mode"** — Confirmed (was added to our implementation)

### Advanced Graphics
- **"Disable Readbacks (Synchronize GS Thread)"** — Toggle in Advanced tab
- **"GS Dump Compression"** — Combo box (likely zstd/none)
- **"Allow Exclusive Fullscreen"** — Toggle
- **"Override Texture Barriers"** — Combo/checkbox
- **"Use Debug Device"** — Toggle for Vulkan/D3D12 debug layers
- **"Use Debug Blend"** — Toggle for debugging blend operations

### Hardware Fixes (new ones)
- **"Software CLUT Render"** — Render color lookup table on CPU
- **"CPU Sprite Render Size"** — Combo for sprite render size override
- **"Disable Render Fixes"** — Master toggle for all render fixes

### Post-Processing
- **"Sharpen and Resize (Display Resolution)"** — CAS sharpening mode
- **"4xRGSS downsampling"** — Confirmed
- **"NxAGSS downsampling"** — Confirmed

### Texture Replacement
- **"Dump FMV Textures"** — Separate from Dump Mipmaps, for FMV video textures

## 5. Missing Emulation Features

### Cheats
- **"Enable Cheats"** in Emulation settings (separate from Cheats panel)
- **"Automatically loads and applies cheats on game start."** tooltip

### Speed Control  
- Fast-forward and slow-motion speed settings are in Emulation (not interface)
- **"Sets the fast-forward speed. This speed will be used when the fast-forward hotkey is pressed/toggled."**
- **"Sets the slow-motion speed. This speed will be used when the slow-motion hotkey is pressed/toggled."**

### VSync / Frame Pacing
- **"Sets the VSync queue size to 0, making every frame be completed and presented by the GS before input is polled and the next frame begins."**
- **"Sets the maximum number of frames that can be queued up to the GS"** — Max Frame Latency
- **"Speeds up emulation so that the guest refresh rate matches the host."** — Sync to Host Refresh Rate
- **"Detects when idle frames are being presented in 25/30fps games, and skips presenting those frames."** — Skip Duplicate Frames

## 6. USB Devices (More Detail)

From USB context:
- **"Konami Capture Eye"** — Camera device
- **"Singstar"** — Microphone device for SingStar games  
- **"USB-Mic: Failed to start player {} audio stream."**
- **"Menu Up" / "Menu Down"** — USB device navigation
- **"Works around bugs in some wheels' firmware"** — Wheel force feedback fix toggle
- **"Failed to open '{}' for printing."** — Printer device support?

From USB binding widgets:
- **GunCon2**: Detailed help text about using mouse vs controller for aiming
- **ShinkansenCon**: Detailed description of train controller (13 power notches, 7 brake notches + emergency brake)

## 7. Memory Card Features (More Detail)

From MemoryCard context:
- **Hot-swap detection**: "Memory cards are being auto-ejected. Can't swap right now."
- **Auto-eject**: "Force ejecting all Memory Cards. Reinserting in 1 second."
- **Missing card**: "%1 [Missing]" label format
- **Memory Card Swap**: Confirmed swap ports feature
- **Delete Memory Card**: Confirmed with warning dialog
- **Rename Memory Card**: Confirmed
- **Eject Memory Card**: Confirmed

## 8. Folder Settings (New)

From `FolderSettingsWidget`:
- **"Used for storing shaders, game list, and achievement data."** — Data directory
- **"Cheats Directory"** — ".pnach files containing game cheats"
- **"Screenshots and GS dumps"** — Output directory
- **"Covers"** directory — "for game grid/Big Picture UIs"
- **"Video Recording Directory"** — with "Save Video Recordings in Game-Specific Folders" toggle
- **"Save States Directory"** — explicit savestate folder

## 9. Setup Wizard Pages

From `SetupWizardDialog`:
- Page 1: **BIOS Image** — Path selection, BIOS dumping guide link
- Page 2: **Game Directories** — Supported formats: .bin/.iso, .mdf, .chd, .cso, .zso, .gz
- Page 3: **RetroAchievements Login** — Optional, with "RAIntegration" detection
- Page 4: **Enable Automatic Updates** — Channel selection
- **Supported formats** list for disc dumps

## 10. Auto-Updater Details

From `AutoUpdaterDialog`:
- **Savestate compatibility warning**: "Installing this update will make your save states incompatible"
- **Settings reset warning**: "Installing this update will reset your program configuration"
- **"Failed to remove updater exe after update"** — Self-update cleanup
- **"No updates are currently available."**

## 11. Shortcut Creation Details

From `ShortcutCreationDialog` (31 strings):
- **"Override boot ELF"** — Custom ELF path
- **"Fullscreen mode"** — Toggle
- **"Use Big Picture mode"** — Launch in Big Picture mode
- **"Load save state by slot"** — Load specific state
- **"Load save state from file"** — Load from file
- **"Do not load save state"** — Skip auto-load
- **Custom arguments**: "You may add additional (space-separated) custom arguments"

## 12. Hotkeys (New Ones)

From Hotkeys context (33 items):
- **"Save Single Frame GS Dump"**
- **"Save Multi Frame GS Dump"**
- **"Toggle Video Capture"**
- **"Toggle Software Rendering"**
- **"Toggle On-Screen Display"**
- **"4xRGSS"** / **"NxAGSS"** — Hotkey to toggle shaders
- **"Toggle Texture Dumping"**

## 13. GS Dump / Capture Details

- **GSDumpFile**: Multiple compression-related strings — LZMA-based compression
- **GSCapture**: "Failed to load FFmpeg" — FFmpeg dependency for video capture
  - Requirements: libavcodec, libavformat, libavutil, libswscale, libswresample
- **GS**: "Saving {0} GS dump {1} to '{2}'" — GS dump with compression
- **"Aborted {} due to encoding error in '{}'."** — Encoding error handling

## 14. CAS (Contrast Adaptive Sharpening)

From `GraphicsPostProcessingSettingsTab`:
- **"Sharpen and Resize (Display Resolution)"** — CAS sharpening (was not in our implementation)
- Different from the TV shaders

## 15. PINE Details

From `AdvancedSystemSettingsWidget`:
- **"PINE Settings"** — Confirmed as separate section in Advanced
- **PINE = PCSX2 Integrated Network Environment** (IPC for external tools)

## 16. Save State Management

- **Version check**: "This save state was created with PCSX2 version {0}. It is no longer compatible..."
- **Backup slots**: "Failed to load state from backup slot"
- **Auto-save on shutdown**: Confirmed

## 17. SDL3/Input Migration

- **"As part of our upgrade to SDL3, we've had to migrate your binds."** — Uses SDL3
- **Xbox layout detection**: "Your controller did not match the Xbox layout and may need rebinding."

## 18. Input Recording Files

- **".p2m2"** file extension for input recordings
- Limit: "you've been playing for far too long and thus have reached the limit of input recording"

## 19. CDVD

- **Block dumping**: "Saving CDVD block dump to '{}'."
- **Precache memory**: "Not enough memory available for precaching ({:.2f} GB required)."

## Summary of Currently MISSING from LumineSX2 Implementation

| Priority | Feature | Where to Add |
|----------|---------|-------------|
| HIGH | FullscreenUI (347 strings) | New file `fullscreen_ui.slint` |
| HIGH | Controller Macro Edit (chord, pressure, frame interval) | `controller_features.slint` |
| HIGH | Input Vibration Binding Widget | `controller_features.slint` |
| MED | Apply Widescreen/No-Interlace Patches in Display | `settings_graphics.slint` |
| MED | FMV Aspect Ratio Override | `settings_graphics.slint` |
| MED | Disable Readbacks, GS Dump Compression, Exclusive FS | `settings_graphics.slint` |
| MED | Override Texture Barriers, Debug Device/Blend | `settings_graphics.slint` |
| MED | Software CLUT Render, CPU Sprite Render Size | `settings_graphics.slint` |
| MED | CAS Sharpening mode | `settings_graphics.slint` |
| MED | Dump FMV Textures toggle | `settings_graphics.slint` |
| MED | Enable Cheats in Emulation settings | `settings_interface.slint` |
| MED | Fast-Forward/Slow-Mo speed setting in Emulation | `settings_interface.slint` |
| LOW | Konami Capture Eye, SingStar USB device types | Already in USBWidget |
| LOW | Shinkansen/GunCon2 binding help text | Already in controller bindings |
| LOW | Memory card hot-swap strings | Already in memory card |
| LOW | Supported formats in GameList | Already documented |
