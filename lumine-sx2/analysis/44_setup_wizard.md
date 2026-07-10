# 44 — Setup Wizard Dialog Analysis

## Files Analyzed
- `SetupWizardDialog.h` — 78 lines
- `SetupWizardDialog.cpp` — 340 lines

---

## Overview
First-run wizard that walks users through initial PCSX2 configuration. Uses a `QStackedWidget` (`m_ui.pages`) with 6 pages navigated by Back/Next/Cancel buttons. Page labels in a sidebar indicate progress (bold = current page).

---

## Page Enum & Order

```
Page_Language        (0) — Theme, language, auto-update toggle
Page_BIOS            (1) — BIOS search directory, BIOS file picker
Page_GameList        (2) — Game search directories (add/remove/recursive toggle)
Page_Controller      (3) — Controller type + automatic mapping for 2 ports
Page_RetroAchievements (4) — RA enable/hardcore/login
Page_Complete        (5) — Finish screen
```

---

## Global Navigation

| Element | Type | Behavior |
|---------|------|----------|
| `back` | QPushButton | Goes to previous page; disabled on page 0 |
| `next` | QPushButton | Text changes to "&Finish" on last page; calls `accept()` on Page_Complete |
| `cancel` | QPushButton | Confirmation dialog ("Are you sure?"), calls `reject()` |
| `logo` | QLabel | Displays `icons/AppIconLarge.png` |
| `m_page_labels[6]` | QLabel[] | Sidebar page indicators; current page bolded |

### Validation Before Next (`canShowNextPage`)
- **Page_BIOS**: If no BIOS selected, warns "PCSX2 will not be able to run games without a BIOS" — asks Yes/No
- **Page_GameList**: If no directories added, warns "list will be empty" — asks Yes/No
- Other pages: no gate

### `pageChangedTo(int page)` triggers
- **Page_GameList**: `resizeDirectoryListColumns()`
- **Page_RetroAchievements**: `refreshRetroAchievementsLoginState()`

---

## Page 0: Language & Theme

### UI Elements

| Element | Type | Setting Key | Notes |
|---------|------|-------------|-------|
| `theme` | QComboBox | `UI/Theme` | Bound via `InterfaceSettingsWidget::THEME_NAMES/VALUES` |
| `language` | QComboBox | `UI/Language` | Populated from `QtHost::GetAvailableLanguageList()`; flag icons if available |
| `autoUpdateEnabled` | QCheckBox | `AutoUpdater/CheckAtStartup` | Default: true |

### Callbacks
- `themeChanged()` → `QtHost::UpdateApplicationTheme()` (immediate theme switch)
- `languageChanged()` → `QtHost::InstallTranslator(this)` + `m_ui.retranslateUi(this)`

### Hidden Features
- **Flag icons per language**: Uses `QtUtils::GetFlagIconForLanguage()` — languages with flag icons get visual indicators
- **Auto-updater opt-in**: Separate from main settings, presented during first run

---

## Page 1: BIOS Setup

### UI Elements

| Element | Type | Setting Key | Notes |
|---------|------|-------------|-------|
| `biosSearchDirectory` | QLineEdit | `Folders/Bios` | Default: `<DataRoot>/bios` |
| `browseBiosSearchDirectory` | QPushButton | — | Folder picker dialog |
| `openBiosSearchDirectory` | QPushButton | — | Opens folder in file manager |
| `resetBiosSearchDirectory` | QPushButton | — | Resets to default path |
| `refreshBiosList` | QPushButton | — | Triggers `refreshBiosList()` |
| `biosList` | QTreeWidget | — | Populated by `BIOSSettingsWidget::populateList()` |

### Callbacks
- `biosSearchDirectory.textChanged` → `refreshBiosList()`
- `refreshBiosList.clicked` → `refreshBiosList()`
- `biosList.currentItemChanged` → writes selected BIOS to `Filenames/BIOS`, commits, applies via emu thread

### Hidden Features
- **Folder binding pattern**: `SettingWidgetBinder::BindWidgetToFolderSetting` — binds browse/open/reset buttons to a single path setting with 4-way sync
- **Live BIOS list refresh**: Changing the search directory immediately repopulates the tree

---

## Page 2: Game List Directories

### UI Elements

| Element | Type | Notes |
|---------|------|-------|
| `searchDirectoryList` | QTableWidget | 2 columns: path (read-only) + recursive checkbox |
| `addSearchDirectoryButton` | QPushButton | Opens folder picker |
| `removeSearchDirectoryButton` | QPushButton | Disabled when nothing selected |

### Settings Modified
- `GameList/Paths` — flat scan directories
- `GameList/RecursivePaths` — recursive scan directories

### Callbacks
- `addSearchDirectoryButton.clicked` → folder picker → asks "Scan Recursively?" (Yes/No/Cancel) → adds to appropriate list
- `removeSearchDirectoryButton.clicked` → removes from both `Paths` and `RecursivePaths` lists
- `searchDirectoryList.customContextMenuRequested` → context menu with:
  - **Remove** — same as remove button
  - **Open Directory...** — opens in system file manager

### Row Structure (`addPathToTable`)
Each row has:
- Column 0: `QTableWidgetItem` with path text (non-editable)
- Column 1: `QCheckBox` for recursive toggle — toggling moves path between `Paths` ↔ `RecursivePaths`

### Hidden Features
- **Recursive toggle per-directory**: Each directory has its own recursive checkbox — toggling live-switches between flat/recursive path lists
- **Context menu**: Right-click on directory → Remove or Open Directory
- **Column resize**: First column fills available space (`-1`), recursive column fixed at 100px
- **Sorted display**: Directory list sorted ascending by path
- **Single selection mode**: Only one directory can be selected at a time

---

## Page 3: Controller Setup

### UI Elements

| Element | Type | Notes |
|---------|------|-------|
| `controller1Type` | QComboBox | Pad 1 type (populated from `Pad::GetControllerTypeNames()`) |
| `controller1Mapping` | QLabel | Shows current mapping result |
| `controller1AutomaticMapping` | QToolButton | Opens device selection menu |
| `controller2Type` | QComboBox | Pad 2 type (same source) |
| `controller2Mapping` | QLabel | Current mapping for pad 2 |
| `controller2AutomaticMapping` | QToolButton | Opens device selection menu |

### Settings Modified
- `Pad1/Type`, `Pad2/Type` — controller type per port

### Device Management
- `m_device_list` — `QList<QPair<QString, QString>>` of (identifier, display_name)
- Populated via `g_emu_thread->enumerateInputDevices()`
- Dynamically updated on connect/disconnect events

### Automatic Mapping Flow
1. User clicks mapping button → `openAutomaticMappingMenu(port, label)`
2. `QMenu` populated from `m_device_list` — each action shows "name (identifier)"
3. If no devices → "No devices available" (disabled)
4. User selects device → `doDeviceAutomaticBinding(port, label, device)`
5. `InputManager::GetGenericBindingMapping(device)` → generic bindings
6. `Pad::MapController()` applies mapping to settings layer
7. Commits settings, updates label to device name

### Signals Connected to EmuThread
- `onInputDevicesEnumerated` → stores full device list
- `onInputDeviceConnected` → appends to `m_device_list`
- `onInputDeviceDisconnected` → removes from `m_device_list`

### Hidden Features
- **Live device hot-plug**: Device list updates in real-time as controllers are connected/disconnected
- **Generic binding mapping**: Uses `InputManager::GetGenericBindingMapping()` — maps device to standard PS2 pad bindings
- **Per-port independent binding**: Port 1 and Port 2 have independent type and mapping
- **Default mapping labels**: Port 1 = "Default (Keyboard)", Port 2 = "Default (None)"

---

## Page 4: RetroAchievements

### UI Elements

| Element | Type | Setting Key | Notes |
|---------|------|-------------|-------|
| `raLogo` | QLabel | — | SVG icon `icons/ra-icon.svg` at 56×56 |
| `raEnableAchievements` | QCheckBox | `Achievements/Enabled` | Default: false |
| `raHardcoreMode` | QCheckBox | `Achievements/ChallengeMode` | Default: false |
| `raLoginButton` | QPushButton | — | "Login..." or "Logout" depending on state |
| `raViewProfileButton` | QPushButton | — | Opens RA profile in browser; disabled when not logged in |
| `raLoginStatus` | QLabel | — | Shows username + token date OR "Not Logged In." |
| `raIntegrationLabel` | QLabel | — | Shown if using RA Integration (replaces content widget) |
| `raContentWidget` | QWidget | — | Hidden if using RA Integration |

### Login Flow
1. If logged in (username exists in `Achievements/Username`):
   - Shows "Username: X\nLogin token generated on Y"
   - Button text = "Logout"
   - Click → `Achievements::Logout()`
2. If not logged in:
   - Shows "Not Logged In."
   - Button text = "Login..."
   - Click → opens `AchievementLoginDialog`
   - On success: refreshes state, syncs enable/hardcore checkboxes

### Hidden Features
- **RA Integration detection**: If `Achievements::IsUsingRAIntegration()` is true, hides all RA settings and shows a label instead — RA Integration takes over
- **Profile URL**: Opens `https://retroachievements.org/user/<encoded_username>` — username is percent-encoded
- **Login auto-sync**: After login, if the login process enabled achievements or hardcore mode, the checkboxes are updated in-place with signal blocking to prevent feedback loops

---

## Page 5: Complete
- Standard finish page
- `next` button text = "&Finish"
- Clicking Next calls `accept()` (closes dialog with accepted result)

---

## Cancel Behavior
- Confirmation dialog: "Any changes have been saved, and the wizard will run again next time you start PCSX2."
- Calls `reject()` — wizard will reappear on next launch

---

## Architecture Notes

### Setting Binding Pattern
Uses `SettingWidgetBinder` for:
- Enum binding (theme combo)
- String binding (language combo)
- Bool binding (auto-update, RA enable, hardcore)
- Folder binding (BIOS directory with browse/open/reset)

### Thread Safety
- Settings committed via `Host::CommitBaseSettingChanges()`
- Applied to emulation via `g_emu_thread->applySettings()`
- Device enumeration via emu thread signals
- RA logout dispatched to CPU thread via `Host::RunOnCPUThread()`

### UI Framework
- Generated from `ui_SetupWizardDialog.h` (Qt Designer `.ui` file)
- `m_ui.retranslateUi(this)` for runtime language switching
- `QSignalBlocker` used to prevent recursive signal firing during programmatic updates

---

## Summary of All Features

1. **Language selection** with flag icons
2. **Theme selection** (live preview)
3. **Auto-updater opt-in** at first run
4. **BIOS directory** with browse/open/reset
5. **BIOS file picker** from tree widget
6. **Game directory management** — add, remove, recursive toggle, context menu, open in file manager
7. **Controller type selection** for 2 ports
8. **Automatic controller mapping** with live device list
9. **Controller hot-plug** detection
10. **RetroAchievements** enable/disable, hardcore mode, login/logout, profile link
11. **RA Integration** detection and fallback
12. **Page validation gates** (BIOS warning, empty game list warning)
13. **Page progress indicators** (bold current page label)
14. **Cancel confirmation** with note about re-running
