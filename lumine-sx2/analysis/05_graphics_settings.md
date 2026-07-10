# PCSX2 Graphics Settings — Complete Analysis

Source: `GraphicsSettingsWidget.h`, `GraphicsSettingsWidget.cpp`

## Architecture

10 UI tabs, each a separate `.ui` form:
- **GraphicsSettingsHeader** — Global: renderer dropdown, adapter dropdown
- **GraphicsDisplaySettingsTab** — Display options
- **GraphicsHardwareRenderingSettingsTab** — HW renderer
- **GraphicsSoftwareRenderingSettingsTab** — SW renderer
- **GraphicsHardwareFixesSettingsTab** — HW fixes (conditional)
- **GraphicsUpscalingFixesSettingsTab** — Upscaling fixes (conditional)
- **GraphicsTextureReplacementSettingsTab** — Texture replacement
- **GraphicsPostProcessingSettingsTab** — Post-processing
- **GraphicsMediaCaptureSettingsTab** — Video/audio capture
- **GraphicsAdvancedSettingsTab** — Advanced (hidden unless advanced settings enabled)

---

## 1. HEADER (Global)

| Setting | Key | Type | Default | Values/Notes |
|---------|-----|------|---------|--------------|
| Renderer | `EmuCore/GS/Renderer` | Enum | Auto | Auto, DX11(Win), DX12(Win), OpenGL, Vulkan, Metal(macOS), SW, Null |
| Adapter | `EmuCore/GS/Adapter` | String | Default | Populated dynamically per renderer |
| Fullscreen Mode | `EmuCore/GS/FullscreenMode` | String | Borderless | Populated from adapter info |

---

## 2. DISPLAY TAB

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| Aspect Ratio | `EmuCore/GS/AspectRatio` | Enum | Auto 4:3/3:2 Progressive | Auto/4:3/16:9/Custom |
| FMV Aspect Ratio | `EmuCore/GS/FMVAspectRatioSwitch` | Enum | Off | Overrides aspect for FMVs |
| Deinterlacing | `EmuCore/GS/deinterlace_mode` | Int | 0 (Auto) | Multiple modes |
| Bilinear Filtering | `EmuCore/GS/linear_present_mode` | Enum | BilinearSmooth | Post-process bilinear |
| Widescreen Patches | `EmuCore/EnableWideScreenPatches` | Bool | false | Migrated to Patches in per-game |
| No-Interlacing Patches | `EmuCore/EnableNoInterlacingPatches` | Bool | false | Migrated to Patches in per-game |
| Integer Scaling | `EmuCore/GS/IntegerScaling` | Bool | false | Integer pixel ratio |
| Screen Offsets (PCRTC) | `EmuCore/GS/pcrtc_offsets` | Bool | false | Position screen as game requests |
| Show Overscan | `EmuCore/GS/pcrtc_overscan` | Bool | false | Show overscan area |
| Anti-Blur | `EmuCore/GS/pcrtc_antiblur` | Bool | true | Internal anti-blur hacks |
| Disable Interlace Offset | `EmuCore/GS/disable_interlace_offset` | Bool | false | Reduce blurring |
| Vertical Stretch | `EmuCore/GS/StretchY` | Float | 100.0 | Stretch/squash vertical |
| Crop Left | `EmuCore/GS/CropLeft` | Int | 0 | Pixels cropped from left |
| Crop Top | `EmuCore/GS/CropTop` | Int | 0 | Pixels cropped from top |
| Crop Right | `EmuCore/GS/CropRight` | Int | 0 | Pixels cropped from right |
| Crop Bottom | `EmuCore/GS/CropBottom` | Int | 0 | Pixels cropped from bottom |

---

## 3. HARDWARE RENDERING TAB

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| Internal Resolution | `EmuCore/GS/upscale_multiplier` | Float | 1.0 | 1x-25x Native (12x max without extended) |
| Texture Filtering | `EmuCore/GS/filter` | Int | Bilinear PS2 | Nearest/Bilinear Forced/Bilinear PS2/Forced Excl Sprites |
| Trilinear Filtering | `EmuCore/GS/TriFilter` | Enum | Automatic | Off/PS2/Forced |
| Anisotropic Filtering | `EmuCore/GS/MaxAnisotropy` | Enum | Off | Off/2x/4x/8x/16x |
| Dithering | `EmuCore/GS/dithering_ps2` | Int | 2 (Unscaled) | Off/Scaled/Unscaled/Force 32bit |
| Mipmapping | `EmuCore/GS/hw_mipmap` | Bool | true | Progressive texture LOD |
| Accurate Alpha Test | `EmuCore/GS/HWAccurateAlphaTest` | Bool | false | More draw calls |
| AA1 (PS2 Antialiasing) | `EmuCore/GS/HWAA1` | Bool | false | Heavy perf penalty |
| Rasterizer Ordered View | `EmuCore/GS/HWROV` | Bool | false | Feedback loops, Vulkan only |
| Blending Accuracy | `EmuCore/GS/accurate_blending_unit` | Enum | Basic | Off/Basic/Medium/High/Full/Maximum(ultra) |
| Enable HW Fixes | `EmuCore/GS/UserHacks` | Bool | false | Manual HW fixes (disabled globally) |

---

## 4. SOFTWARE RENDERING TAB

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| Texture Filtering | `EmuCore/GS/filter` | Int | Bilinear PS2 | Same key as HW, synced |
| Extra SW Threads | `EmuCore/GS/extrathreads` | Int | 2 | 0-8, multithreading |
| Auto Flush | `EmuCore/GS/autoflush_sw` | Bool | true | Fix shadows (Jak), radiosity (GTA:SA) |
| Mipmapping | `EmuCore/GS/mipmap` | Bool | true | SW mipmapping |

---

## 5. HARDWARE FIXES TAB (Conditional: only when HW Fixes enabled)

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| CPU Sprite Render BW | `EmuCore/GS/UserHacks_CPUSpriteRenderBW` | Int | 0 (Disabled) | Max target memory width |
| CPU Sprite Render Level | `EmuCore/GS/UserHacks_CPUSpriteRenderLevel` | Int | 0 | Sub-setting of above |
| Software CLUT Render | `EmuCore/GS/UserHacks_CPUCLUTRender` | Int | 0 (Disabled) | CPU palette rendering |
| GPU Target CLUT Mode | `EmuCore/GS/UserHacks_GPUTargetCLUTMode` | Int | 0 (Disabled) | GPU palette handling |
| Skip Draw Range Start | `EmuCore/GS/UserHacks_SkipDraw_Start` | Int | 0 | Skip drawing surfaces |
| Skip Draw Range End | `EmuCore/GS/UserHacks_SkipDraw_End` | Int | 0 | End of skip range |
| Auto Flush | `EmuCore/GS/UserHacks_AutoFlushLevel` | Int | 0 | HW auto flush |
| Framebuffer Conversion | `EmuCore/GS/UserHacks_CPU_FB_Conversion` | Bool | false | 4/8-bit on CPU (Harry Potter, Stuntman) |
| Disable Depth Conversion | `EmuCore/GS/UserHacks_DisableDepthSupport` | Bool | false | Debug only |
| Disable Safe Features | `EmuCore/GS/UserHacks_Disable_Safe_Features` | Bool | false | Xenosaga, Kingdom Hearts |
| Disable Render Fixes | `EmuCore/GS/UserHacks_DisableRenderFixes` | Bool | false | Disable game-specific fixes |
| Preload Frame Data | `EmuCore/GS/preload_frame_with_gs_data` | Bool | false | Upload GS data on new frame |
| Disable Partial Invalidation | `EmuCore/GS/UserHacks_DisablePartialInvalidation` | Bool | false | Snowblind engine |
| Texture Inside RT | `EmuCore/GS/UserHacks_TextureInsideRt` | Enum | Disabled | Disabled/InsideTarget |
| Limit 24-bit Depth | `EmuCore/GS/UserHacks_Limit24BitDepth` | Enum | Disabled | Z-fighting fix |
| Read Targets When Closing | `EmuCore/GS/UserHacks_ReadTCOnClose` | Bool | false | Flush targets on shutdown |
| Estimate Texture Region | `EmuCore/GS/UserHacks_EstimateTextureRegion` | Bool | false | Snowblind games |
| Draw Buffering | `EmuCore/GS/UserHacks_DrawBuffering` | Bool | false | Reduce draw calls |
| GPU Palette Conversion | `EmuCore/GS/paltex` | Bool | false | GPU vs CPU colormap |

---

## 6. UPSCALING FIXES TAB (Conditional: only when HW Fixes enabled)

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| Half Pixel Offset | `EmuCore/GS/UserHacks_HalfPixelOffset` | Int | 0 | Fix fog/bloom/blend alignment |
| Native Scaling | `EmuCore/GS/UserHacks_native_scaling` | Int | 0 | Native resolution scaling |
| Round Sprite | `EmuCore/GS/UserHacks_round_sprite_offset` | Int | 0 | Fix 2D sprite sampling |
| Bilinear Dirty Upscale | `EmuCore/GS/UserHacks_BilinearHack` | Int | 0 | Smooth textures on upscale |
| Texture Offsets X | `EmuCore/GS/UserHacks_TCOffsetX` | Int | 0 | ST/UV coordinate offset |
| Texture Offsets Y | `EmuCore/GS/UserHacks_TCOffsetY` | Int | 0 | ST/UV coordinate offset |
| Align Sprite | `EmuCore/GS/UserHacks_align_sprite_X` | Bool | false | Fix vertical lines (Namco games) |
| Merge Sprite | `EmuCore/GS/UserHacks_merge_pp_sprite` | Bool | false | Reduce upscaling lines |
| Force Even Sprite Position | `EmuCore/GS/UserHacks_forceEvenSpritePosition` | Bool | false | Fix Wild Arms text |
| Unscaled Palette Texture Draws | `EmuCore/GS/UserHacks_NativePaletteDraw` | Bool | false | Native resolution palette draws |

---

## 7. TEXTURE REPLACEMENT TAB

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| Dump Textures | `EmuCore/GS/DumpReplaceableTextures` | Bool | false | Dump to disk |
| Dump Mipmaps | `EmuCore/GS/DumpReplaceableMipmaps` | Bool | false | Include mipmaps |
| Dump FMV Textures | `EmuCore/GS/DumpTexturesWithFMVActive` | Bool | false | Dump during FMVs (not recommended) |
| Load Textures | `EmuCore/GS/LoadTextureReplacements` | Bool | false | Load user replacements |
| Async Texture Loading | `EmuCore/GS/LoadTextureReplacementsAsync` | Bool | true | Worker thread loading |
| Precache Textures | `EmuCore/GS/PrecacheTextureReplacements` | Bool | false | Preload all to memory |
| Textures Directory | `Folders/Textures` | Path | `{data}/textures` | Browse/Open/Reset UI |

---

## 8. POST-PROCESSING TAB

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| FXAA | `EmuCore/GS/fxaa` | Bool | false | Anti-aliasing |
| Shade Boost | `EmuCore/GS/ShadeBoost` | Bool | false | Enable brightness/contrast/saturation/gamma |
| Brightness | `EmuCore/GS/ShadeBoost_Brightness` | Int | 50 | 0-100 |
| Contrast | `EmuCore/GS/ShadeBoost_Contrast` | Int | 50 | 0-100 |
| Gamma | `EmuCore/GS/ShadeBoost_Gamma` | Int | 50 | 0-100 |
| Saturation | `EmuCore/GS/ShadeBoost_Saturation` | Int | 50 | 0-100 |
| TV Shader | `EmuCore/GS/TVShader` | Int | 0 (None) | CRT effects |
| CAS Mode | `EmuCore/GS/CASMode` | Enum | None | FidelityFX Contrast Adaptive Sharpening |
| CAS Sharpness | `EmuCore/GS/CASSharpness` | Int | 50 | 0-100% |

---

## 9. MEDIA CAPTURE TAB

### Video Capture
| Setting | Key | Type | Default |
|---------|-----|------|---------|
| Enable Video Capture | `EmuCore/GS/EnableVideoCapture` | Bool | true |
| Video Codec | `EmuCore/GS/VideoCaptureCodec` | String | Default |
| Video Format | `EmuCore/GS/VideoCaptureFormat` | String | Default |
| Video Bitrate | `EmuCore/GS/VideoCaptureBitrate` | Int | 6000 kbps |
| Video Width | `EmuCore/GS/VideoCaptureWidth` | Int | Default |
| Video Height | `EmuCore/GS/VideoCaptureHeight` | Int | Default |
| Auto Resolution | `EmuCore/GS/VideoCaptureAutoResolution` | Bool | true |
| Enable Extra Arguments | `EmuCore/GS/EnableVideoCaptureParameters` | Bool | false |
| Extra Arguments | `EmuCore/GS/VideoCaptureParameters` | String | "" |

### Audio Capture
| Setting | Key | Type | Default |
|---------|-----|------|---------|
| Enable Audio Capture | `EmuCore/GS/EnableAudioCapture` | Bool | true |
| Audio Codec | `EmuCore/GS/AudioCaptureCodec` | String | Default |
| Audio Bitrate | `EmuCore/GS/AudioCaptureBitrate` | Int | 192 kbps |
| Enable Extra Arguments | `EmuCore/GS/EnableAudioCaptureParameters` | Bool | false |
| Extra Arguments | `EmuCore/GS/AudioCaptureParameters` | String | "" |

### Screenshot
| Setting | Key | Type | Default |
|---------|-----|------|---------|
| Screenshot Size | `EmuCore/GS/ScreenshotSize` | Enum | WindowResolution |
| Screenshot Format | `EmuCore/GS/ScreenshotFormat` | Enum | PNG |
| Screenshot Quality | `EmuCore/GS/ScreenshotQuality` | Int | 90 |

### Container
| Setting | Key | Type | Default |
|---------|-----|------|---------|
| Capture Container | `EmuCore/GS/CaptureContainer` | String | Default |

---

## 10. ADVANCED TAB (Hidden unless "Show Advanced Settings" enabled)

| Setting | Key | Type | Default | Notes |
|---------|-----|------|---------|-------|
| GS Download Mode | `EmuCore/GS/HWDownloadMode` | Enum | Enabled | Disabled globally, per-game only |
| Texture Preloading | `EmuCore/GS/texture_preloading` | Enum | Full (Hash Cache) | Off/Partial/Full |
| NTSC Frame Rate | `EmuCore/GS/FrameRateNTSC` | Float | 59.94 | Custom NTSC framerate |
| PAL Frame Rate | `EmuCore/GS/FrameRatePAL` | Float | 50.00 | Custom PAL framerate |
| Use Blit Swap Chain | `EmuCore/GS/UseBlitSwapChain` | Bool | false | DX11 only, streaming apps |
| Exclusive Fullscreen Control | `EmuCore/GS/ExclusiveFullscreenControl` | Int | -1 (Auto) | Windows+Vulkan only |
| Override Texture Barriers | `EmuCore/GS/OverrideTextureBarriers` | Int | -1 (Auto) | Not Metal/SW |
| Disable Framebuffer Fetch | `EmuCore/GS/DisableFramebufferFetch` | Bool | false | Not SW/DX |
| Disable Shader Cache | `EmuCore/GS/DisableShaderCache` | Bool | false | |
| Disable Vertex Shader Expand | `EmuCore/GS/DisableVertexShaderExpand` | Bool | false | |
| GS Dump Compression | `EmuCore/GS/GSDumpCompression` | Enum | Zstandard | Compression algorithm |
| Extended Upscaling Multipliers | `EmuCore/GS/ExtendedUpscalingMultipliers` | Bool | false | >12x if GPU supports |
| Use Debug Device | `EmuCore/GS/UseDebugDevice` | Bool | false | API validation |
| Use Debug Blend | `EmuCore/GS/UseDebugBlend` | Bool | false | Force SW blending |
| Disable Mailbox Presentation | `EmuCore/GS/DisableMailboxPresentation` | Bool | false | Double buffer vs triple |
| Spin CPU During Readbacks | `EmuCore/GS/HWSpinCPUForReadbacks` | Bool | false | Prevent CPU powersave |
| Spin GPU During Readbacks | `EmuCore/GS/HWSpinGPUForReadbacks` | Bool | false | Prevent GPU powersave |
| ROV Barriers Vulkan | `EmuCore/GS/HWROVBarriersVK` | Bool | false | Extra barriers for ROV+VK |

---

## HIDDEN/SPECIAL BEHAVIORS

1. **Tab visibility is dynamic**: HW/SW rendering tabs swap based on renderer selection
2. **HW Fixes + Upscaling Fixes tabs**: Only visible when "Enable HW Fixes" is checked
3. **Texture Replacement tab**: Only visible for hardware renderers
4. **Advanced tab**: Hidden unless "Show Advanced Settings" is enabled
5. **Per-game settings**: "Use Global Setting" option added to renderer, adapter, fullscreen mode, upscale multiplier
6. **Widescreen/No-Interlace**: Migrated from checkboxes to Patches system in per-game settings
7. **HW Fixes globally disabled**: `enableHWFixes` checkbox removed for global settings (only per-game)
8. **GS Download mode**: Only available in per-game settings (too dangerous globally)
9. **Extended upscaling**: Only enabled if adapter supports >12x and checkbox is checked
10. **Trilinear Forced disables Texture Filtering dropdown**
11. **GPU Palette Conversion disables Anisotropic Filtering**
12. **CPU Sprite Render BW=0 disables CPU Sprite Render Level**
13. **Shade Boost disabled → brightness/contrast/gamma/saturation sliders disabled**
14. **Texture Dump disabled → mipmap dump + FMV dump disabled**
15. **Texture Replacement disabled → async loading + precache disabled**
16. **Video Capture disabled → all video options disabled**
17. **Audio Capture disabled → all audio options disabled**
18. **Auto Resolution enabled → width/height disabled**
19. **Enable Extra Arguments disabled → arguments text disabled**
20. **Fullscreen modes populated from adapter info**
21. **Adapter list populated dynamically per renderer**
22. **Capture codecs populated dynamically per container**

---

## TOTAL SETTINGS COUNT: **~120 individual settings**
