# SettingsWindow Deep Analysis — PCSX2 Qt

## Source Files
- `pcsx2-qt/Settings/SettingsWindow.h`
- `pcsx2-qt/Settings/SettingsWindow.cpp`
- `pcsx2-qt/Settings/SettingsWindow.ui`

---

## 1. Window Architecture

### Layout Structure (SettingsWindow.ui)
```
QGridLayout (columnStretch="0,1")
├── [0,0] QListWidget "settingsCategory" (min 160px, max 160px, iconSize 32x32)
├── [0,1] QStackedWidget "settingsContainer" (min 500px)
├── [1,0:2] QTextEdit "helpText" (readOnly, fixed height 122px)
└── [2,0:2] QHBoxLayout "footerLayout"
    ├── QPushButton "restoreDefaultsButton" ("Restore Defaults")
    ├── QPushButton "copyGlobalSettingsButton" ("Copy Global Settings")
    ├── QPushButton "clearGameSettingsButton" ("Clear Settings")
    ├── QSpacer
    └── QPushButton "closeButton" ("Close", default=true)
```

**Hidden detail**: The category list is a LEFT SIDEBAR with icon+text, not bottom tabs. The help text panel is a FIXED 122px area at the BOTTOM showing contextual help for whatever widget the mouse is hovering over.

---

## 2. Settings Categories (Ordered)

### Global Settings Mode (m_sif == nullptr)
| # | Category | Widget Class | Icon | Advanced Only? | Per-Game? |
|---|----------|-------------|------|----------------|-----------|
| 0 | Interface | InterfaceSettingsWidget | interface-line | No | Yes |
| 1 | Game List | GameListSettingsWidget | folder-open-line | No | **NO** |
| 2 | BIOS | BIOSSettingsWidget | chip-line | No | **NO** |
| 3 | Emulation | EmulationSettingsWidget | emulation-line | No | Yes |
| 4 | Graphics | GraphicsSettingsWidget | image-fill | No | Yes |
| 5 | On-Screen Display | OSDSettingsWidget | heart-circle-line | No | Yes |
| 6 | Audio | AudioSettingsWidget | volume-up-line | No | Yes |
| 7 | Memory Cards | MemoryCardSettingsWidget | memcard-line | No | Yes |
| 8 | Network & HDD | DEV9SettingsWidget | global-line | No | Yes |
| 9 | Folders | FolderSettingsWidget | folder-settings-line | No | **NO** |
| 10 | Achievements | AchievementSettingsWidget | trophy-line | No | Yes |
| 11 | Advanced | AdvancedSettingsWidget | warning-line | **YES** | Yes |
| 12 | Debug | DebugSettingsWidget | bug-line | **YES** | Yes |

### Per-Game Settings Mode (m_sif != nullptr)
| # | Category | Widget Class | Notes |
|---|----------|-------------|-------|
| 0 | Summary | GameSummaryWidget | **ONLY in per-game mode** |
| 1 | Interface | InterfaceSettingsWidget | |
| 2 | Emulation | EmulationSettingsWidget | |
| 3 | Patches | GamePatchSettingsWidget | **ONLY in per-game mode** |
| 4 | Cheats | GameCheatSettingsWidget | **ONLY in per-game mode** |
| 5 | Game Fixes | GameFixSettingsWidget | **Per-game + Advanced only** |
| 6 | Graphics | GraphicsSettingsWidget | |
| 7 | On-Screen Display | OSDSettingsWidget | |
| 8 | Audio | AudioSettingsWidget | |
| 9 | Memory Cards | MemoryCardSettingsWidget | |
| 10 | Network & HDD | DEV9SettingsWidget | |
| 11 | Achievements | AchievementSettingsWidget | |
| 12 | Advanced | AdvancedSettingsWidget | Advanced only |
| 13 | Debug | DebugSettingsWidget | Advanced only |

---

## 3. Hidden/Non-Obvious Features

### 3.1 Per-Game vs Global Settings Architecture
- **Two constructors**: `SettingsWindow()` for global, `SettingsWindow(unique_ptr<INISettingsInterface>, ...)` for per-game
- `m_sif` being non-null means per-game mode
- Per-game settings are stored in separate INI files per serial/CRC
- **Copy Global Settings**: Copies all global config into per-game INI
- **Clear Settings**: Clears per-game INI + disables all cheats/patches
- **Restore Defaults**: Only in global mode, resets everything

### 3.2 Help Text System (Mouse-Over Contextual Help)
- `registerWidgetHelp(QObject*, title, recommended_value, text)` — attaches hover help to ANY widget
- Uses event filter: `QEvent::Enter` shows help, `QEvent::Leave` restores category help
- Help text format: title + recommended value in table, then description below `<hr>`
- **Shift+Wheel** scrolls the help text panel (not the settings!)

### 3.3 Category Navigation
- `setCategory(const char*)` — programmatically switch to a category by name (translated)
- `getCategory()` — get current category text
- Used by MainWindow to open settings to specific tab (e.g., controller settings)

### 3.4 Effective Value Resolution (Per-Game Layering)
```cpp
getEffectiveBoolValue(section, key, default)  // per-game → global → default
getEffectiveIntValue(section, key, default)
getEffectiveFloatValue(section, key, default)
getEffectiveStringValue(section, key, default)
```
- If per-game INI has value, use it; else fall back to global; else use default

### 3.5 Setting Value Getters (Layer-Specific)
```cpp
getBoolValue(section, key, default)   // returns optional — empty means "use global"
getIntValue(section, key, default)
getFloatValue(section, key, default)
getStringValue(section, key, default)
```
- For per-game: returns the value from per-game INI only (or default if not set)
- For global: returns from global settings

### 3.6 Setting Value Setters (Auto-Save)
```cpp
setBoolSettingValue(section, key, optional_value)
setIntSettingValue(section, key, optional_value)
setFloatSettingValue(section, key, optional_value)
setStringSettingValue(section, key, optional_value)
```
- `nullopt` → deletes the key (revert to global/default)
- Per-game mode: saves to per-game INI, triggers `reloadGameSettings()`
- Global mode: saves to base settings, calls `applySettings()`

### 3.7 Game Properties Dialog Management
- `openGamePropertiesDialog()` — opens per-game settings for a specific game
- Checks for existing dialog with same filename → brings to front instead of creating duplicate
- `closeGamePropertiesDialogs()` — closes ALL open per-game dialogs
- `s_open_game_properties_dialogs` — static list tracking open per-game windows

### 3.8 Disc Serial Change Signal
- `discSerialChanged()` signal — emitted when serial changes
- `setSerial()` updates serial and emits signal

### 3.9 Reopen Pattern
- `reopen(message)` — closes current per-game dialog, reopens with fresh INI
- Used after Copy Global or Clear Settings to refresh the UI

### 3.10 RAIntegration Fallback
- If `Achievements::IsUsingRAIntegration()`, shows placeholder label instead of AchievementSettingsWidget
- Built-in RetroAchievements support is disabled when RAIntegration is active

### 3.11 Footer Button Visibility
- **Global mode**: Shows "Restore Defaults" + "Close"; hides "Copy Global" + "Clear"
- **Per-game mode**: Shows "Copy Global" + "Clear" + "Close"; hides "Restore Defaults"
- Buttons are actually `deleteLater()`'d, not just hidden

### 3.12 Window Title for Per-Game
- Global: "PCSX2 Settings"
- Per-game: "Game Title [filename.ini]"

---

## 4. Widget Creation Flow

```
setupUi(game)
├── m_ui.setupUi(this)  // Qt Designer form
├── if per-game:
│   ├── add GameSummaryWidget (or placeholder)
│   └── remove restoreDefaultsButton
├── else global:
│   ├── remove copyGlobalSettingsButton
│   └── remove clearGameSettingsButton
├── addWidget(InterfaceSettingsWidget)
├── if !per-game: addWidget(GameListSettingsWidget, BIOSSettingsWidget)
├── addWidget(EmulationSettingsWidget)
├── if per-game: addWidget(GamePatchSettingsWidget, GameCheatSettingsWidget)
├── if advanced && per-game: addWidget(GameFixSettingsWidget)
├── addWidget(GraphicsSettingsWidget)
├── addWidget(OSDSettingsWidget)
├── addWidget(AudioSettingsWidget)
├── addWidget(MemoryCardSettingsWidget)
├── addWidget(DEV9SettingsWidget)
├── if !per-game: addWidget(FolderSettingsWidget)
├── addWidget(AchievementSettingsWidget) or placeholder
├── if advanced: addWidget(AdvancedSettingsWidget, DebugSettingsWidget)
├── set category list to row 0
├── connect signals
```

---

## 5. Signals & Slots

| Signal/Slot | Type | Trigger |
|-------------|------|---------|
| `discSerialChanged()` | Signal | Serial changes via `setSerial()` |
| `onCategoryCurrentRowChanged(int)` | Slot | Category list row changes |
| `onRestoreDefaultsClicked()` | Slot | "Restore Defaults" button |
| `onCopyGlobalSettingsClicked()` | Slot | "Copy Global Settings" button |
| `onClearSettingsClicked()` | Slot | "Clear Settings" button |

---

## 6. Features NOT Yet in LumineSX2

| Feature | Priority | Notes |
|---------|----------|-------|
| Per-game settings (Summary tab) | HIGH | Separate INI per game, Copy/Clear buttons |
| Help text system (hover) | MEDIUM | Contextual help for every widget |
| Restore Defaults dialog | MEDIUM | With UI reset checkbox |
| Game Fixes (per-game only) | HIGH | 13 hardware fix toggles |
| Patches (per-game only) | HIGH | Per-game patches list |
| Cheats (per-game only) | HIGH | Per-game cheat toggles |
| RAIntegration fallback | LOW | Shows placeholder when RAIntegration active |
| Category programmatic navigation | MEDIUM | `setCategory()` for deep linking |
| Advanced/Debug toggle | MEDIUM | Hidden when `ShouldShowAdvancedSettings()` is false |
| Shift+Wheel help scroll | LOW | Scroll help panel with Shift+Wheel |
| Game properties dialog management | HIGH | Deduplicate, reopen pattern |
| Footer button lifecycle (deleteLater) | LOW | Proper cleanup, not just hide |
