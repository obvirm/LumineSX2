# Graphics Settings UI File Analysis — Widget-Level Detail from .ui XML Files

Files analyzed: 11 .ui files from `Settings/`

---

## 1. GraphicsSettingsHeader.ui
**Widget**: QWidget
**Layout**: QVBoxLayout > QGroupBox > QFormLayout

| Row | Widget | Type | Text | Buddy |
|-----|--------|------|------|-------|
| 0 | rendererText | QLabel | "Graphics API:" | rendererDropdown |
| 0 | rendererDropdown | QComboBox | (populated at runtime) | - |
| 1 | adapterText | QLabel | "Adapter:" | adapterDropdown |
| 1 | adapterDropdown | QComboBox | (populated at runtime) | - |

**Tab order**: rendererDropdown → adapterDropdown
**NOT in LumineSX2**: Wrapping in QGroupBox (visual grouping), buddy labels for accessibility.

---

## 2. GraphicsDisplaySettingsTab.ui
**Widget**: QWidget (Expanding, Preferred)
**Layout**: QGridLayout (columnstretch=0,1)

### Combo Boxes (6):
| Name | Items | Buddy |
|------|-------|-------|
| `fullscreenModes` | (populated at runtime) | fsModeLabel "Fullscreen Mode:" |
| `aspectRatio` | "Fit to Window / Fullscreen", "Auto Standard (4:3 Interlaced / 3:2 Progressive)", "Standard (4:3)", "Widescreen (16:9)", "Native/Full (10:7)" | aspectRatioLabel |
| `fmvAspectRatio` | Same 5 options + "Off (Default)" as first | fmvAspectRatioLabel "FMV Aspect Ratio Override:" |
| `interlacing` | **10 modes**: Automatic (Default), No Deinterlacing, Weave (TFF/BFF), Bob (TFF/BFF), Blend (TFF/BFF), Adaptive (TFF/BFF) | interlacingLabel "Deinterlacing:" |
| `bilinearFiltering` | "None", "Bilinear (Smooth)", "Bilinear (Sharp)" | billinearLabel "Bilinear Filtering:" |

### SpinBox (1):
| Name | Min | Max | Suffix | Control |
|------|-----|-----|--------|---------|
| `stretchY` | 1 | 300 | "%" | verticalStretchLabel "Vertical Stretch:" |

### Crop SpinBoxes (4) in sub-gridLayout:
| Name | Min | Max | Suffix |
|------|-----|-----|--------|
| `cropLeft` | 0 | 1000 | "px" |
| `cropRight` | 0 | 1000 | "px" |
| `cropTop` | 0 | 1000 | "px" |
| `cropBottom` | 0 | 1000 | "px" |

### Checkboxes in displayGridLayout (7):
| Name | Text | Notes |
|------|------|-------|
| `widescreenPatches` | "Apply Widescreen Patches" | |
| `noInterlacingPatches` | "Apply No-Interlacing Patches" | |
| `PCRTCAntiBlur` | "Anti-Blur" | **Shortcut: Ctrl+S** |
| `integerScaling` | "Integer Scaling" | |
| `PCRTCOffsets` | "Screen Offsets" | |
| `disableInterlaceOffset` | "Disable Interlace Offset" | |
| `PCRTCOverscan` | "Show Overscan" | |

### Overall tab order: 19 tabstops
fullscreenModes → aspectRatio → fmvAspectRatio → interlacing → bilinearFiltering → stretchY → cropLeft → cropRight → cropTop → cropBottom → widescreenPatches → noInterlacingPatches → PCRTCAntiBlur → integerScaling → PCRTCOffsets → disableInterlaceOffset → PCRTCOverscan

### MISSING from LumineSX2:
- Fullscreen Mode dropdown (completely absent from LumineSX2 GraphicsSettingsView)
- 10 deinterlacing modes (LumineSX2 has none)
- Bilinear Filtering (Smooth/Sharp) instead of generic
- Vertical Stretch QSpinBox 1-300%
- Crop (Left/Right/Top/Bottom) px values
- Anti-Blur with Ctrl+S shortcut
- Show Overscan
- Integer Scaling
- FMV Aspect Ratio Override

---

## 3. GraphicsHardwareRenderingSettingsTab.ui
**Layout**: QGridLayout columnstretch=0,1

### Combo Boxes (7):
| Name | Items | Notes |
|------|-------|-------|
| `upscaleMultiplier` | (populated at runtime - 1x-8x) | |
| `textureFiltering` | "Nearest", "Bilinear (Forced)", "Bilinear (PS2)", "Bilinear (Forced excluding sprite)" | |
| `trilinearFiltering` | "Automatic (Default)", "Off (None)", "Trilinear (PS2)", "Trilinear (Forced)" | |
| `anisotropicFiltering` | (populated at runtime) | |
| `dithering` | "Off", "Scaled", "Unscaled (Default)", "Force 32bit" | |
| `blending` | **6 levels**: "Minimum", "Basic (Recommended)", "Medium", "High", "Full (Slow)", "Maximum (Very Slow)" | |
| | | |

### Checkboxes in hardwareRenderingOptionsLayout (5):
| Name | Text |
|------|------|
| `mipmapping` | "Mipmapping" |
| `accurateAlphaTest` | "Accurate Alpha Test" |
| `hwAA1` | "AA1" |
| `rov` | "Rasterizer Ordered View" |
| `enableHWFixes` | "Manual Hardware Renderer Fixes" |

### Tab order: 10 stops

### MISSING from LumineSX2:
- Bilinear (Forced excluding sprite) — 4th texture filtering option
- Dithering: "Force 32bit" option (LumineSX2 has none)
- Blending Accuracy: 6 levels (LumineSX2 has none)
- Accurate Alpha Test
- AA1 checkbox
- Rasterizer Ordered View
- Manual Hardware Renderer Fixes toggle

---

## 4. GraphicsHardwareFixesSettingsTab.ui
**Layout**: QGridLayout columnstretch=0,1

### Combo Boxes (6):
| Name | Items | Notes |
|------|-------|-------|
| `cpuSpriteRenderBW` | **11 items**: "0 (Disabled)" through "10 (640 Max Width)" | Paired with SpriteRenderLevel |
| `cpuSpriteRenderLevel` | "Sprites Only", "Sprites/Triangles", "Blended Sprites/Triangles" | |
| `cpuCLUTRender` | "0 (Disabled)", "1 (Normal)", "2 (Aggressive)" | Default=0 |
| `gpuTargetCLUTMode` | "Disabled (Default)", "Enabled (Exact Match)", "Enabled (Check Inside Target)" | |
| `hwAutoFlush` | "Disabled (Default)", "Enabled (Sprites Only)", "Enabled (All Primitives)" | |
| `textureInsideRt` | "Disabled (Default)", "Inside Target", "Merge Targets" | |
| `limit24BitDepth` | "Disabled (Default)", "Prioritize Upper Bits", "Prioritize Lower Bits" | |

### SpinBoxes (2):
| Name | Min | Max |
|------|-----|-----|
| `skipDrawStart` | 0 | 10000 |
| `skipDrawEnd` | 0 | 10000 |

### Checkboxes in hwFixesLayout (10):
| Row | Name | Text |
|-----|------|------|
| 0,0 | `disableDepthEmulation` | "Disable Depth Conversion" |
| 0,1 | `frameBufferConversion` | "Framebuffer Conversion" |
| 1,0 | `disablePartialInvalidation` | "Disable Partial Source Invalidation" |
| 1,1 | `gpuPaletteConversion` | "GPU Palette Conversion" |
| 2,0 | `disableSafeFeatures` | "Disable Safe Features" |
| 2,1 | `preloadFrameData` | "Preload Frame Data" |
| 3,0 | `disableRenderFixes` | "Disable Render Fixes" |
| 3,1 | `readTCOnClose` | "Read Targets When Closing" |
| 4,0 | `estimateTextureRegion` | "Estimate Texture Region" |
| 4,1 | `drawBuffering` | "Draw Buffering" |

### Tab order: 18 stops

### MISSING from LumineSX2:
- CPU Sprite Render Size/BW (0-10 range, 640 max width)
- CPU Sprite Render Level (3 modes)
- Software CLUT Render (Disabled/Normal/Aggressive)
- GPU Target CLUT (3 modes with CLUT explanation)
- Auto Flush 3-state (Disabled/Sprites/All) - LumineSX2 only has toggle
- Texture Inside RT (3-state: Disabled/Inside/Merge)
- Limit Depth to 24 Bits (3-state)
- Skip Draw Range with END spinbox too (LumineSX2 only has start)
- Disable Depth Conversion, Framebuffer Conversion
- Disable Safe Features, Disable Render Fixes
- Read Targets When Closing
- Estimate Texture Region
- Draw Buffering
- ALL 10 hardware fix checkboxes

---

## 5. GraphicsUpscalingFixesSettingsTab.ui
**Layout**: QGridLayout

### Combo Boxes (4):
| Name | Items | Notes |
|------|-------|-------|
| `halfPixelOffset` | **6 modes**: "Off (Default)", "Normal (Vertex)", "Special (Texture)", "Special (Texture - Aggressive)", "Align to Native", "Align to Native - with Texture Offset" | LumineSX2 has only 4 |
| `nativeScaling` | **5 modes**: "Off", "Normal", "Aggressive", "Normal (Maintain Upscale)", "Aggressive (Maintain Upscale)" | LumineSX2 has none |
| `roundSprite` | "Off (Default)", "Half", "Full" | |
| `bilinearHack` | "Automatic (Default)", "Force Bilinear", "Force Nearest" | Label: "Bilinear Dirty Upscale:" |

### SpinBoxes (2):
| Name | Min | Max |
|------|-----|-----|
| `textureOffsetX` | 0 | 1000 |
| `textureOffsetY` | 0 | 1000 |

### Checkboxes in upscalingFixesLayout:
| Name | Text |
|------|------|
| `alignSprite` | "Align Sprite" |
| `nativePaletteDraw` | "Unscaled Palette Texture Draws" |
| `mergeSprite` | "Merge Sprite" |
| `forceEvenSpritePosition` | "Force Even Sprite Position" |

### Tab order: 10 stops

### MISSING from LumineSX2:
- Half Pixel Offset: "Align to Native" + "Align to Native - with Texture Offset" (2 more modes)
- Native Scaling with "Maintain Upscale" variants (5 modes total)
- Texture Offset X/Y (0-1000 pixels)
- Align Sprite, Merge Sprite, Force Even Sprite Position
- Unscaled Palette Texture Draws (nativePaletteDraw)

---

## 6. GraphicsSoftwareRenderingSettingsTab.ui
**Layout**: QGridLayout columnstretch=0,1

### Combo Boxes (1):
| Name | Items | Notes |
|------|-------|-------|
| `swTextureFiltering` | "Nearest", "Bilinear (Forced)", "Bilinear (PS2)", "Bilinear (Forced excluding sprite)" | Same options as HW tab |

### SpinBoxes (1):
| Name | Suffix | Notes |
|------|--------|-------|
| `extraSWThreads` | " threads" | Label: "Software Rendering Threads:" |

### Checkboxes (2):
| Name | Text |
|------|------|
| `swAutoFlush` | "Auto Flush" |
| `swMipmap` | "Mipmapping" |

### Tab order: 4 stops

### MISSING from LumineSX2:
- " threads" suffix on SW threads spinbox
- Bilinear (Forced excluding sprite) option in SW mode

---

## 7. GraphicsPostProcessingSettingsTab.ui
**Layout**: QVBoxLayout with QGroupBoxes

### GroupBox: "Sharpening/Anti-Aliasing"

#### Combo Boxes (1):
| Name | Items |
|------|-------|
| `casMode` | "None (Default)", "Sharpen Only (Internal Resolution)", "Sharpen and Resize (Display Resolution)" |

#### SpinBoxes (1):
| Name | Min | Max | Default | Suffix |
|------|-----|-----|---------|--------|
| `casSharpness` | 0 | 100 | 50 | "%" |

#### Checkboxes (1):
| Name | Text |
|------|------|
| `fxaa` | "FXAA" |

### GroupBox: "Filters"

#### Combo Boxes (1):
| Name | Items |
|------|-------|
| `tvShader` | **8 modes**: "None (Default)", "Scanline Filter", "Diagonal Filter", "Triangular Filter", "Wave Filter", "Lottes CRT", "4xRGSS downsampling", "NxAGSS downsampling" |

#### SpinBoxes (4) in shadeboost sub-grid:
| Name | Min | Max | Notes |
|------|-----|-----|-------|
| `shadeBoostBrightness` | 1 | 100 | |
| `shadeBoostContrast` | 1 | 100 | |
| `shadeBoostGamma` | 1 | 100 | |
| `shadeBoostSaturation` | 1 | 100 | |

#### Checkboxes (1):
| Name | Text |
|------|------|
| `shadeBoost` | "Shade Boost" |

### Tab order: 9 stops

### MISSING from LumineSX2:
- **CAS (Contrast Adaptive Sharpening)** with 3 modes + sharpness slider — completely absent
- **FXAA** checkbox
- **TV Shader** with 8 modes (Scanline, Diagonal, Triangular, Wave, Lottes CRT, 4xRGSS, NxAGSS) — LumineSX2 has none
- Shade Boost with Gamma control (LumineSX2 only has Brightness/Contrast/Saturation)
- QSpinBox for shadeboost values (1-100) vs LumineSX2 SliderRow (0-1)

---

## 8. GraphicsTextureReplacementSettingsTab.ui
**Layout**: QVBoxLayout with QGroupBoxes

### GroupBox: "Options"
#### Checkboxes (6):
| Row | Name | Text |
|-----|------|------|
| 1,0 | `loadTextureReplacements` | "Load Textures" |
| 1,1 | `dumpReplaceableTextures` | "Dump Textures" |
| 2,0 | `loadTextureReplacementsAsync` | "Asynchronous Texture Loading" |
| 2,1 | `dumpReplaceableMipmaps` | "Dump Mipmaps" |
| 3,0 | `precacheTextureReplacements` | "Precache Textures" |
| 3,1 | `dumpTexturesWithFMVActive` | "Dump FMV Textures" |

### GroupBox: "Search Directory"
| Row | Widget | Type | Text |
|-----|--------|------|------|
| 0 | `textureDescriptionText` | QLabel | "PCSX2 will dump and load texture replacements from this directory." |
| 1,0 | `texturesDirectory` | QLineEdit | (path) |
| 1,1 | `texturesBrowse` | QPushButton | "Browse..." |
| 1,2 | `texturesOpen` | QPushButton | "Open..." |
| 1,3 | `texturesReset` | QPushButton | "Reset" |

### Tab order: 10 stops

### MISSING from LumineSX2:
- **Asynchronous Texture Loading** toggle
- **Dump Mipmaps** toggle  
- **Precache Textures** toggle
- **Dump FMV Textures** toggle
- Browse/Open/Reset buttons for texture directory (LumineSX2 has none)
- Description text label

---

## 9. GraphicsMediaCaptureSettingsTab.ui
**Layout**: QVBoxLayout with QGroupBoxes

### GroupBox: "Screenshot Capture Setup"
#### Combo Boxes (2):
| Name | Items |
|------|-------|
| `screenshotSize` | "Display Resolution (Aspect Corrected)", "Internal Resolution (Aspect Corrected)", "Internal Resolution (No Aspect Correction)" |
| `screenshotFormat` | "PNG", "JPEG", "WebP" |

#### SpinBoxes (1):
| Name | Min | Max | Suffix |
|------|-----|-----|--------|
| `screenshotQuality` | 1 | 100 | "%" |

### GroupBox: "Video Recording Setup"
#### Combo Boxes (5):
| Name | Label Text | Notes |
|------|-----------|-------|
| `captureContainer` | "Container:" | Runtime-populated |
| `videoCaptureCodec` | "Codec:" | |
| `videoCaptureFormat` | "Format:" | |
| `audioCaptureCodec` | "Codec:" | |
| `audioCaptureFormat` | "Format:" | |

#### SpinBoxes (4):
| Name | Min | Max | Step | Default | Suffix | Notes |
|------|-----|-----|------|---------|--------|-------|
| `videoCaptureBitrate` | 100 | 200000 | 100 | 420 | " kbps" | |
| `videoCaptureWidth` | 320 | 32768 | 16 | 640 | | |
| `videoCaptureHeight` | 240 | 32768 | 16 | 480 | | |
| `audioCaptureBitrate` | 16 | 2048 | 1 | 67 | " kbps" | |

#### Checkboxes (5):
| Name | Text |
|------|------|
| `enableVideoCapture` | "Capture Video" |
| `videoCaptureResolutionAuto` | "Auto" |
| `enableVideoCaptureArguments` | "Extra Arguments" |
| `enableAudioCapture` | "Capture Audio" |
| `enableAudioCaptureArguments` | "Extra Arguments" |

#### LineEdits (2):
| Name | Placeholder | Notes |
|------|------------|-------|
| `videoCaptureArguments` | (blank) | Extra ffmpeg arguments |
| `audioCaptureArguments` | (blank) | Extra ffmpeg arguments |

### Tab order: 19 stops

### MISSING from LumineSX2:
- Screenshot format selection (PNG/JPEG/WebP)
- Screenshot quality 1-100%
- Video capture container/codec/format selection
- Video bitrate 100-200,000 kbps
- Video resolution 320-32768 x 240-32768 with Auto checkbox
- Extra ffmpeg arguments for video AND audio
- Audio capture with codec/bitrate
- All capture UI completely absent in LumineSX2

---

## 10. GraphicsAdvancedSettingsTab.ui
**Layout**: QVBoxLayout with 3 QGroupBoxes

### GroupBox: "Advanced Options"
#### Combo Boxes (4):
| Name | Items |
|------|-------|
| `gsDownloadMode` | **5 modes**: "Accurate (Recommended)", "Accurate Force Full", "Disable Readbacks (Synchronize GS Thread)", "Unsynchronized (Non-Deterministic)", "Disabled (Ignore Transfers)" |
| `gsDumpCompression` | "Uncompressed", "LZMA (xz)", "Zstandard (zst)" |
| `texturePreloading` | "None", "Partial", "Full (Hash Cache)" |
| `exclusiveFullscreenControl` | "Automatic (Default)", "Disallowed", "Allowed" |

#### Checkboxes in advancedLayout (6):
| Row | Name | Text |
|-----|------|------|
| 1,0 | `rov` | "ROV" (already in HW Rendering tab) |
| 1,1 | `extendedUpscales` | "Extended Upscaling Multipliers" |
| 2,0 | `spinGPUDuringReadbacks` | "Spin GPU During Readbacks" |
| 2,1 | `spinCPUDuringReadbacks` | "Spin CPU During Readbacks" |
| 3,0 | `useBlitSwapChain` | "Use Blit Swap Chain" |
| 3,1 | `disableMailboxPresentation` | "Disable Mailbox Presentation" |

### GroupBox: "Frame Rate Options"
#### QDoubleSpinBoxes (2):
| Name | Min | Max | Step | Suffix |
|------|-----|-----|------|--------|
| `ntscFrameRate` | 10.0 | 300.0 | 0.01 | " Hz" |
| `palFrameRate` | 10.0 | 300.0 | 0.01 | " Hz" |

### GroupBox: "Debugging Options"
#### Combo Boxes (1):
| Name | Items |
|------|-------|
| `overrideTextureBarriers` | "Automatic (Default)", "Force Disabled", "Force Enabled" |

#### Checkboxes in debuggingOptionsLayout (5):
| Row | Name | Text |
|-----|------|------|
| 0,0 | `useDebugDevice` | "Use Debug Device" |
| 0,1 | `disableFramebufferFetch` | "Disable Framebuffer Fetch" |
| 1,0 | `disableShaderCache` | "Disable Shader Cache" |
| 1,1 | `disableVertexShaderExpand` | "Disable Vertex Shader Expand" |
| 2,0 | `useDebugBlend` | "Use Debug Blend" |

### Tab order: 20 stops

### MISSING from LumineSX2:
- **GS Hardware Download Mode** — 5 modes (completely absent from LumineSX2)
- **GS Dump Compression** — 3 modes
- **Texture Preloading** — 3 modes (None/Partial/Full)
- **Exclusive Fullscreen** — 3-state
- **Extended Upscaling Multipliers** checkbox
- **Spin GPU/CPU During Readbacks** checkboxes
- **Use Blit Swap Chain** checkbox
- **Disable Mailbox Presentation** checkbox
- **NTSC/PAL Frame Rate** double spinboxes (10-300 Hz, step 0.01)
- **Override Texture Barriers** — 3-state
- **Use Debug Device, Disable Framebuffer Fetch, Disable Shader Cache** — 5 debug checkboxes

---

## 11. EmulationSettingsWidget.ui (cross-reference)
**Layout**: QVBoxLayout with 4 QGroupBoxes

### Speed Control Group:
- normalSpeed, fastForwardSpeed, slowMotionSpeed (QComboBoxes, runtime-populated)

### System Settings Group:
| Name | Widget | Items/Text |
|------|--------|------------|
| `eeCycleRate` | QComboBox | 50%, 60%, 75%, 100%, 130%, 180%, 300% |
| `eeCycleSkipping` | QComboBox | Disabled, Mild, Moderate, Maximum Underclock |
| `MTVU` | QCheckBox | "Enable Multithreaded VU1 (MTVU)" |
| `threadPinning` | QCheckBox | "Enable Thread Pinning" |
| `cheats` | QCheckBox | "Enable Cheats" |
| `hostFilesystem` | QCheckBox | "Enable Host Filesystem" |
| `fastCDVD` | QCheckBox | "Enable Fast CDVD" |
| `precacheCDVD` | QCheckBox | "Enable CDVD Precaching" |

### Frame Pacing Group:
| Name | Widget | Items/Text |
|------|--------|------------|
| `maxFrameLatency` | QSpinBox | 1-5, suffix " frames" |
| `optimalFramePacing` | QCheckBox | "Optimal Frame Pacing" |
| `syncToHostRefreshRate` | QCheckBox | "Sync to Host Refresh Rate" |
| `vsync` | QCheckBox | "Vertical Sync (VSync)" |
| `useVSyncForTiming` | QCheckBox | "Use Host VSync Timing" |
| `skipPresentingDuplicateFrames` | QCheckBox | "Skip Presenting Duplicate Frames" |

### Real-Time Clock Group:
| Name | Widget | Text |
|------|--------|------|
| `manuallySetRealTimeClock` | QCheckBox | "Manually Set Real-Time Clock" |
| `rtcDateTime` | **QDateTimeEdit** | Full datetime picker (not 6 separate text fields!) |
| `rtcUseSystemLocaleFormat` | QCheckBox | "Use System Locale Format" |

### MISSING from Emulation LumineSX2:
- EE Cycle Rate: 50%, 60%, 75% underclock options (LumineSX2 only has -3 to +3 relative)
- EE Cycle Skipping: "Underclock" labeling (LumineSX2 uses "Moderate", "Maximum")
- MTVU checkbox: "Enable Multithreaded VU1" (clearer label)
- Cheats checkbox in emulation (LumineSX2 has it in CheatsSettingsView)
- Fast CDVD: "Enable Fast CDVD" with "Enable" prefix
- CDVD Precaching checkbox
- RTC uses **QDateTimeEdit** (not 6 separate TextFieldRows)
- "Use System Locale Format" checkbox for RTC
- Max Frame Latency QSpinBox (1-5 frames) vs LumineSX2 ComboBox
- "frames" suffix on Max Frame Latency
- "Skip Presenting Duplicate Frames" vs LumineSX2 "Skip Duplicate Frames"

---

## Summary of Features NOT in LumineSX2 (discovered from .ui files)

| Tab | Missing Count | Key Missing Features |
|-----|--------------|---------------------|
| Display | 12 | Fullscreen modes, 10 deinterlace modes, Crop px, Anti-Blur Ctrl+S, Integer Scaling, Overscan, FMV Aspect Override |
| HW Rendering | 8 | Dithering 4-mode, Blending 6-level, Accurate Alpha, AA1, ROV, HW Fixes toggle |
| HW Fixes | 20 | 11 all-new combo/checkbox widgets (CPU Sprite BW, Limit Depth, Draw Buffering, etc.) |
| Upscaling Fixes | 8 | Native Scaling 5-mode, Texture Offset X/Y, Align/Merge Sprite, Force Even |
| SW Rendering | 1 | " threads" suffix, Bilinear (excl. sprite) |
| Post-Processing | 14 | CAS (3-mode + sharpness), FXAA, TV Shader 8-mode, Gamma, QSpinBox vs Slider |
| Texture Replacement | 5 | Async loading, Dump Mipmaps/FMV, Precache, Browse/Open/Reset buttons |
| Media Capture | 14 | Screenshot format/quality, Video/audio codec/bitrate/resolution, ffmpeg args |
| Advanced | 21 | GS Download 5-mode, GS Dump Compression, Texture Preload 3-mode, Exclusive FS, Frame Rate Hz, Debug options 5 |
| Emulation Widget | 7 | RTC QDateTimeEdit, "Enable" prefix on all checkboxes, System Locale, Cheats in emulation |

**Total new features discovered: ~110 widgets/properties not in LumineSX2 Graphics settings.**
