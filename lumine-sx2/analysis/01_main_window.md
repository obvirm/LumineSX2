# PCSX2 Qt MainWindow — Complete Feature Analysis

## 1. MENUS (6 top-level menus)

### 1.1 System Menu (`menuSystem`)
| Action | Type | Description |
|--------|------|-------------|
| `actionStartFile` | Trigger | Open file dialog to start game (supports .bin/.iso/.cue/.mdf/.chd/.cso/.zso/.gz/.elf/.irx/.gs/.dump) |
| `actionStartDisc` | Trigger | Start game from physical disc drive |
| `actionStartBios` | Trigger | Start BIOS without disc |
| `actionStartFullscreenUI` | Trigger | Start/Stop Big Picture Mode |
| `actionPowerOff` | Trigger | Shutdown VM with save option |
| `actionPowerOffWithoutSaving` | Trigger | Shutdown VM without saving |
| `actionReset` | Trigger | Reset running VM |
| `actionPause` | Toggle | Pause/Resume emulation |
| `menuChangeDisc` | SubMenu | Change disc: From File, From Device, From Game List, Remove Disc |
| `actionScreenshot` | Trigger | Take screenshot |
| `actionVideoCapture` | Toggle | Start/stop video capture (supports record-on-boot) |
| `menuLoadState` | SubMenu | Load state: From File, Resume, Slots 1-N, Backup slots, Delete states |
| `menuSaveState` | SubMenu | Save state: To File, Slots 1-N |
| `actionSettings` | Trigger | Open Settings window |
| `actionExit` | Trigger | Exit application |

### 1.2 Settings Menu (`menuSettings`)
| Action | Category String |
|--------|----------------|
| `actionViewGameProperties` | Game Properties (runtime) |
| `actionInterfaceSettings` | "Interface" |
| `actionGameListSettings` | "Game List" |
| `actionBIOSSettings` | "BIOS" |
| `actionEmulationSettings` | "Emulation" |
| `actionGraphicsSettings` | "Graphics" |
| `actionOSDSettings` | "On-Screen Display" |
| `actionAudioSettings` | "Audio" |
| `actionMemoryCardSettings` | "Memory Cards" |
| `actionDEV9Settings` | "Network & HDD" |
| `actionFolderSettings` | "Folders" |
| `actionAchievementSettings` | "Achievements" |
| `actionControllerSettings` | Controllers (separate window) |
| `actionHotkeySettings` | Hotkeys (separate window) |
| `actionAddGameDirectory` | Add search directory |
| `actionScanForNewGames` | Refresh game list (incremental) |
| `actionRescanAllGames` | Refresh game list (full rescan) |

### 1.3 View Menu (`menuView`)
| Action | Description |
|--------|-------------|
| `actionViewToolbar` | Toggle toolbar visibility |
| `actionViewLockToolbar` | Lock/unlock toolbar position |
| `actionViewStatusBar` | Toggle status bar visibility |
| `actionViewStatusBarVerbose` | Toggle verbose status info |
| `actionViewGameList` | Switch to game list view |
| `actionViewGameGrid` | Switch to game grid view |
| `actionViewSystemDisplay` | Switch to emulation display |
| `actionFullscreen` | Toggle fullscreen |
| `menuWindowSize` | Window scale (1x-10x + Internal Resolution) |
| `actionGridViewShowTitles` | Show/hide titles in grid view |
| `actionGridViewZoomIn` | Grid zoom in (Ctrl++) |
| `actionGridViewZoomOut` | Grid zoom out (Ctrl+-) |
| `actionGridViewRefreshCovers` | Refresh grid covers |

### 1.4 Tools Menu (`menuTools`)
| Action | Description |
|--------|-------------|
| `actionOpenDataDirectory` | Open data directory in file explorer |
| `actionCoverDownloader` | Download game covers dialog |
| `actionToggleSoftwareRendering` | Toggle software rendering |
| `actionEditCheats` | Edit cheats PNACH file |
| `actionEditPatches` | Edit patches PNACH file |
| `actionReloadPatches` | Reload cheats/patches |
| `menuInputRecording` | SubMenu: New, Play, Stop, Viewer, Console Logs, Controller Logs |
| `actionEnableSystemConsole` | Toggle system console |
| `actionEnableDebugConsole` | Toggle debug console |
| `actionEnableLogWindow` | Toggle log window |
| `actionEnableFileLogging` | Toggle file logging |
| `actionEnableVerboseLogging` | Toggle verbose logging |
| `actionShowAdvancedSettings` | Toggle advanced settings (with warning dialog) |
| `actionSaveBlockDump` | Toggle CDVD block dump |
| `actionSaveGSDump` | Save single frame GS dump |

### 1.5 Debug Menu (`menuDebug`) — Hidden unless Advanced Settings enabled
| Action | Description |
|--------|-------------|
| `menuDebugSwitchRenderer` | Switch Graphics API (Auto/DX11/DX12/Metal/OGL/VK/SW/Null) |
| `actionDebugger` | Open Debugger window |
| `actionEnableLogTimestamps` | Toggle log timestamps |
| `actionEnableEEConsoleLogging` | Toggle EE console logging |
| `actionEnableIOPConsoleLogging` | Toggle IOP console logging |
| `actionEnableCDVDVerboseReads` | Toggle CDVD verbose reads |

### 1.6 Help Menu (`menuHelp`)
| Action | Description |
|--------|-------------|
| `actionGitHubRepository` | Open GitHub repo |
| `actionSupportForums` | Open support forums |
| `actionWiki` | Open PCSX2 Wiki |
| `actionDocumentation` | Open documentation |
| `actionDiscordServer` | Open Discord server |
| `actionCheckForUpdates` | Check for updates |
| `actionAboutQt` | About Qt dialog |
| `actionAbout` | About PCSX2 dialog |

---

## 2. TOOLBAR ACTIONS (16 buttons)
| Action | Description |
|--------|-------------|
| `actionToolbarStartFile` | Start File |
| `actionToolbarStartDisc` | Start Disc |
| `actionToolbarStartBios` | Start BIOS |
| `actionToolbarStartFullscreenUI` | Big Picture |
| `actionToolbarPowerOff` | Shut Down |
| `actionToolbarReset` | Reset |
| `actionToolbarPause` | Pause (toggle) |
| `actionToolbarChangeDisc` | Change Disc (popup menu) |
| `actionToolbarScreenshot` | Screenshot |
| `actionVideoCapture` | Video Capture (toggle) |
| `actionToolbarLoadState` | Load State (popup menu) |
| `actionToolbarSaveState` | Save State (popup menu) |
| `actionToolbarFullscreen` | Fullscreen (toggle) |
| `actionToolbarSettings` | Settings (popup menu if VM running) |
| `actionToolbarControllerSettings` | Controllers |
| `actionToolbarHotkeySettings` | Hotkeys |

---

## 3. STATUS BAR WIDGETS (8 widgets)
| Widget | Type | Description |
|--------|------|-------------|
| `m_status_progress_widget` | QProgressBar | Game list refresh progress |
| `m_status_verbose_widget` | QLabel | Verbose status text (FPS info when running, "Paused" when paused) |
| `m_status_renderer_widget` | QLabel | Current renderer name |
| `m_status_resolution_widget` | QLabel | Current internal resolution |
| `m_status_volume_widget` | QToolButton | Volume control (click → popup menu with slider, mute toggle, per-game toggle) |
| `m_status_speed_widget` | QToolButton | Speed control (click → popup: Unlimited/Turbo/Slow-Motion/Normal) |
| `m_status_gpu_widget` | QLabel | GPU name/info |
| `m_status_fps_widget` | QLabel | Frames per second |
| `m_status_vps_widget` | QLabel | Vsyncs per second |

### Status Bar Volume Menu Features:
- Slider (0-100)
- Toggle Mute action
- Adjust Per-Game action (checkbox, saves to per-game settings)

### Status Bar Speed Menu:
- Unlimited
- Turbo
- Slow-Motion
- Normal

---

## 4. GAME LIST CONTEXT MENU (right-click on game)
| Action | Condition | Description |
|--------|-----------|-------------|
| Properties... | Always | Open game properties dialog |
| Show in File Explorer | Always | Open file location |
| Set Cover Image... | Always | Custom cover image (jpg/jpeg/png/webp) |
| Create Game Shortcut... | Not macOS | Create desktop shortcut |
| Exclude From List | Always | Add to excluded paths |
| Reset Play Time | If play time > 0 | Reset play time counter |
| Check Wiki Page | If serial exists | Open wiki page |
| Open Memory Card Folder | Always | Open memcard directory |
| Open Snapshots Folder | Always | Open snapshots directory (per-game if configured) |
| Open Texture Dump/Replacement Folder | Always | Open texture directory (per-game by serial) |
| Open Video Capture Folder | Always | Open video capture directory (per-game if configured) |
| Default Boot | Not running | Start game (default) |
| Fast Boot | Not running | Start game (fast boot) |
| Full Boot | Not running | Start game (full boot) |
| Boot and Debug | Not running + Debug visible | Start game with debugger paused on entry |
| Load State submenu | Not running | Load from save slots |
| Change Disc | Running | Change disc to this game |
| Add Search Directory... | Always | Add game directory |

---

## 5. WINDOW STATE MANAGEMENT

### Geometry Persistence
- `UI/MainWindowGeometry` — Base64-encoded window geometry
- `UI/MainWindowState` — Base64-encoded window state (toolbar positions etc.)
- `UI/MainWindowMaximized` — Whether window was maximized
- `UI/MainWindowFullscreen` — Whether window was fullscreen
- `UI/DisplayWindowGeometry` — Separate display window geometry

### Window Modes
- Normal windowed
- Maximized
- Fullscreen (exclusive or borderless)
- Render to main window (display inside mainContainer)
- Render to separate window (DisplaySurface)
- Hide main window when running
- Temporarily windowed (for modal dialogs from fullscreen)

---

## 6. DRAG & DROP SUPPORT
- Accepts single file drops
- Supports: game files (.bin/.iso/.cue/.mdf/.chd/.cso/.zso/.gz/.elf/.irx/.gs/.dump) and save states (.p2s)
- Save state drops → load state
- Game file drops while running → disc change
- ELF drops while running → prompt for reset

---

## 7. KEY SIGNALS & CALLBACKS

### VM Lifecycle
- `onVMStarting()` — VM starting
- `onVMStarted()` — VM fully started
- `onVMPaused()` — VM paused
- `onVMResumed()` — VM resumed
- `onVMStopped()` — VM stopped

### Game List
- `onGameListRefreshProgress(status, current, total)` — Refresh progress
- `onGameListRefreshComplete()` — Refresh done
- `onGameListSelectionChanged()` — Selection changed
- `onGameListEntryActivated()` — Double-click/enter on entry
- `onGameListEntryContextMenuRequested(point)` — Right-click

### Display
- `acquireRenderWindow(recreate, fullscreen, render_to_main, surfaceless)` — Create/acquire display
- `releaseRenderWindow()` — Release display
- `displayResizeRequested(width, height)` — Resize request from GS thread
- `mouseModeRequested(relative_mode, hide_cursor)` — Mouse mode change
- `mouseLockRequested(state)` — Mouse lock change

### Game
- `onGameChanged(title, elf_override, disc_path, serial, disc_crc, crc)` — Game changed
- `onCaptureStarted(filename)` — Video capture started
- `onCaptureStopped()` — Video capture stopped

### Achievements
- `onAchievementsLoginRequested(reason)` — Login prompt
- `onAchievementsHardcoreModeChanged(enabled)` — Hardcore mode toggle (disables debugger)

---

## 8. HIDDEN/ADVANCED FEATURES

### Advanced Settings Toggle
- Warning dialog on first enable
- Shows/hides Debug menu
- Shows/hides System Console, Debug Console, Verbose Logging options

### Mouse Lock System
- Platform-specific (X11 on Linux, raw input on Windows)
- Locks cursor to display window bounds
- Configurable via `EmuCore/EnableMouseLock`

### Device Notifications (Windows)
- Listens for `WM_DEVICECHANGE` → reloads input devices
- Raw input for mouse position tracking

### Resume State Prompt
- On game launch, checks for resume save state
- Options: Load State, Fresh Boot, Delete And Boot, Cancel

### Per-Game Volume
- Status bar volume can be adjusted per-game
- Saves to game settings layer or global settings

### VMLock (Scoped VM Lock)
- Pauses VM for modal dialogs
- Exits fullscreen temporarily
- Restores fullscreen on unlock
- Supports cancelResume() for shutdown scenarios

### Window Size Scale Menu
- Dynamic 1x-10x scale options
- Internal Resolution option
- Calls `g_emu_thread->requestDisplaySize(scale)`

### Renderer Switch Menu (Debug)
- Dynamic menu built from available renderers
- Auto/DX11/DX12/Metal/OGL/VK/SW/Null
- Platform-specific availability

### Memory Card Busy Protection
- Warns on shutdown if memory card is saving
- Prevents data corruption

### Log Window Integration
- Attached/detached to main window
- Reacts to move/resize events
- Settings-based visibility

---

## 9. SETTINGS CATEGORIES (14 categories)
1. Interface — Theme, Language, Background, Mouse Lock, Confirm on Exit, etc.
2. Game List — Search directories, recursive scan, excluded paths
3. BIOS — BIOS directory, fast boot
4. Emulation — Speed, frame pacing, VSync, timing
5. Graphics — Renderer, resolution, filtering, post-processing, texture replacement
6. On-Screen Display — Performance overlay, notifications, OSD elements
7. Audio — Backend, volume, latency, sync mode
8. Memory Cards — Port 1/2, card type, size
9. Network & HDD (DEV9) — Network adapter, HDD image
10. Folders — Custom paths for saves, snapshots, logs, etc.
11. Achievements — RetroAchievements login, hardcore, notifications
12. Controllers — Global settings, per-port binding, hotkeys, USB devices, LED
13. Hotkeys — Keyboard/controller hotkey bindings
14. Game Properties — Per-game settings (all categories above, per-game override)

---

## 10. SUB-WINDOWS
| Window | Class | Description |
|--------|-------|-------------|
| Settings | `SettingsWindow` | Main settings dialog with categories |
| Controller Settings | `ControllerSettingsWindow` | Separate window for controller/hotkey config |
| Debugger | `DebuggerWindow` | 13-view debugger (singleton) |
| Log Window | `LogWindow` | Debug log output |
| Input Recording Viewer | `InputRecordingViewer` | View/edit input recordings |
| Auto Updater | `AutoUpdaterDialog` | Update checker/downloader |
| Cover Download | `CoverDownloadDialog` | Batch cover downloader |
| Game Properties | `SettingsWindow` (per-game) | Per-game settings dialog |
| About | `AboutDialog` | About dialog |
| Achievement Login | `AchievementLoginDialog` | RetroAchievements login |
| Memory Card Create | `MemoryCardCreateDialog` | Create new memory card |
| Shortcut Creation | `ShortcutCreationDialog` | Create game shortcut (Windows/Linux) |

---

## 11. FILE FILTERS

### Open File Filter
- All: *.bin *.iso *.cue *.mdf *.chd *.cso *.zso *.gz *.elf *.irx *.gs *.gs.xz *.gs.zst *.dump
- Single-Track Raw: *.bin *.iso
- Cue Sheets: *.cue
- Media Descriptor: *.mdf
- MAME CHD: *.chd
- CSO: *.cso
- ZSO: *.zso
- GZ: *.gz
- ELF: *.elf
- IRX: *.irx
- GS Dumps: *.gs *.gs.xz *.gs.zst
- Block Dumps: *.dump

### Disc Image Filter
- Same as above but without ELF/IRX/GS dumps

### Save State Filter
- *.p2s *.p2s.backup

### Cover Image Filter
- *.jpg *.jpeg *.png *.webp

### Video Capture Filter
- Dynamic based on configured container (e.g., mp4, avi)
