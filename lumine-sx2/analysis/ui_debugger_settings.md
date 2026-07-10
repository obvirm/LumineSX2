# UI Analysis — Debugger & Settings .ui Files

## Files Analyzed: 22 .ui XML files from Debugger/ and Settings/

---

## 1. BREAKPOINT DIALOG (BreakpointDialog.ui)
- **Window**: ApplicationModal, 375x300 fixed size
- **Title**: "Create / Modify Breakpoint"
- **BP Type**: QRadioButton group — `rdoMemory` (default checked) / `rdoExecute`
- **Address**: QLineEdit, disabled by default (enabled only when Execute selected), default text "0"
- **Description**: QLineEdit
- **Memory type**: QCheckBox Read (default checked), Write (default checked), Change
- **Size**: QLineEdit, default "1"
- **Condition**: QLineEdit (for Execute type)
- **Log**: QCheckBox, default checked
- **Enable**: QCheckBox, default checked
- **OK/Cancel**: QDialogButtonBox
- **NOT IMPLEMENTED IN LumineSX2** Condition field, Log/Enable separate checkboxes, Size as text field

## 2. BREAKPOINT VIEW (BreakpointView.ui)
- Empty QTableView with zero margins
- Populated programmatically

## 3. DISASSEMBLY VIEW (DisassemblyView.ui)
- Empty QWidget 400x300, title "Disassembly"
- All content generated in code

## 4. REGISTER VIEW (RegisterView.ui)
- QTabBar named `registerTabs` (tabs added in code)
- Zero-margin layout
- **Important**: Uses QTabBar with custom tabs, NOT QTabWidget

## 5. MEMORY VIEW (MemoryView.ui)
- Empty QWidget 400x300

## 6. MEMORY SEARCH (MemorySearchView.ui)
- **Search types**: 1 Byte, 2 Bytes, 4 Bytes, 8 Bytes, Float, Double, String, Byte Array
- **Comparisons**: Equals, Not Equals, Greater Than, Greater Than Or Equal, Less Than, Less Than Or Equal, Unknown Initial Value
- **Value**: QLineEdit
- **Hex toggle**: CheckBox (default checked)
- **Search range**: Start (default 0x00), End (default 0x2000000)
- **Filter Search**: Button (disabled by default, enabled after initial search)
- **Results**: QListWidget
- **Results count**: QLabel (hidden by default)
- **NOT IMPLEMENTED IN LumineSX2**: Byte Array type, Unknown Initial Value comparison, End address range, Filter Search button, Hex toggle

## 7. STACK VIEW (StackView.ui)
- QTableView with zero margins

## 8. THREAD VIEW (ThreadView.ui)
- QTableView with zero margins

## 9. MODULE VIEW (ModuleView.ui)
- QTableView with zero margins

## 10. SAVED ADDRESSES VIEW (SavedAddressesView.ui)
- QTableView with zero margins

## 11. SYMBOL TREE VIEW (SymbolTreeView.ui)
- QTreeView
- Bottom toolbar: Refresh button, Filter QLineEdit (placeholder "Filter"), New (+) button 26px, Delete (-) button 26px

## 12. NEW SYMBOL DIALOG (NewSymbolDialog.ui)
- **Size**: 600x400, min 300x200, max 600x400
- **QTabBar** `storageTabBar` (multiple storage backends)
- **Fields**: Name, Address, Register (ComboBox), Stack Pointer Offset (SpinBox max 268435456), Size (radio group), Existing Functions (radio group), Type (LineEdit), Function (ComboBox)
- **Size options**: Fill Existing Function (default), Fill Empty Space, Custom (SpinBox step 4, max 268435456)
- **Existing Functions**: Shrink to avoid overlaps (default), Do not modify
- **Error message**: QLabel with red stylesheet, word wrap

## 13. LAYOUT EDITOR (LayoutEditorDialog.ui)
- 400x150
- **Fields**: Name (LineEdit), Target/CPU (ComboBox), Initial State (ComboBox)
- Error label with red stylesheet

## 14. NO LAYOUTS WIDGET (NoLayoutsWidget.ui)
- "There are no layouts." centered label
- "Create Default Layouts" button centered
- AutoFillBackground false

## 15. DEBUGGER WINDOW (DebuggerWindow.ui) — CRITICAL
- **Size**: 1000x750
- **Icon**: AppIcon64.png
- **6 Menus**:
  - File: Analyze, Settings, Game Settings, separator, Close
  - Debug: Run, Step Into, Step Over, Step Out
  - Windows: [dynamic]
  - View: Always On Top (checkable, pin icon), separator, Increase Font Size, Decrease Font Size (Ctrl+-), Reset Font Size
  - Layouts: Reset All Layouts, Reset Default Layouts
  - Tools: [dynamic]
- **4 Toolbars** (all Top area, ToolButtonTextBesideIcon):
  1. Debug: Run, Step Into, Step Over, Step Out
  2. File: Analyze, Settings, Game Settings
  3. System: Shut Down, Reset
  4. View: On Top, Increase Font, Decrease Font, Reset Font
- **Shortcuts**: F11 (Step Into), F10 (Step Over), Shift+F11 (Step Out)
- **NOT IMPLEMENTED IN LumineSX2**: All 6 menus, all 4 toolbars, all shortcuts, Always On Top, Font size controls, Layout management actions

## 16. DEBUG UI SETTINGS (DebugUserInterfaceSettingsTab.ui)
- 700x600
- **Debugger Window** group:
  - Show On Startup checkbox
  - Save Window Geometry checkbox
  - Refresh Interval: SpinBox 10-100000ms, suffix "ms", default 1000
- **Docking** group:
  - Drop Indicator Style: ComboBox

## 17. DEBUG ANALYSIS SETTINGS (DebugAnalysisSettingsTab.ui)
- 700x500
- **Warning label**: "These settings control what and when analysis passes should be performed..."
- **Analysis** group:
  - Auto Analyze: Always / If Debugger Is Open / Never
  - Generate Symbols For IRX Exports checkbox
- Empty analysis settings widget (populated programmatically)

## 18. DEBUG GS SETTINGS (DebugGSSettingsTab.ui) — MASSIVE
- 700x500
- **Draw Dumping** group:
  - Dump GS Draws checkbox
  - Save Frame, Save RT, Save Depth, Save Texture, Save Alpha, Save Info — 6 checkboxes
  - Save Draw Stats, Save Frame Stats — 2 checkboxes
  - Save Transfer Image Data, Save HW Config — 2 checkboxes
  - Save Draw Start: SpinBox max 99999999
  - Save Draw Count: SpinBox min 1, max 99999999
  - Save Frame Start: SpinBox max 99999999
  - Save Frame Count: SpinBox min 1, max 99999999
  - HW Dump Directory: LineEdit + Browse/Open buttons
  - SW Dump Directory: LineEdit + Browse/Open buttons
- **NOT IMPLEMENTED IN LumineSX2**: ALL of the above

## 19. DEBUG LOGGING SETTINGS (DebugLoggingSettingsTab.ui) — MASSIVE
- 700x500
- **Enable** master checkbox
- **EE**: 20 checkboxes in grid:
  - COP0, COP1 (FPU), COP2 (VU0 Macro), R5900, Cache, Memory
  - HW Regs (MMIO), Unknown MMIO, DMA Registers, DMA Control
  - MSKPATH3, SPR/MFIFO, IPU, Counters
  - VIFCodes, GIFTags, VIF, GIF, BIOS, SIF
- **IOP**: 12 checkboxes in grid:
  - COP2 (GPU), R3000A, Memcards, Pad
  - DMA Registers, DMA Control, HW Regs (MMIO), Unknown MMIO
  - Counters, CDVD, MDEC, BIOS
- Total: 33 checkboxes!
- **NOT IMPLEMENTED IN LumineSX2**: ALL logging functionality

## 20. GAME PATCH DETAILS WIDGET (GamePatchDetailsWidget.ui)
- 541x112, Expanding/MinimumExpanding
- **Title**: QLabel, 16pt bold, word wrap
- **Enabled**: QCheckBox with "Enabled" text
- **Description**: QLabel with Rich HTML: `<b>Author:</b> Patch Author`, word wrap, align left-top

## 21. GRAPHICS DISPLAY SETTINGS (GraphicsDisplaySettingsTab.ui) — CRITICAL
- 700x500, Expanding/Preferred
- **Fullscreen Mode**: ComboBox
- **Aspect Ratio**: 5 options (Fit to Window, Auto Standard, Standard 4:3, Widescreen 16:9, Native/Full 10:7)
- **FMV Aspect Ratio Override**: 5 options (Off, Auto Standard, Standard 4:3, Widescreen 16:9, Native/Full 10:7)
- **Deinterlacing**: 10 options!
  - Automatic (Default), No Deinterlacing
  - Weave (Top/Bottom Field First, Sawtooth) — 2 variants
  - Bob (Top/Bottom Field First, Full Frames) — 2 variants
  - Blend (Top/Bottom Field First, Merge 2 Fields) — 2 variants
  - Adaptive (Top/Bottom Field First, Similar to Bob + Weave) — 2 variants
- **Bilinear Filtering**: None / Bilinear (Smooth) / Bilinear (Sharp)
- **Vertical Stretch**: SpinBox 1-300%, suffix "%"
- **Crop**: 4 SpinBoxes (Left, Right, Top, Bottom), suffix "px", max 1000
- **Checkboxes**: Widescreen Patches, No-Interlacing Patches, Anti-Blur (Ctrl+S), Integer Scaling, Screen Offsets, Disable Interlace Offset, Show Overscan
- **NOT IMPLEMENTED IN LumineSX2**: FMV Aspect Ratio Override, 10 deinterlacing modes, Crop (4 spinboxes), Bilinear Filtering (3 modes), Integer Scaling, Anti-Blur (PCRTC), Screen Offsets, Disable Interlace Offset, Show Overscan

## 22. GRAPHICS HARDWARE RENDERING (GraphicsHardwareRenderingSettingsTab.ui)
- 700x400
- **Internal Resolution**: ComboBox
- **Texture Filtering**: Nearest / Bilinear (Forced) / Bilinear (PS2) / Bilinear (Forced excluding sprite)
- **Trilinear Filtering**: Automatic / Off / Trilinear (PS2) / Trilinear (Forced)
- **Anisotropic Filtering**: ComboBox
- **Dithering**: Off / Scaled / Unscaled (Default) / Force 32bit
- **Blending Accuracy**: Minimum / Basic / Medium / High / Full / Maximum (Very Slow)
- **Checkboxes**: Mipmapping, Accurate Alpha Test, AA1, Rasterizer Ordered View, Manual Hardware Renderer Fixes
- **NOT IMPLEMENTED IN LumineSX2**: Dithering (4 modes), Blending Accuracy (6 levels), Accurate Alpha Test, AA1, ROV

## 23. GRAPHICS ADVANCED SETTINGS (GraphicsAdvancedSettingsTab.ui)
- **Advanced Options** group:
  - Hardware Download Mode: Accurate (Recommended) / Accurate Force Full / Disable Readbacks / Unsynchronized (Non-Deterministic) / Disabled (Ignore Transfers)
  - GS Dump Compression: Uncompressed / LZMA (xz) / Zstandard (zst)
  - Texture Preloading: None / Partial / Full (Hash Cache)
  - Exclusive Fullscreen: Automatic (Default) / Disallowed / Allowed
  - Checkboxes: ROV Barriers Vulkan, Extended Upscaling Multipliers, Spin GPU During Readbacks, Spin CPU During Readbacks, Use Blit Swap Chain, Disable Mailbox Presentation
- **Frame Rate Options** group:
  - NTSC Frame Rate: QDoubleSpinBox, suffix " Hz", 10-300 Hz, step 0.01
  - PAL Frame Rate: QDoubleSpinBox, suffix " Hz", 10-300 Hz, step 0.01
- **Debugging Options** group:
  - Override Texture Barriers: Automatic / Force Disabled / Force Enabled
  - Checkboxes: Use Debug Device, Disable Framebuffer Fetch, Disable Shader Cache, Disable Vertex Shader Expand, Use Debug Blend
- **NOT IMPLEMENTED IN LumineSX2**: ALL of the above

## 24. GRAPHICS HARDWARE FIXES (GraphicsHardwareFixesSettingsTab.ui)
- **CPU Sprite Render Size**: Combo 0-10 (Disabled to 640 Max Width) + Level (Sprites Only/Sprites-Triangles/Blended Sprites-Triangles)
- **Software CLUT Render**: 0 (Disabled) / 1 (Normal) / 2 (Aggressive)
- **GPU Target CLUT**: Disabled / Enabled (Exact Match) / Enabled (Check Inside Target)
- **Auto Flush**: Disabled / Enabled (Sprites Only) / Enabled (All Primitives)
- **Texture Inside RT**: Disabled / Inside Target / Merge Targets
- **Skip Draw Range**: Start + End SpinBoxes max 10000
- **Limit Depth to 24 Bits**: Disabled / Prioritize Upper Bits / Prioritize Lower Bits
- **Checkboxes**: Disable Depth Conversion, Framebuffer Conversion, Disable Partial Source Invalidation, GPU Palette Conversion, Disable Safe Features, Preload Frame Data, Disable Render Fixes, Read Targets When Closing, Estimate Texture Region, Draw Buffering
- **NOT IMPLEMENTED IN LumineSX2**: CPU Sprite Render (11+3 variants), GPU Target CLUT (3 modes), Auto Flush (3 modes), Limit Depth to 24 Bits (3 modes), Texture Inside RT (3 modes), 10 checkboxes

## 25. GRAPHICS UPSCALING FIXES (GraphicsUpscalingFixesSettingsTab.ui)
- **Half Pixel Offset**: 6 modes (Off, Normal/Vertex, Special/Texture, Special/Texture-Aggressive, Align to Native, Align to Native with Texture Offset)
- **Native Scaling**: 5 modes (Off, Normal, Aggressive, Normal Maintain, Aggressive Maintain)
- **Round Sprite**: Off / Half / Full
- **Bilinear Dirty Upscale**: Automatic / Force Bilinear / Force Nearest
- **Texture Offsets**: X + Y SpinBoxes max 1000
- **Checkboxes**: Align Sprite, Unscaled Palette Texture Draws, Merge Sprite, Force Even Sprite Position
- **NOT IMPLEMENTED IN LumineSX2**: Native Scaling (5 modes), Bilinear Dirty Upscale (3 modes), Texture Offsets (X/Y), Align Sprite, Merge Sprite, Force Even Sprite Position, Unscaled Palette Texture Draws

## 26. GRAPHICS MEDIA CAPTURE (GraphicsMediaCaptureSettingsTab.ui)
- **Screenshot** group:
  - Resolution: Display Resolution (Aspect Corrected) / Internal Resolution (Aspect Corrected) / Internal Resolution (No Aspect Correction)
  - Format: PNG / JPEG / WebP
  - Quality: SpinBox 1-100%, suffix "%"
- **Video Recording** group:
  - Container: ComboBox
  - Video Capture: checkbox, then Codec, Format, Bitrate (100-200000 kbps, step 100, default 420), Resolution (Width 320-32768 step 16, Height 240-32768 step 16, Auto checkbox), Extra Arguments (checkbox + LineEdit)
  - Audio Capture: checkbox, then Codec, Bitrate (16-2048 kbps, step 1, default 67), Extra Arguments (checkbox + LineEdit)
- **NOT IMPLEMENTED IN LumineSX2**: ALL media capture settings (full video/audio recording pipeline)

---

## SUMMARY: Total new features discovered from .ui files only = ~150

All documented above are NOT in the previous 50-agent .cpp/.h analysis and NOT in the current LumineSX2 UI implementation.
