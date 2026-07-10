# Agent 50: Misc Utilities — Deep Analysis

## 1. QtUtils.h / QtUtils.cpp — General Utility Functions

### Core UI Utilities
| Function | Description | UI Relevance |
|----------|-------------|--------------|
| `MarkActionAsDefault(QAction*)` | Makes action text **bold** (visually marks default action) | Menu items — "default" visual indicator |
| `CreateHorizontalLine(QWidget*)` | Creates `QFrame` with `HLine` + `Sunken` shadow | Visual separator in dialogs |
| `GetRootWidget(QWidget*, bool)` | Walks parent chain to find top-level `QMainWindow`/`QDialog` | Used everywhere to get dialog parent |

### Table/Tree Column Sizing
| Function | Description |
|----------|-------------|
| `ResizeColumnsForTableView(QTableView*, initializer_list<int>)` | Resize columns by spec; **negative width = stretch to fill** |
| `ResizeColumnsForTreeView(QTreeView*, initializer_list<int>)` | Same for tree views |

**Key detail**: The algorithm accounts for:
- Hidden columns (skipped)
- Scrollbar width (if visible)
- Minimum section size from header
- Flex items share remaining space equally

### Pixmap Scaling System (5 modes)
| ScalingMode | Behavior |
|-------------|----------|
| `Fit` | Keep aspect ratio, fit inside bounds (letterbox) |
| `Fill` | Keep aspect ratio, expand to cover bounds (crop) |
| `Stretch` | Ignore aspect, fill exactly |
| `Center` | No scale, centered at original size |
| `Tile` | Repeat image as tile brush |

**`resizeAndScalePixmap()`** — Full implementation with:
- DPR-aware (device pixel ratio)
- Opacity support (0-100%)
- Antialiasing + smooth transform
- Early-return optimization if already correct size

**⚠️ HIDDEN FEATURE**: `Tile` mode exists for custom backgrounds — could be used for custom game grid backgrounds.

### Key Event System (`KeyEventToCode`)
- Maps QKeyEvent → u32 keycode including modifiers
- **Shift+symbol remapping**: `!`→`1`, `@`→`2`, `#`→`3`, etc.
- **macOS fix**: Undoes Qt's Control/Meta swap on macOS
- **macOS fix**: Differentiates numpad vs arrow keys by checking `ev->text()`
- Numpad keys get `Qt::KeypadModifier` flag

### File/URL Operations
| Function | Description |
|----------|-------------|
| `ShowInFileExplorer(QWidget*, QFileInfo)` | Win32: `SHOpenFolderAndSelectItems`, macOS: `ShowInFinder`, Linux: opens containing dir |
| `GetShowInFileExplorerMessage()` | Returns platform-specific label: "Show in Explorer"/"Show in Finder"/"Open Containing Directory" |
| `OpenURL(QWidget*, url)` | Opens URL with `QDesktopServices::openUrl()`, shows error dialog on failure |

### Widget Helpers
| Function | Description |
|----------|-------------|
| `StringViewToQString(string_view)` | Safe conversion, handles empty |
| `SetWidgetFontForInheritedSetting(QWidget*, bool)` | Sets **italic** font when setting is inherited (per-game override indicator) |
| `BindLabelToSlider(QSlider*, QLabel*, float)` | Auto-updates label text when slider moves; `range` divides value |
| `SetWindowResizeable(QWidget*, bool)` | Toggles between `Fixed`/`Preferred` size policy; updates status bar grip |
| `ResizePotentiallyFixedSizeWindow(QWidget*, int, int)` | Resizes even if fixed-size (temporarily adjusts min/max) |

### CSV Export
**`AbstractItemModelToCSV(QAbstractItemModel*, int role, bool useQuotes)`**
- Exports any `QAbstractItemModel` to CSV string
- Includes headers
- Optional quote wrapping for values

**⚠️ HIDDEN FEATURE**: Could be used for "Export Game List to CSV" feature.

### Compositor Check
**`IsCompositorManagerRunning()`**
- Checks `PCSX2_NO_COMPOSITING` env var
- Linux X11: checks `QX11Info::isCompositingManagerRunning()`
- Used for dock drop indicators transparency

### Scalable Icon System
**`SetScalableIcon(QLabel*, QIcon, QSize)`**
- Installs event filter that reloads pixmap on `QEvent::DevicePixelRatioChange`
- Used for SVG icons and multi-size pixmap icons
- Auto-updates on HiDPI changes

**⚠️ HIDDEN FEATURE**: All flag icons and UI icons use this system — they auto-sharpen on DPI changes.

### Language/Localization System
| Function | Description |
|----------|-------------|
| `GetSystemLanguageCode()` | Matches system locale against available languages; falls back to "en-US" |
| `GetFlagIconForLanguage(QString)` | Loads SVG flag from `icons/flags/{country_code}.svg` |

**Special cases**:
- `"system"` → resolves to actual system language
- `es-419` (Latin America) → Mexico flag
- `sr-SP` (Serbia) → RS country code
- Language-only codes (e.g., "en") → assumes US flag

---

## 2. QtKeyCodes.cpp — Keyboard Input Mapping

### Key Name Database
**`s_qt_key_names[]`** — Comprehensive table of ~350 key mappings:

| Category | Keys | Has Icons |
|----------|------|-----------|
| **Modifiers** | Escape, Tab, Backspace, Return, Shift, Control, Meta, Alt, CapsLock, NumLock, ScrollLock | ✅ |
| **Navigation** | Home, End, Left/Up/Right/Down, PageUp/Down, Insert, Delete | ✅ |
| **Function keys** | F1–F12 | ✅ (ICON_PF_F1..F12) |
| **Extended function** | F13–F35 | ❌ |
| **Letters** | A–Z | ✅ (ICON_PF_KEY_A..Z) |
| **Digits** | 0–9 | ✅ (ICON_PF_0..9) |
| **Symbols** | Comma, Period, Slash, Semicolon, etc. | ❌ |
| **International** | Kanji, Hangul, Hiragana, Katakana, etc. | ❌ |
| **Media** | VolumeUp/Down/Mute, MediaPlay/Stop/Next/Prev, etc. | ❌ |
| **System** | Power, Sleep, Wake, Eject, Print, etc. | ❌ |

### InputManager Integration
| Function | Description |
|----------|-------------|
| `ConvertHostKeyboardStringToCode(string_view)` | String→keycode; handles `Numpad` prefix |
| `ConvertHostKeyboardCodeToString(u32)` | Keycode→string; prepends `Numpad` if keypad modifier |
| `ConvertHostKeyboardCodeToIcon(u32)` | Keycode→icon font character (for visual key display) |

---

## 3. QtProgressCallback.h / .cpp — Progress UI System

### QtModalProgressCallback (Synchronous/Blocking)
- Extends `BaseProgressCallback` + `QObject`
- Uses `QProgressDialog` internally
- **Delayed show**: Won't appear until `m_show_delay` seconds elapsed (prevents flash for fast ops)
- Min width: 500px
- Modal when parent provided
- `autoClose=false`, `autoReset=false` (manual control)

| Method | Behavior |
|--------|----------|
| `SetCancellable(bool)` | Shows/hides Cancel button |
| `SetTitle(const char*)` | Sets window title |
| `SetStatusText(const char*)` | Sets label text (shows dialog if delay passed) |
| `SetProgressRange(u32)` | Sets max value |
| `SetProgressValue(u32)` | Updates progress + calls `processEvents()` |
| `ModalError/Confirmation/Information` | Shows QMessageBox on top of progress dialog |

### QtAsyncProgressThread (Asynchronous)
- Extends `QThread` + `BaseProgressCallback`
- Runs work on background thread
- Emits signals for UI updates (cross-thread safe):
  - `titleUpdated(QString)`
  - `statusUpdated(QString)`
  - `progressUpdated(int value, int range)`
  - `threadStarting()`
  - `threadFinished()`
- Uses `QSemaphore` for start synchronization
- `join()` = `QThread::wait()`

**⚠️ HIDDEN FEATURE**: `processEvents()` in `SetProgressValue` keeps UI responsive during long operations.

---

## 4. SettingWidgetBinder.h — Setting Widget Auto-Binding System

### Architecture
This is the **core settings binding framework** — auto-connects Qt widgets to PCSX2 settings with:
- Read from settings → widget
- Widget change → write to settings → apply
- Per-game override support (nullable/global)

### Supported Widget Types
| Widget | Bool | Int | Float | String | Enum | DateTime | Nullable |
|--------|------|-----|-------|--------|------|----------|----------|
| `QLineEdit` | ✅ | ✅ | ✅ | ✅ | — | — | ✅ |
| `QComboBox` | ✅ | ✅ | ✅ | ✅ | ✅ | — | ✅ (with "Use Global Setting" prefix) |
| `QCheckBox` | ✅ | ✅ | ✅ | ✅ | — | — | ✅ (tristate) |
| `QSlider` | ✅ | ✅ | ✅ | ✅ | — | — | ✅ (context menu Reset) |
| `QSpinBox` | ✅ | ✅ | ✅ | ✅ | — | — | ✅ (prefix "Default: ") |
| `QDoubleSpinBox` | ✅ | ✅ | ✅ | ✅ | — | — | ✅ (prefix "Default: ") |
| `QAction` | ✅ | ✅ | ✅ | ✅ | — | — | ❌ |
| `QDateTimeEdit` | — | — | — | — | — | ✅ | — |

### Nullable/Per-Game System
For per-game settings, widgets support **3 states**:
1. **Explicit value** — user set it for this game
2. **Null/Default** — uses global setting (shown as "Use Global Setting [value]" or "Default: X")
3. **Reset to null** — right-click context menu → "Reset"

**Qt Properties used**:
- `SettingWidgetBinder_isNullable` — widget supports null
- `SettingWidgetBinder_isNull` — currently null
- `SettingWidgetBinder_globalValue` — the global default value

### Bind Functions
| Function | Purpose |
|----------|---------|
| `BindWidgetToBoolSetting` | Bind widget↔bool setting |
| `BindWidgetToIntSetting` | Bind widget↔int setting (with optional option_offset) |
| `BindWidgetAndLabelToIntSetting` | Bind widget↔int + auto-update label with value+suffix |
| `BindWidgetToFloatSetting` | Bind widget↔float setting |
| `BindWidgetToNormalizedSetting` | Bind widget↔float with range multiplier (e.g., slider 0-100 → 0.0-1.0) |
| `BindWidgetToStringSetting` | Bind widget↔string setting |
| `BindWidgetToEnumSetting` (3 variants) | Bind widget↔enum via from_string/to_string functions or name/value arrays |
| `BindWidgetToFolderSetting` | Folder path with Browse/Open/Reset buttons |
| `BindWidgetToAudioFileSetting` | Audio file with Browse/Preview/Reset buttons |
| `BindWidgetToDateTimeSetting` | Date+time with year/month/day/hour/minute/second keys |

### Folder Setting Features
- Browse button → `QFileDialog::getExistingDirectory`
- Open button → opens folder in file explorer
- Reset button → restores default
- Auto-creates directory if doesn't exist (with confirmation dialog)
- Relative path support (relative to `EmuFolders::DataRoot`)
- Disabled in per-game settings (folder changes only in base config)

### Audio File Setting Features
- Browse button → `QFileDialog::getOpenFileName` with filter
- Preview button → `Common::PlaySoundAsync()`
- Reset button → clears value
- Validates file existence after selection

### DateTime Setting Features
- Year offset of 2000 (stored as 0-255, displayed as 2000-2255)
- Default: 2000-01-01 00:00:00
- Stored across 6 separate int settings (year, month, day, hour, minute, second)
- Per-game override support

### Per-Game vs Global Settings Flow
```
if (sif) {
    // Per-game: nullable, saves to game .ini, reloads game settings
    sif->SetXxxValue(...) / sif->DeleteValue(...)
    QtHost::SaveGameSettings(sif, true);
    g_emu_thread->reloadGameSettings();
} else {
    // Global: direct write, commits, applies
    Host::SetBaseXxxSettingValue(...)
    Host::CommitBaseSettingChanges();
    g_emu_thread->applySettings();
}
```

---

## 5. AsyncDialogs.h / .cpp — Non-Blocking Dialog System

### Problem Solved
Qt's built-in `QInputDialog::getText()` etc. are **blocking** — if parent is destroyed during dialog, crash. These wrappers use `dialog->open()` (non-blocking) + `WA_DeleteOnClose`.

### Available Async Dialogs
| Function | Replaces | Callback Type |
|----------|----------|---------------|
| `getText()` (2 overloads) | `QInputDialog::getText` | `QString` or `optional<QString>` |
| `getMultiLineText()` (2 overloads) | `QInputDialog::getMultiLineText` | `QString` or `optional<QString>` |
| `getItem()` (2 overloads) | `QInputDialog::getItem` | `QString` or `optional<QString>` |
| `getInt()` (2 overloads) | `QInputDialog::getInt` | `int` or `optional<int>` |
| `getDouble()` (2 overloads) | `QInputDialog::getDouble` | `double` or `optional<double>` |
| `information()` (2 overloads) | `QMessageBox::information` | `StandardButton` |
| `question()` (2 overloads) | `QMessageBox::question` | `void` (yes callback) or `StandardButton` |
| `warning()` (2 overloads) | `QMessageBox::warning` | `StandardButton` |
| `critical()` (2 overloads) | `QMessageBox::critical` | `StandardButton` |

### Internal Helper
**`wrapValueCallback<T>`** — Converts `function<void(T)>` to `function<void(optional<T>)>` for simple overloads.

**`openAsyncMessageBox()`** — Custom implementation based on `QMessageBoxPrivate::showNewMessageBox`:
- Creates QMessageBox with `WA_DeleteOnClose`
- Iterates button mask to add standard buttons
- Sets default button based on AcceptRole or explicit default
- Connects `finished` signal to callback

---

## 6. EarlyHardwareCheck.cpp — Pre-Main CPU Check

### Purpose
**Windows MSVC only** — runs BEFORE `main()` and global constructors.

### How It Works
- Uses `#pragma init_seg(".CRT$XCT")` — earliest CRT initialization segment
- Calls `VMManager::PerformEarlyHardwareChecks()` (checks SSE4.1/AVX2 support)
- On failure: Shows `MessageBoxW` with error, then `TerminateProcess()`
- Uses raw Win32 APIs only (`HeapAlloc`, `MultiByteToWideChar`) — no CRT dependency (CRT might not be initialized yet)

**⚠️ HIDDEN FEATURE**: This prevents crashes from AVX instructions in global object constructors that MSVC generates for zeroing memory.

---

## 7. VCRuntimeChecker.cpp — VC++ Runtime Version Check

### Purpose
**Windows only** — checks if MSVC runtime is new enough before PCSX2 starts.

### Minimum Version
- **14.38.33135.0** (Visual Studio 2022 17.x)

### How It Works
1. Gets handle to `msvcp140.dll`
2. Gets module filename (with retry loop for buffer size)
3. Gets `VS_FIXEDFILEINFO` via `GetFileVersionInfoExW`
4. Compares version against `MIN_VERSION`
5. If too old: Shows error with current vs required version + download URL
6. If user clicks Yes: Opens `https://aka.ms/vs/17/release/vc_redist.x64.exe`
7. If user clicks No: Continues (will likely crash)

### Download URL
```
https://aka.ms/vs/17/release/vc_redist.x64.exe
```

Also uses `#pragma init_seg(".CRT$XCT")` for pre-main execution.

---

## Summary: Hidden Features Found

1. **Tile scaling mode** — for custom backgrounds
2. **CSV export** — `AbstractItemModelToCSV()` for game list export
3. **DPI-aware icon system** — auto-updates on HiDPI changes
4. **Nullable widget system** — 3-state per-game settings with context menu reset
5. **Audio preview** — `Common::PlaySoundAsync()` for sound test
6. **DateTime setting** — 6-key split storage with year offset
7. **Delayed progress dialog** — prevents flash for fast operations
8. **Cross-thread progress** — signal-based UI updates from background thread
9. **Pre-main hardware check** — CPU feature verification before any CRT init
10. **Pre-main VC runtime check** — prevents mysterious crashes from old runtime
