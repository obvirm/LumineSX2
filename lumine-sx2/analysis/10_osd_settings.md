# OSD Settings — Deep Analysis

## Source Files
- `OSDSettingsWidget.h` / `.cpp` — Main OSD settings tab
- `OsdFontPickerDialog.h` / `.cpp` — Full-featured font picker dialog

---

## 1. OSD Settings Widget (OSDSettingsWidget)

### 1.1 Scale & Layout
| Setting | Config Key | Type | Default | Range | Description |
|---------|-----------|------|---------|-------|-------------|
| OSD Scale | `EmuCore/GS/OsdScale` | float | 100.0 | 50–500% | Scales all OSD elements |
| OSD Margin | `EmuCore/GS/OsdMargin` | float | 10.0 | px | Distance from screen edges |
| OSD Font Path | `EmuCore/GS/OsdFontPath` | string | (empty=RobotoMono-Medium.ttf) | file path | Custom font file |
| Bold Text | `EmuCore/GS/OsdBoldText` | bool | true | — | Heavier font weight |

### 1.2 Position Selectors (ComboBox)
| Setting | Config Key | Default | Options |
|---------|-----------|---------|---------|
| Messages Position | `EmuCore/GS/OsdMessagesPos` | TopLeft (0) | TopLeft, TopRight, BottomLeft, BottomRight |
| Performance Position | `EmuCore/GS/OsdPerformancePos` | TopRight (1) | TopLeft, TopRight, BottomLeft, BottomRight |

**Behavior**: When messages position changes → disables/enables "Warn About Unsafe Settings" checkbox. When performance position changes → disables/enables ALL performance OSD checkboxes.

### 1.3 Performance Overlay Toggles (22 total)
| # | Toggle | Config Key | Default | Description |
|---|--------|-----------|---------|-------------|
| 1 | Show Speed Percentages | `OsdShowSpeed` | false | Current emulation speed % |
| 2 | Show FPS | `OsdShowFPS` | false | Internal video frames/sec |
| 3 | Show VPS | `OsdShowVPS` | false | Vsyncs per second |
| 4 | Show Resolution | `OsdShowResolution` | false | Internal game resolution |
| 5 | Show GS Stats | `OsdShowGSStats` | false | Primitives, draw calls |
| 6 | Show CPU Usage | `OsdShowCPU` | false | Host CPU utilization |
| 7 | Show GPU Usage | `OsdShowGPU` | false | Host GPU utilization |
| 8 | Show Debug GPU | `OsdShowGPUDebug` | false | Renderer debug info (Win-only) |
| 9 | Show Status Indicators | `OsdShowIndicators` | **true** | Pausing/Turbo/FF/Slow-Mo icons |
| 10 | Show Frame Times | `OsdShowFrameTimes` | false | Frametime graph |
| 11 | Show Hardware Info | `OsdShowHardwareInfo` | false | CPU+GPU model info |
| 12 | Show Version | `OsdShowVersion` | false | PCSX2 version string |
| 13 | Show Settings | `OsdShowSettings` | false | Active settings summary |
| 14 | Show Patches | `OsdshowPatches` | false | Active patches/cheats count |
| 15 | Show Inputs | `OsdShowInputs` | false | Controller state visualization |
| 16 | Show Video Capture | `OsdShowVideoCapture` | **true** | Video capture status |
| 17 | Show Input Recording | `OsdShowInputRec` | **true** | Input recording status |
| 18 | Show Texture Replacements | `OsdShowTextureReplacements` | false | Dumped/loaded texture count |
| 19 | Warn About Unsafe Settings | `OsdWarnAboutUnsafeSettings` | **true** | Warning for unstable options |

### 1.4 Hidden Feature: Select All / Deselect All
- **Select All** button — enables all 14+ checkboxes
- **Deselect All** button — disables all 14+ checkboxes
- **Exception**: Status Indicators, Video Capture, Input Recording, Warn About Unsafe → always stay checked
- **Win-only**: Show Debug GPU included in select/deselect

### 1.5 Font Picker Button Flow
- **Browse** → opens `OSDFontPickerDialog` (non-modal, WA_DeleteOnClose)
- **Clear** → resets font path to default (RobotoMono-Medium.ttf)
- Font path stored in `EmuCore/GS/OsdFontPath`

---

## 2. Font Picker Dialog (OSDFontPickerDialog) — HIDDEN FEATURE

This is a **massive hidden feature** — a full Google Fonts browser built into PCSX2.

### 2.1 Architecture
```
┌─────────────────────────────────────────────┐
│  Tab Widget                                 │
│  ┌──────────────┬──────────────────────────┐ │
│  │ System Fonts │ Online Catalog           │ │
│  │  (local)     │  (Google Fonts)          │ │
│  └──────────────┴──────────────────────────┘ │
│  ┌──────────────────────────────────────────┐│
│  │ Search + Filter                          ││
│  │ Family List (left) │ Info Panel (right)  ││
│  └──────────────────────────────────────────┘│
│  ┌──────────────────────────────────────────┐│
│  │ Preview Area (sample OSD messages)       ││
│  └──────────────────────────────────────────┘│
│  [Choose Local] [Use Default] [Download] [OK]│
└─────────────────────────────────────────────┘
```

### 2.2 Font Sources (3 tabs)
1. **System Fonts** — Lists all installed OS fonts via `QFontDatabase::families()`
2. **Online Catalog** — Downloads Google Fonts metadata from `fontsource/google-font-metadata`
3. **Local File** — Browse for .ttf/.otf/.ttc/.otc files

### 2.3 Online Catalog Features
- Downloads `google-fonts-v2.json` from GitHub (fontsource project)
- Parses 1500+ font families with metadata (family, category, license, download URL)
- Downloads `licenses.json` for license metadata
- Filters: search text, "show downloaded only"
- Font categories: sans-serif, serif, display, handwriting, monospace
- License display with links to original copyright

### 2.4 Font Download System
- Downloads from Google Fonts CDN (`fonts.gstatic.com`)
- Cache directory: `<EmuFolders::Cache>/fonts/`
- File naming: `<SanitizedFamilyName>-Regular.ttf`
- Writes `.LICENSE.txt` sidecar file with:
  - Font family name
  - Source URL (Google Fonts specimen page)
  - Download URL
  - License type + URL
  - Copyright/original text
- Validates downloaded font before committing

### 2.5 System Font Resolution (Platform-Specific)
| Platform | Method | API |
|----------|--------|-----|
| Windows | Registry lookup | `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts` |
| Linux | Fontconfig | `FcFontMatch()` |
| macOS | CoreText | `CTFontCopyAttribute(kCTFontURLAttribute)` |
| Fallback | QStandardPaths | `FontsLocation` scan |

### 2.6 Preview System
- Sample text: save state messages, GS dump messages, speed change
- Font applied in real-time as you browse
- Bold preview respects current "Bold Text" checkbox
- Uses `QFontDatabase::addApplicationFont()` for preview, removes on close

### 2.7 UI Elements
| Element | Type | Purpose |
|---------|------|---------|
| `search` | QLineEdit | Filter catalog families |
| `systemSearch` | QLineEdit | Filter system families |
| `showDownloadedOnly` | QCheckBox | Show only cached fonts |
| `familyList` | QListWidget | Catalog font list |
| `systemFamilyList` | QListWidget | System font list |
| `familyInfo` | QLabel (RichText) | Family/category/license/copyright info |
| `selectedPath` | QLabel | Currently selected font path |
| `preview` | QLabel | Live font preview |
| `status` | QLabel | Status messages |
| `catalogInfo` | QLabel | Catalog family count |
| `systemInfo` | QLabel | System family count |
| `refreshCatalog` | QPushButton | Force re-download catalog |
| `downloadSelected` | QPushButton | Download selected font |
| `chooseLocal` | QPushButton | Browse local font file |
| `useDefault` | QPushButton | Reset to bundled font |

### 2.8 Catalog Data Structure
```json
{
  "font-id": {
    "family": "Roboto Mono",
    "category": "monospace",
    "defSubset": "latin",
    "variants": {
      "400": {
        "normal": {
          "latin": {
            "url": { "truetype": "...", "opentype": "...", "woff2": "..." }
          }
        }
      }
    }
  }
}
```

### 2.9 License Index Structure
```json
{
  "font-id": {
    "license": { "type": "OFL", "url": "...", "original": "Copyright ..." }
  }
}
```

---

## 3. Features NOT in LumineSX2-Ori

| Feature | Priority | Complexity |
|---------|----------|------------|
| OSD Scale (50-500%) | HIGH | Simple SliderRow |
| OSD Margin (px) | HIGH | Simple SliderRow |
| Font Picker Dialog | MEDIUM | Complex (separate component) |
| Bold Text toggle | HIGH | Simple ToggleRow |
| 22 OSD toggles | HIGH | Already partially done |
| Messages Position | MEDIUM | ComboBox |
| Performance Position | MEDIUM | ComboBox |
| Select All / Deselect All | LOW | 2 buttons |
| Show Debug GPU (Win-only) | LOW | Conditional toggle |
| Video Capture Status toggle | MEDIUM | ToggleRow |
| Input Recording Status toggle | MEDIUM | ToggleRow |
| Texture Replacement Status toggle | MEDIUM | ToggleRow |
| GS Stats toggle | MEDIUM | ToggleRow |
| VPS toggle | MEDIUM | ToggleRow |
| Resolution toggle | MEDIUM | ToggleRow |
| GPU Usage toggle | MEDIUM | ToggleRow |
| Hardware Info toggle | MEDIUM | ToggleRow |
| Font download from Google Fonts | LOW | Complex |
| System font picker | LOW | Platform-specific |
| Font license management | LOW | Complex |

---

## 4. Current LumineSX2-Ori OSD Coverage

**Already implemented**: show-perf, perf-pos, show-notifications, notif-duration, show-fps, show-speed, show-cpu, show-frame-times, show-hw-info, show-version, show-settings-summary, show-patches-list, show-inputs, warn-unsafe

**Missing from LumineSX2-Ori**:
1. OSD Scale
2. OSD Margin  
3. Font Path + Font Picker
4. Bold Text
5. Show VPS
6. Show Resolution
7. Show GS Stats
8. Show GPU Usage
9. Show Debug GPU (Win-only)
10. Show Status Indicators
11. Show Video Capture
12. Show Input Recording
13. Show Texture Replacements
14. Messages Position (separate from Performance Position)
15. Select All / Deselect All buttons
