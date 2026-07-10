# Analysis 43: QtHost.h / QtHost.cpp — EmuThread, VM Lifecycle, Host Bridge

## Files Analyzed
- `pcsx2-qt/QtHost.h` — EmuThread class declaration, QtHost namespace API
- `pcsx2-qt/QtHost.cpp` — 2582 lines, the heart of PCSX2 Qt: EmuThread loop, VM lifecycle, settings, signal/slot plumbing, CLI parsing, `main()` entry point

---

## 1. EmuThread Class — The CPU Thread

`EmuThread` extends `QThread` and runs the PS2 emulation CPU thread. It is the single bridge between Qt's UI thread and the emulation core.

### 1.1 Lifecycle Methods (public slots)

| Slot | Purpose |
|------|---------|
| `startVM(VMBootParameters)` | Boot a game. Sets fullscreen state, starts async VM initialization with hardcore-disable-callback and done-callback. Done callback calls `VMManager::SetState(Running)` or stays paused if `StartPaused` setting. |
| `shutdownVM(save_state)` | Stops VM. Sets `m_save_state_on_shutdown`, calls `VMManager::SetState(Stopping)`. If paused, quits event loop first. |
| `resetVM()` | Calls `VMManager::Reset()` |
| `setVMPaused(bool)` | Calls `VMManager::SetPaused(paused)` |
| `startFullscreenUI(bool)` | Launches Big Picture / FullscreenUI mode. Initializes ImGui fullscreen UI, opens MTGS display. Polls controllers at 8ms interval (vs 100ms for background). Emits `onFullscreenUIStateChange(true)` |
| `stopFullscreenUI()` | Stops FSUI. Waits for MTGS to close. Restores normal controller polling. Calls `updateGameListBackground` on main window. |

### 1.2 Save State Methods

| Slot | Purpose |
|------|---------|
| `loadState(QString filename)` | Loads state from file path. Reports error via `g_main_window->reportStateLoadError` |
| `loadStateFromSlot(qint32 slot, bool load_backup)` | Loads from numbered slot. Supports backup loading. |
| `saveState(QString filename)` | Saves state to file. Async write. Error reports via `reportStateSaveError` |
| `saveStateToSlot(qint32 slot)` | Saves to numbered slot. |

### 1.3 Display / Rendering

| Slot/Method | Purpose |
|-------------|---------|
| `acquireRenderWindow(bool recreate)` | Called from GS thread. Checks exclusive fullscreen, window fullscreen, render-to-main. Emits `onAcquireRenderWindowRequested` signal. |
| `releaseRenderWindow()` | Emits `onReleaseRenderWindowRequested` |
| `connectDisplaySignals(DisplaySurface*)` | Connects widget resize/restore events to EmuThread handlers |
| `toggleFullscreen()` | Toggles m_is_fullscreen |
| `setFullscreen(bool, bool allow_render_to_main)` | Sets fullscreen. HACK: blocked when `s_vm_locked_with_dialog > 0` to prevent crash from destroying dialog mid-exec. |
| `setSurfaceless(bool)` | Makes the display surfaceless (no rendering) |
| `requestDisplaySize(float scale)` | Requests display resize via VMManager |
| `redrawDisplayWindow()` | Presents current frame if VM exists but isn't running (paused) |

### 1.4 Settings / Config

| Slot | Purpose |
|------|---------|
| `loadSettings(SettingsInterface&, mutex lock)` | Reads `VerboseStatusBar` and `PauseOnFocusLoss` from UI settings |
| `checkForSettingChanges(Pcsx2Config& old_config)` | Invokes `MainWindow::checkForSettingChanges`. Updates render-to-main if changed. |
| `applySettings()` | Calls `VMManager::ApplySettings()` |
| `reloadGameSettings()` | Calls `VMManager::ReloadGameSettings()` |
| `updateEmuFolders()` | Calls `VMManager::Internal::UpdateEmuFolders()` |

### 1.5 Input

| Slot | Purpose |
|------|---------|
| `reloadInputSources()` | `VMManager::ReloadInputSources()` |
| `reloadInputBindings()` | `VMManager::ReloadInputBindings()` |
| `reloadInputDevices()` | `InputManager::ReloadDevices()` |
| `closeInputSources()` | `InputManager::CloseSources()` — uses `BlockingQueuedConnection` |
| `enumerateInputDevices()` | Enumerates devices, emits `onInputDevicesEnumerated(QList<QPair<QString,QString>>)` |
| `enumerateVibrationMotors()` | Enumerates motors, emits `onVibrationMotorsEnumerated(QList<InputBindingKey>)` |

### 1.6 Disc / ELF / Patches

| Slot | Purpose |
|------|---------|
| `changeDisc(CDVD_SourceType, QString path)` | Changes disc source at runtime |
| `setELFOverride(QString path)` | Overrides boot ELF |
| `changeGSDump(QString path)` | Changes GS dump source |
| `reloadPatches()` | Calls `VMManager::ReloadPatches(true, false, true, true)` |

### 1.7 Capture / Snapshot

| Slot | Purpose |
|------|---------|
| `queueSnapshot(quint32 gsdump_frames)` | Queues GS snapshot via MTGS |
| `beginCapture(QString path)` | Starts video capture via `GSBeginCapture`. Waits for GS sync. |
| `endCapture()` | Stops video capture via `GSEndCapture` |

### 1.8 Performance Metrics

`updatePerformanceMetrics(bool force)` — The status bar updater. Reads from `PerformanceMetrics` and GS, updates MainWindow labels via `QMetaObject::invokeMethod`:

- **Slot**: current save state slot number
- **GS stats** (verbose mode): EE%, VU%, GS% thread usage + internal resolution info
- **Renderer**: current GS renderer type (Auto, Vulkan, etc.)
- **Resolution**: internal width x height + upscale multiplier
- **GPU usage**: percentage
- **FPS**: internal game FPS
- **VPS**: video frames per second
- **Speed**: emulation speed percentage
- **Limiter mode**: icon changes (fast-forward, slow-mo, speed-line, dashboard)
- **Volume**: percentage or "Muted"

### 1.9 Signals (EmuThread emits → MainWindow/UI receives)

| Signal | Payload | When |
|--------|---------|------|
| `statusMessage(QString)` | Status bar text | Various info messages |
| `onVMStarting()` | none | VM init begins |
| `onVMStarted()` | none | VM created |
| `onVMPaused()` | none | VM paused |
| `onVMResumed()` | none | VM resumed |
| `onVMStopped()` | none | VM destroyed |
| `onGameChanged(title, elf, disc_path, serial, disc_crc, crc)` | Game info | Running executable changes |
| `onAcquireRenderWindowRequested(recreate, fullscreen, render_to_main, surfaceless)` | Display params | GS thread needs window |
| `onResizeRenderWindowRequested(w, h)` | Dimensions | Display resize |
| `onReleaseRenderWindowRequested()` | none | Display release |
| `onMouseModeRequested(relative, hide_cursor)` | Mouse mode | GS thread request |
| `onMouseLockRequested(state)` | Lock state | GS thread request |
| `onFullscreenUIStateChange(bool running)` | FSUI state | Big Picture mode toggle |
| `onInputDevicesEnumerated(QList<QPair<QString,QString>>)` | Device list | After enumeration |
| `onInputDeviceConnected(id, name)` | Device info | Hotplug |
| `onInputDeviceDisconnected(id)` | Device id | Hot-unplug |
| `onVibrationMotorsEnumerated(QList<InputBindingKey>)` | Motor list | After enumeration |
| `onSaveStateLoading(path)` | Path | Before state load |
| `onSaveStateLoaded(path, was_successful)` | Path + success | After state load |
| `onSaveStateSaved(path)` | Path | After state save initiated |
| `onAchievementsLoginRequested(reason)` | Login reason | RA login needed |
| `onAchievementsRefreshed(id, game_info_string)` | Game info | RA refreshed |
| `onAchievementsHardcoreModeChanged(enabled)` | bool | Hardcore toggle |
| `onCaptureStarted(filename)` | Path | Video capture starts |
| `onCaptureStopped()` | none | Video capture stops |

### 1.10 Main Thread Loop (`run()`)

```
while (!shutdown_flag):
  switch VMState:
    Initializing → fail (shouldn't be here)
    Shutdown/Paused → event_loop.exec() [BLOCKS]
    Running → event_loop.processEvents() + VMManager::Execute()
    Resetting → VMManager::Reset()
    Stopping → destroyVM()
```

Key: Paused state blocks in event loop. Resume quits the event loop.

### 1.11 Application State Handler

`onApplicationStateChanged(Qt::ApplicationState state)`:
- **Focus loss**: If `PauseOnFocusLoss` setting enabled AND VM is running → pause VM. Also clears keyboard bind state to prevent stuck keys.
- **Focus gain**: If was paused by focus loss → resume VM.

### 1.12 Background Controller Polling

- **Normal mode**: 100ms interval (`BACKGROUND_CONTROLLER_POLLING_INTERVAL`)
- **Fullscreen UI mode**: 8ms interval (`FULLSCREEN_UI_CONTROLLER_POLLING_INTERVAL`)
- Timer type: `Qt::CoarseTimer`
- Polls `VMManager::IdlePollUpdate()`

---

## 2. QtHost Namespace — Host Bridge Functions

### 2.1 Thread Utilities

| Function | Purpose |
|----------|---------|
| `IsOnUIThread()` | Checks if current thread == QApplication thread |
| `RunOnUIThread(func, block)` | Invokes `g_main_window->runOnUIThread` via QMetaObject. Block=false by default (QueuedConnection). |
| `LockVMWithDialog()` / `UnlockVMWithDialog()` | Increments/decrements `s_vm_locked_with_dialog`. Prevents fullscreen toggles during modal dialogs. |

### 2.2 Theme / Appearance

| Function | Purpose |
|----------|---------|
| `GetDefaultThemeName()` | Platform-specific default theme |
| `GetDefaultLanguage()` | Platform-specific default language |
| `UpdateApplicationTheme()` | Applies theme from settings |
| `IsDarkApplicationTheme()` | Returns true if dark theme |
| `SetIconThemeFromStyle()` | Sets icon theme (light/dark) |

### 2.3 App Info

| Function | Returns |
|----------|---------|
| `GetAppNameAndVersion()` | `"PCSX2 {GitRev}"` |
| `GetAppConfigSuffix()` | `" [Debug]"` / `" [Devel]"` / empty |
| `GetAppIcon()` | QIcon from `:/icons/AppIcon64.png` |
| `GetResourcesBasePath()` | `EmuFolders::Resources` |
| `GetRuntimeDownloadedResourceURL(name)` | GitHub URL for runtime resources |
| `ShouldShowAdvancedSettings()` | `UI/ShowAdvancedSettings` bool |
| `GetAvailableLanguageList()` | List of (language_name, code) pairs |
| `InstallTranslator(dialog_parent)` | Loads translation for current language |

### 2.4 VM State Queries (Thread-Safe)

| Function | Returns |
|----------|---------|
| `IsVMValid()` | `VMManager::HasValidVM()` |
| `IsVMPaused()` | `VMManager::GetState() == Paused` |
| `GetCurrentGameTitle()` | Current game title string |
| `GetCurrentGameSerial()` | Current game serial string |
| `GetCurrentGamePath()` | Current game file path |

### 2.5 Settings

| Function | Purpose |
|----------|---------|
| `SaveGameSettings(sif, delete_if_empty)` | Saves INI. If empty, deletes file instead. Cleans empty sections. |
| `InitializeConfig()` | Loads `PCSX2.ini` + `secrets.ini`. Sets up base/secrets layers. Handles version check. Flags setup wizard if incomplete. |
| `SaveSettings()` | Saves `s_base_settings_interface` to disk. Deletes timer. |
| `CommitBaseSettingChanges()` | Debounced save — creates 1-second timer, saves once after changes settle. |

### 2.6 Download

| Function | Purpose |
|----------|---------|
| `DownloadFile(parent, title, url, data*)` | Downloads URL to memory with progress dialog. Returns `optional<bool>`. |
| `DownloadFile(parent, title, url, path)` | Downloads URL to file path. Creates directories as needed. |

### 2.7 Clipboard

| Function | Purpose |
|----------|---------|
| `InitializeClipboard()` | Monitors clipboard changes, caches text in `s_clipboard_cache` mutex-protected |
| `Host::CopyTextToClipboard(text)` | Sets clipboard text |
| `Host::GetTextFromClipboard()` | Returns cached clipboard text |

### 2.8 Localization

| Function | Purpose |
|----------|---------|
| `LocaleSensitiveCompare(lhs, rhs)` | Compares strings in current UI locale order |

---

## 3. Host::* Callbacks (Core→Qt Bridge)

These are implementations of the abstract `Host` interface that the emulation core calls:

### 3.1 VM Lifecycle Callbacks

| Callback | Action |
|----------|--------|
| `Host::OnVMStarting()` | Stops background controller poll, emits `onVMStarting` |
| `Host::OnVMStarted()` | Updates perf metrics, emits `onVMStarted` |
| `Host::OnVMDestroyed()` | Emits `onVMStopped`, restarts background poll |
| `Host::OnVMPaused()` | Restarts background poll, emits `onVMPaused` |
| `Host::OnVMResumed()` | Quits event loop (unblocks paused loop), stops background poll, un-surfaces display, emits `onVMResumed` |
| `Host::OnGameChanged(...)` | Emits `onGameChanged` with all game info |
| `Host::RequestVMShutdown(...)` | Routes to `g_main_window->requestShutdown` or directly calls `shutdownVM` |
| `Host::RequestExitApplication(allow_confirm)` | Invokes `g_main_window->requestExit` |
| `Host::RequestExitBigPicture()` | Calls `g_emu_thread->stopFullscreenUI()` |

### 3.2 Save State Callbacks

| Callback | Action |
|----------|--------|
| `Host::OnSaveStateLoading(filename)` | Emits `onSaveStateLoading` |
| `Host::OnSaveStateLoaded(filename, success)` | Emits `onSaveStateLoaded` |
| `Host::OnSaveStateSaved(filename)` | Emits `onSaveStateSaved` |

### 3.3 Achievement Callbacks

| Callback | Action |
|----------|--------|
| `Host::OnAchievementsLoginRequested(reason)` | Emits `onAchievementsLoginRequested` |
| `Host::OnAchievementsLoginSuccess(username, pts, sc_pts, unread)` | Shows status message "Logged in as X" |
| `Host::OnAchievementsRefreshed()` | Builds game info string, emits `onAchievementsRefreshed` |
| `Host::OnAchievementsHardcoreModeChanged(enabled)` | Emits `onAchievementsHardcoreModeChanged` |

### 3.4 Display Callbacks

| Callback | Action |
|----------|--------|
| `Host::AcquireRenderWindow(recreate)` | Delegates to `g_emu_thread->acquireRenderWindow` |
| `Host::ReleaseRenderWindow()` | Delegates to `g_emu_thread->releaseRenderWindow` |
| `Host::RequestResizeHostDisplay(w, h)` | Emits `onResizeRenderWindowRequested` |
| `Host::BeginPresentFrame()` | Empty (no-op) |
| `Host::IsFullscreen()` | Returns `g_emu_thread->isFullscreen()` |
| `Host::SetFullscreen(enabled)` | Calls `g_emu_thread->setFullscreen(enabled, true)` |

### 3.5 Input Callbacks

| Callback | Action |
|----------|--------|
| `Host::OnInputDeviceConnected(id, name)` | Emits signal + shows OSD message "Controller connected" |
| `Host::OnInputDeviceDisconnected(key, id)` | Emits signal + OSD message. If `PauseOnControllerDisconnection` setting enabled, pauses VM. |
| `Host::SetMouseMode(relative, hide)` | Emits `onMouseModeRequested` |
| `Host::SetMouseLock(state)` | Emits `onMouseLockRequested` |

### 3.6 File/Dialog Callbacks

| Callback | Action |
|----------|--------|
| `Host::OpenHostFileSelectorAsync(...)` | Runs QFileDialog on UI thread. Pauses+locks VM during dialog. Returns path via callback. |
| `Host::ShouldPreferHostFileSelector()` | Linux flatpak detection |
| `Host::ReportInfoAsync(title, msg)` | Logs + invokes `g_main_window->reportInfo` |
| `Host::ReportErrorAsync(title, msg)` | Logs + invokes `g_main_window->reportError` |
| `Host::OpenURL(url)` | Opens URL via `QtUtils::OpenURL` |
| `Host::BeginTextInput()` / `EndTextInput()` | Shows/hides virtual keyboard (QInputMethod) |
| `Host::GetTopLevelWindowInfo()` | Blocking call to get main window info |
| `Host::PumpMessagesOnCPUThread()` | Processes events on emu thread's event loop |
| `Host::RunOnCPUThread(func, block)` | Invokes `runOnCPUThread` on emu thread. Uses BlockingQueuedConnection if block=true. |
| `Host::RunOnGSThread(func)` | Wraps in RunOnCPUThread → MTGS::RunOnGSThread |
| `Host::RefreshGameListAsync(invalidate)` | Invokes `g_main_window->refreshGameList` |
| `Host::CancelGameListRefresh()` | Blocking invoke of `cancelGameListRefresh` |
| `Host::OnPerformanceMetricsUpdated()` | Calls `updatePerformanceMetrics(false)` |
| `Host::CreateHostProgressCallback()` | Returns `QtHostProgressCallback` instance |
| `Host::RequestResetSettings(...)` | Resets settings via `VMManager::SetDefaultSettings`, commits, applies |

### 3.7 Thread-Safe Settings Access

The settings system uses a layered approach:
1. `s_base_settings_interface` — main `PCSX2.ini` (INISettingsInterface)
2. `s_secrets_settings_interface` — `secrets.ini` for sensitive data
3. Both are set via `Host::Internal::SetBaseSettingsLayer` / `SetSecretsSettingsLayer`

Save is debounced: `CommitBaseSettingChanges()` creates a 1-second `QTimer`, saves once after changes stop arriving.

---

## 4. Command-Line Interface (CLI)

### 4.1 CLI Flags

| Flag | Effect |
|------|--------|
| `-help` | Print help and exit |
| `-version` | Print version and exit |
| `-batch` | Exit after game shuts down (`s_batch_mode = true`) |
| `-nogui` | Hide main window while running (implies batch) |
| `-portable` | Force portable mode |
| `-datapath <path>` | Custom data directory |
| `-fastboot` | Force fast boot |
| `-slowboot` | Force slow boot |
| `-state <index>` | Load save state by index at boot |
| `-statefile <filename>` | Load state from file at boot |
| `-elf <file>` | Override boot ELF |
| `-gameargs <string>` | Pass game launch arguments |
| `-disc <path>` | Use host DVD drive |
| `-logfile <path>` | Custom log file path |
| `-bios` | Boot BIOS (NoDisc) |
| `-fullscreen` | Start in fullscreen |
| `-nofullscreen` | Prevent fullscreen |
| `-bigpicture` | Force Big Picture mode |
| `-earlyconsolelog` | Enable early console logging |
| `-testconfig` | Check config and exit |
| `-setupwizard` | Force setup wizard |
| `-debugger` | Open debugger, break on entry |
| `-turbo` | Start in turbo mode |
| `-unlimited` | Start in unlimited speed mode |
| `-raintegration` | Use RAIntegration (Windows only) |
| `-updatecleanup` | Clean up after auto-update |
| `--` | End of flags, rest is filename |

### 4.2 CLI Parsing Edge Cases

- Space-delimited filenames are reconstructed: `file name.iso` → `file name.iso`
- If `-turbo` and `-unlimited` both specified, `-unlimited` wins
- If batch mode but no autoboot and no bigpicture → error exit
- If nogui mode but no autoboot → error exit

---

## 5. `main()` Entry Point — Full Boot Sequence

1. `CrashHandler::Install()`
2. Set locale on Windows
3. Set HighDPI rounding policy (PassThrough)
4. `RegisterTypes()` — registers all QMetaType types for signal/slot
5. Create `PCSX2MainApplication` (extends QApplication, handles `QEvent::FileOpen` for macOS drag-drop)
6. `InitializeClipboard()` — monitor clipboard
7. Hardware checks on non-Windows
8. `ParseCommandLineOptions()` — builds autoboot params
9. `InitializeConfig()` — load PCSX2.ini, secrets.ini, setup wizard check
10. If `-testconfig` → exit
11. If `-updatecleanup` → `AutoUpdaterDialog::cleanupAfterUpdate()`
12. `UpdateApplicationTheme()` — apply theme
13. `LogWindow::updateSettings()` — start logging
14. **`EmuThread::start()`** — creates and starts the emu thread
15. If setup wizard needed → `RunSetupWizard()`
16. Create `g_main_window = new MainWindow()` → `initialize()`
17. Refresh game list (unless batch mode)
18. Show window (unless nogui mode)
19. If big picture mode → `startFullscreenUI()`
20. If debugger → open debugger window
21. If autoboot → `startVM(autoboot)`
22. Else → `startupUpdateCheck()`
23. `app.exec()` — main Qt event loop
24. Shutdown: `EmuThread::stop()`, close/delete main window, save dirty config

### 5.1 Custom Application Class

`PCSX2MainApplication` overrides `event()` to handle `QEvent::FileOpen` — macOS file open events (drag-drop to dock icon).

---

## 6. QtHostProgressCallback — Progress Dialog System

Used for long operations (save states, downloads, game list scanning):

- Creates `QProgressDialog` lazily on first update
- If fullscreen → exits fullscreen before showing dialog, restores after
- Minimum width: 400px
- Cancellable via "Cancel" button
- Push/pop state stack for nested operations
- Progress updates via `Redraw()` → `QtHost::RunOnUIThread`
- On emu thread: pumps messages during redraw to process fullscreen exit

---

## 7. Signal Handler

- `SIGINT` / `SIGTERM` → graceful shutdown via `requestExit(false)`
- Second signal → force exit via `quick_exit(1)` / `_Exit(1)`
- Windows: `ConsoleCtrlHandler` for `CTRL_C_EVENT`
- Linux: `SIGCHLD` set to `SA_NOCLDSTOP | SA_NOCLDWAIT` (for async `aplay`)

---

## 8. Settings Defaults (`Host::SetDefaultUISettings`)

| Setting | Key | Default |
|---------|-----|---------|
| Inhibit Screensaver | `UI/InhibitScreensaver` | true |
| Confirm Shutdown | `UI/ConfirmShutdown` | true |
| Start Paused | `UI/StartPaused` | false |
| Pause On Focus Loss | `UI/PauseOnFocusLoss` | false |
| Start Fullscreen | `UI/StartFullscreen` | false |
| Double Click Toggles Fullscreen | `UI/DoubleClickTogglesFullscreen` | true |
| Hide Mouse Cursor | `UI/HideMouseCursor` | false |
| Render To Separate Window | `UI/RenderToSeparateWindow` | false |
| Hide MainWindow When Running | `UI/HideMainWindowWhenRunning` | false |
| Disable Window Resize | `UI/DisableWindowResize` | false |
| Prefer English Game List | `UI/PreferEnglishGameList` | false |
| Theme | `UI/Theme` | platform default |
| Show Advanced Settings | `UI/ShowAdvancedSettings` | false |
| Verbose Status Bar | `UI/VerboseStatusBar` | false |
| Start Big Picture Mode | `UI/StartBigPictureMode` | false |
| Setup Wizard Incomplete | `UI/SetupWizardIncomplete` | (set on first run) |
| Pause On Controller Disconnection | `UI/PauseOnControllerDisconnection` | false |

---

## 9. HIDDEN / SUBTLE Features

### 9.1 VM Lock with Dialog (`LockVMWithDialog`/`UnlockVMWithDialog`)
A reference-counted lock that prevents fullscreen toggles while modal dialogs (like file pickers) are open. Prevents crash from destroying dialog widget mid-exec.

### 9.2 Focus-Loss Keyboard State Clearing
When app loses focus, `InputManager::ClearBindStateFromSource(keyboard)` is called to prevent stuck keys (e.g., held shift that the user released in another window).

### 9.3 Controller Disconnection Auto-Pause
Hidden setting `UI/PauseOnControllerDisconnection` — automatically pauses the VM when a controller with active bindings disconnects. Shows OSD warning message.

### 9.4 Clipboard Cache with Mutex
Clipboard content is cached in a mutex-protected string (`s_clipboard_cache`) that updates on every clipboard change. `GetTextFromClipboard()` reads the cache, not the live clipboard — thread-safe for emu thread access.

### 9.5 Progress Callback Fullscreen Escape
When a progress dialog needs to show while in fullscreen, it automatically exits fullscreen, shows the dialog, and restores fullscreen on completion.

### 9.6 Debounced Settings Save
Settings don't save immediately on every change. A 1-second timer (`SETTINGS_SAVE_DELAY = 1000`) accumulates changes, then saves once. Prevents thrashing disk on rapid setting changes.

### 9.7 Dual INI Layer (Base + Secrets)
Settings are split into `PCSX2.ini` (normal) and `secrets.ini` (sensitive data like RA tokens). Secrets layer is overlaid on top of base.

### 9.8 macOS File Open Event Handling
`PCSX2MainApplication::event()` handles `QEvent::FileOpen` for macOS where files can be dragged to the dock icon.

### 9.9 Signal Handler Graceful Shutdown
CTRL+C first attempts graceful shutdown (requestExit). Second CTRL+C forces exit. Prevents data loss from abrupt termination.

### 9.10 Setup Wizard Incomplete Flag
`UI/SetupWizardIncomplete` is set to true on first run. If wizard is not completed, it runs again next startup. Only cleared on successful wizard completion.

### 9.11 Surfaceless Display for Background Running
When VM is paused and user switches to game list, display becomes "surfaceless" (no rendering). On resume, `setSurfaceless(false)` restores the display automatically.

### 9.12 Exclusive Fullscreen Detection
`acquireRenderWindow` checks `GSWantsExclusiveFullscreen()` — if true, enters exclusive fullscreen (different from window fullscreen). Only one mode is active.

### 9.13 Turbo/Unlimited Frame Limiter
CLI flags `-turbo` and `-unlimited` set on `VMBootParameters`. If both specified, unlimited wins with warning. These map to `LimiterModeType::Turbo` / `LimiterModeType::Unlimited`.

### 9.14 `Host::RunOnCPUThread` Recursive Safety
If called with `block=true` from the emu thread itself, executes directly instead of deadlocking.

### 9.15 `Host::RunOnGSThread` Double-Hop
GSThread calls go through CPUThread first: `RunOnCPUThread → MTGS::RunOnGSThread`. This ensures thread-safe access to the GS thread.

### 9.16 Status Bar Verbose Mode
When `UI/VerboseStatusBar` is true, shows EE%, VU%, GS% thread usage and internal resolution in the status bar. Otherwise just basic FPS/speed.

### 9.17 Empty Game Settings Auto-Delete
`SaveGameSettings()` with `delete_if_empty=true` deletes the INI file if it has no keys. Also cleans empty sections before save.

### 9.18 Background Controller Polling Dual Rate
- Normal: 100ms (10 Hz) — low overhead
- Fullscreen UI: 8ms (125 Hz) — matches ~120fps to reduce missed input events

### 9.19 `host_hotkeys` List
Empty `BEGIN_HOTKEY_LIST(g_host_hotkeys) / END_HOTKEY_LIST()` — the host-level hotkeys are registered elsewhere (likely in MainWindow).

### 9.20 `runOnCPUThread` Slot
A simple slot that just calls `func()`. Used as the target for `QMetaObject::invokeMethod` to run arbitrary lambdas on the emu thread.

---

## 10. Global Variables

| Variable | Type | Purpose |
|----------|------|---------|
| `g_emu_thread` | `EmuThread*` | Singleton emu thread |
| `s_settings_save_timer` | `QTimer*` | Debounce timer for settings save |
| `s_base_settings_interface` | `unique_ptr<INISettingsInterface>` | Main config |
| `s_secrets_settings_interface` | `unique_ptr<INISettingsInterface>` | Secrets config |
| `s_batch_mode` | `bool` | CLI batch mode |
| `s_nogui_mode` | `bool` | CLI no-gui mode |
| `s_start_big_picture_mode` | `bool` | CLI big picture |
| `s_start_fullscreen` | `bool` | CLI fullscreen |
| `s_test_config_and_exit` | `bool` | CLI test config |
| `s_run_setup_wizard` | `bool` | Force setup wizard |
| `s_cleanup_after_update` | `bool` | Post-update cleanup |
| `s_boot_and_debug` | `bool` | CLI debugger mode |
| `s_vm_locked_with_dialog` | `atomic_int` | Dialog lock counter |
| `s_clipboard_cache` | `string` (mutex-protected) | Clipboard cache |

---

## 11. Key Architectural Patterns

### 11.1 Thread Marshalling pattern
Every EmuThread method follows the same pattern:
```cpp
if (!isOnEmuThread()) {
    QMetaObject::invokeMethod(this, "methodName", Qt::QueuedConnection, Q_ARG(...));
    return;
}
// actual implementation
```
This ensures all emulation work happens on the emu thread, even when called from UI.

### 11.2 Signal/Slot Bridge
EmuThread declares signals for every VM event. Core `Host::` callbacks emit these signals. MainWindow connects to them. This decouples core from UI.

### 11.3 Settings Layer Stack
```
Layer 0: Defaults (VMManager::SetDefaultSettings)
Layer 1: PCSX2.ini (s_base_settings_interface)
Layer 2: secrets.ini (s_secrets_settings_interface)
Layer 3: Per-game settings (GameSettings/*.ini)
```

### 11.4 Event Loop for Pause/Resume
The emu thread uses `QEventLoop::exec()` when paused/shutdown. `onVMResumed()` calls `m_event_loop->quit()` to unblock. This elegantly handles the pause state without busy-waiting.
