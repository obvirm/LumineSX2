# Interface Settings — Deep Analysis

Source: `pcsx2-qt/Settings/InterfaceSettingsWidget.h` + `.cpp`

## Signals (events emitted to parent)
- `themeChanged()` — when theme combo changes
- `languageChanged()` — when language combo changes
- `backgroundChanged()` — when background image/opacity/scale changes
- `preferEnglishGameListChanged()` — when English title preference toggles

## Public Methods
- `updatePromptOnStateLoadSaveFailureCheckbox(Qt::CheckState)` — external sync
- `updateMouseLockCheckbox(Qt::CheckState)` — external sync

## Static Constants
- `THEME_NAMES[]` / `THEME_VALUES[]` — 18 themes total
- `BACKGROUND_SCALE_NAMES[]` — fit, fill, stretch, center, tile
- `IMAGE_FILE_FILTER` — bmp, gif, jpg, jpeg, png, webp

---

## ALL Settings (by group)

### 1. Inhibit Screensaver
- Key: `EmuCore/InhibitScreensaver` (bool, default: true)
- UI: Checkbox
- Effect: Prevents screen saver + host sleep during emulation

### 2. Confirm Shutdown
- Key: `UI/ConfirmShutdown` (bool, default: true)
- UI: Checkbox
- Effect: Show confirmation dialog when shutting down VM via hotkey

### 3. Pause On Focus Loss
- Key: `UI/PauseOnFocusLoss` (bool, default: false)
- UI: Checkbox
- Effect: Pause on minimize/alt-tab, unpause on return

### 4. Pause On Controller Disconnection
- Key: `UI/PauseOnControllerDisconnection` (bool, default: false)
- UI: Checkbox
- Effect: Pause when a bound controller disconnects

### 5. Prompt On State Load/Save Failure
- Key: `UI/PromptOnStateLoadSaveFailure` (bool, default: true)
- UI: Checkbox
- Effect: Show modal dialog on savestate failure

### 6. Use Savestate Selector
- Key: `EmuCore/UseSavestateSelector` (bool, default: true)
- UI: Checkbox
- Effect: Show slot selector UI vs notification bubble

### 7. Discord Presence
- Key: `EmuCore/EnableDiscordPresence` (bool, default: false)
- UI: Checkbox
- Effect: Show current game in Discord profile

### 8. Prefer English Game List Titles
- Key: `UI/PreferEnglishGameList` (bool, default: false)
- UI: Checkbox → emits `preferEnglishGameListChanged()`
- Effect: Prefer English title for games with multilingual titles

### 9. Mouse Lock
- Key: `EmuCore/EnableMouseLock` (bool, default: false)
- UI: Checkbox (disabled on Linux Wayland, unavailable without X11/xcb)
- Effect: Lock cursor to window; attaches/detaches mouse position callback
- **HIDDEN**: On check, calls `Common::AttachMousePositionCb()` → `g_main_window->checkMousePosition(x, y)`
- **HIDDEN**: On uncheck, calls `Common::DetachMousePositionCb()`

### 10. Start Fullscreen
- Key: `UI/StartFullscreen` (bool, default: false)
- UI: Checkbox
- Effect: Auto-fullscreen on game start

### 11. Double-Click Toggles Fullscreen
- Key: `UI/DoubleClickTogglesFullscreen` (bool, default: true)
- UI: Checkbox
- Effect: Double-click game window to toggle fullscreen

### 12. Hide Cursor In Fullscreen
- Key: `UI/HideMouseCursor` (bool, default: false)
- UI: Checkbox
- Effect: Hide mouse pointer when in fullscreen mode

### 13. Render To Separate Window
- Key: `UI/RenderToSeparateWindow` (bool, default: false)
- UI: Checkbox → triggers `onRenderToSeparateWindowChanged()`
- Effect: Game renders in separate window instead of main window
- **HIDDEN**: When unchecked, "Hide Main Window" checkbox is disabled

### 14. Hide Main Window When Running
- Key: `UI/HideMainWindowWhenRunning` (bool, default: false)
- UI: Checkbox (enabled only when RenderToSeparateWindow is checked)
- Effect: Hide the game list window during gameplay
- Dependency: Requires "Render To Separate Window"

### 15. Disable Window Resizing
- Key: `UI/DisableWindowResize` (bool, default: false)
- UI: Checkbox
- Effect: Prevent main window from being resized

### 16. Start In Big Picture Mode
- Key: `UI/StartBigPictureMode` (bool, default: false)
- UI: Checkbox
- Effect: Launch into Big Picture Mode instead of Qt interface

### 17. Theme
- Key: `UI/Theme` (enum string, default: system-dependent)
- UI: ComboBox with 18 options
- Emits: `themeChanged()`
- Themes:
  - `""` — Native
  - `"windowsvista"` — Classic Windows (Win only)
  - `"fusion"` — Fusion [Light/Dark]
  - `"darkfusion"` — Dark Fusion (Gray) [Dark]
  - `"darkfusionblue"` — Dark Fusion (Blue) [Dark]
  - `"GreyMatter"` — Grey Matter (Gray) [Dark]
  - `"UntouchedLagoon"` — Untouched Lagoon (Grayish Green/-Blue) [Light]
  - `"BabyPastel"` — Baby Pastel (Pink) [Light]
  - `"PizzaBrown"` — Pizza Time! (Brown-ish/Creamy White) [Light]
  - `"PCSX2Blue"` — PCSX2 (White/Blue) [Light]
  - `"ScarletDevilRed"` — Scarlet Devil (Red/Purple) [Dark]
  - `"VioletAngelPurple"` — Violet Angel (Blue/Purple) [Dark]
  - `"CobaltSky"` — Cobalt Sky (Blue) [Dark]
  - `"AMOLED"` — AMOLED (Black) [Dark]
  - `"Ruby"` — Ruby (Black/Red) [Dark]
  - `"Sapphire"` — Sapphire (Black/Blue) [Dark]
  - `"Emerald"` — Emerald (Black/Green) [Dark]
  - `"Custom"` — custom.qss [Drop in PCSX2 Folder]

### 18. Language
- Key: `UI/Language` (string, default: system locale)
- UI: ComboBox with flag icons
- Emits: `languageChanged()`
- **HIDDEN**: Languages populated dynamically via `QtHost::GetAvailableLanguageList()`, each entry has flag icon

### 19. Game List Background Path
- Key: `UI/GameListBackgroundPath` (string)
- UI: Browse button → file dialog (bmp, gif, jpg, jpeg, png, webp)
- Emits: `backgroundChanged()`
- **HIDDEN**: Path stored as relative to `EmuFolders::DataRoot`
- **HIDDEN**: Reset button clears the setting entirely

### 20. Game List Background Opacity
- Key: `UI/GameListBackgroundOpacity` (float, default: 100.0)
- UI: SpinBox
- Emits: `backgroundChanged()`
- Effect: 0-100% opacity for custom background

### 21. Background Image Scaling Mode
- Key: `UI/GameListBackgroundMode` (enum, default: "fit")
- UI: ComboBox
- Modes: fit, fill, stretch, center, tile
- Emits: `backgroundChanged()`

### 22. Auto Update Check At Startup
- Key: `AutoUpdater/CheckAtStartup` (bool, default: true)
- UI: Checkbox (hidden if not supported or per-game)
- Effect: Check for updates on launch

### 23. Auto Update Tag
- Key: `AutoUpdater/UpdateTag` (string)
- UI: ComboBox (populated from `AutoUpdaterDialog::getTagList()`)
- Effect: Select update channel (stable, nightly, etc.)

### 24. Auto Update Current Version Display
- UI: Read-only label showing `%1 (%2)` → version + date
- Source: `AutoUpdaterDialog::getCurrentVersion()` + `getCurrentVersionDate()`

### 25. Check For Updates Button
- UI: Button → calls `g_main_window->checkForUpdates(true, true)`
- Effect: Manual update check trigger

### 26. Pause On Start
- Key: `UI/StartPaused` (bool, default: false)
- UI: Checkbox (disabled in per-game settings)
- Effect: Pause emulator when game starts

---

## Per-Game Settings Behavior
- When `dialog()->isPerGameSettings()`:
  - **Appearances group hidden** (theme, language, background)
  - **Pause On Start disabled** (settings applied after ELF load)
  - **Auto Updater group hidden** (global only)

## Platform-Specific Behavior
- **Linux Wayland**: Mouse Lock disabled (only X11/xcb supported)
- **Linux X11**: Mouse Lock available
- **Windows/macOS**: Mouse Lock always available
- **Windows only**: "Classic Windows" theme available
- **macOS**: Mouse Lock requires accessibility permissions

## Signal→Slot Connections
1. `preferEnglishGameList::checkStateChanged` → emit `preferEnglishGameListChanged()`
2. `mouseLock::checkStateChanged` → `Common::AttachMousePositionCb()` / `DetachMousePositionCb()`
3. `renderToSeparateWindow::checkStateChanged` → `onRenderToSeparateWindowChanged()` → enables/disables hideMainWindow
4. `theme::currentIndexChanged` → emit `themeChanged()`
5. `language::currentIndexChanged` → emit `languageChanged()`
6. `backgroundBrowse::clicked` → `onSetGameListBackgroundTriggered()` → file dialog → emit `backgroundChanged()`
7. `backgroundReset::clicked` → `onClearGameListBackgroundTriggered()` → remove setting → emit `backgroundChanged()`
8. `backgroundOpacity::editingFinished` → emit `backgroundChanged()`
9. `backgroundScale::currentIndexChanged` → emit `backgroundChanged()`
10. `checkForUpdates::clicked` → `g_main_window->checkForUpdates(true, true)`

## Hidden Features Not Yet in LumineSX2 UI
1. **Inhibit Screensaver** — prevent screen sleep during emulation
2. **Pause On Controller Disconnection** — auto-pause when controller disconnects
3. **Savestate Selector** — toggle between selector UI vs notification
4. **Discord Presence** — show game in Discord profile
5. **Double-Click Toggles Fullscreen** — fullscreen toggle gesture
6. **Hide Cursor In Fullscreen** — cursor visibility control
7. **Render To Separate Window** — separate game window
8. **Hide Main Window When Running** — hide library during gameplay
9. **Disable Window Resizing** — lock window size
10. **Big Picture Mode** — console-like fullscreen UI
11. **18 Themes** — only 3-4 are in LumineSX2 UI (our implementation has just Dark)
12. **Background Image** — custom game list background with opacity + 5 scale modes
13. **Auto Updater** — update channel selection + manual check
14. **Pause On Start** — pause at game launch
15. **Mouse Lock Position Callback** — `checkMousePosition(x, y)` for boundary enforcement
16. **Custom QSS Theme** — drop custom.qss in PCSX2 folder
17. **Language with Flag Icons** — locale-aware language display
