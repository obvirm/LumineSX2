# LogWindow — Deep Analysis

## Files
- `pcsx2-qt/LogWindow.h` (44 lines)
- `pcsx2-qt/LogWindow.cpp` (327 lines)

## Window Structure
- Inherits `QMainWindow`
- Global singleton: `g_log_window` protected by `s_log_mutex`
- Default size: 750×400, persisted to settings (`UI/LogWindowWidth`, `UI/LogWindowHeight`)
- Can't be closed by user (close button hidden, `closeEvent` ignored unless `m_destroying == true`)

## Menu Bar

### Log Menu (`&Log`)
1. **&Clear** → `onClearTriggered()` — clears all text
2. **&Save...** → `onSaveTriggered()` — file dialog, saves as UTF-8 `.txt`
3. **Cl&ose** → close window

### Settings Menu (`&Settings`)
1. **Attach To &Main Window** — `bool` bound to `Logging/AttachLogWindowToMainWindow` (default: true)
   - When enabled, log window repositions itself to the right of main window on `reattachToMainWindow()`
2. **Show &Timestamps** — `bool` bound to `Logging/EnableTimestamps` (default: true)
   - Timestamps formatted as `[  0.1234]` (10-char float, 4 decimals) in gray `#CCCCCC`
3. **Show EE SIO &Input** — `bool` bound to `Logging/ShowEESIOInput` (default: false)
   - Shows/hides the input widget at bottom

## Log Display (`m_text` — QPlainTextEdit)
- Read-only, no undo/redo
- Selectable by keyboard and mouse
- Scrollbar always on
- Word wrap at any character (`WrapAnywhere`)
- Platform-specific monospace fonts:
  - Windows: Consolas 10pt
  - macOS: Monaco 11pt
  - Linux: Monospace (TypeWriter hint)

## Color System (22 colors × 2 themes)
Full color array for light and dark themes:
- `Color_Default`, `Color_Black`, `Color_Red`, `Color_Green`, `Color_Blue`, `Color_Magenta`, `Color_Orange`, `Color_Gray`
- `Color_Cyan`, `Color_Yellow`, `Color_White`
- `Color_StrongBlack` through `Color_StrongWhite` (bright variants)
- Theme detection: `QtHost::IsDarkApplicationTheme()`
- Timestamps always gray `#CCCCCC` regardless of theme

## Log Level Detection
```
#ifdef _DEBUG → LOGLEVEL_DEBUG
#else → (IsDevBuild || Logging/EnableVerbose) ? LOGLEVEL_DEV : LOGLEVEL_INFO
```

## EE SIO Input System
- `m_line_input` (QLineEdit) — text input for EE SIO RX FIFO
- `m_local_echo_checkbox` — echo sent text to log window
- `m_newline_on_enter_checkbox` — append `\n` to input on Enter (default: true)
- Flow:
  1. User types in input, presses Enter
  2. Input disabled during send
  3. `Host::RunOnCPUThread` → validates VM running + not hardcore mode
  4. `VMManager::WriteBytesToEESIORXFIFO()` sends bytes
  5. On success: clears input, optionally echoes to log
  6. Re-enables input, restores focus
- **Hardcore mode guard**: Blocks EE SIO input when RetroAchievements hardcore is active

## Attach/Detach Behavior
- `m_attached_to_main_window` controls positioning
- `reattachToMainWindow()` places window at `main_window.pos + (main_window.width + 10, 0)`
- Skips reattach if main window is maximized or fullscreen
- Can be toggled at runtime via settings menu

## Thread Safety
- `s_log_mutex` protects all access to `g_log_window`
- `logCallback` is called from any thread; uses `QMetaObject::invokeMethod` with `QueuedConnection` for non-UI threads
- Direct call to `appendMessage` when already on UI thread

## Scroll Behavior
- Tracks cursor position and scroll position before append
- If cursor was at end AND scroll at end → auto-scroll to new content
- If cursor was at end but scroll NOT at end → preserve scroll position (user reading history)
- Smart scroll preservation during rapid log output

## Window Title
- Default: `"Log Window"`
- When VM running: `"Log Window - SERIAL [filename]"`

## Settings Bound
| Setting | Type | Default | Purpose |
|---------|------|---------|---------|
| `Logging/EnableLogWindow` | bool | false | Show log window |
| `Logging/AttachLogWindowToMainWindow` | bool | true | Attach positioning |
| `Logging/EnableTimestamps` | bool | true | Show timestamps |
| `Logging/ShowEESIOInput` | bool | false | Show EE SIO input |
| `Logging/EnableVerbose` | bool | false | Verbose log level |
| `Logging/EnableSystemConsole` | bool | false | (TODO: commented out) |
| `Logging/EnableDebugConsole` | bool | false | (TODO: commented out) |
| `Logging/EnableFileLogging` | bool | false | (TODO: commented out) |
| `UI/LogWindowWidth` | int | 750 | Window width |
| `UI/LogWindowHeight` | int | 400 | Window height |

## Hidden/Advanced Features
1. **Can't be closed normally** — close button hidden via `Qt::WindowCloseButtonHint`, closeEvent ignored
2. **EE SIO injection** — send arbitrary bytes to PS2's EE SIO RX FIFO
3. **Hardcore mode enforcement** — blocks SIO input in RA hardcore
4. **Smart scroll preservation** — doesn't jump when reading history
5. **Thread-safe log callback** — handles cross-thread log messages
6. **Auto-positioning** — docks to right side of main window
7. **Local echo option** — for SIO input debugging
8. **Log level auto-detection** — debug builds get DEBUG level, release gets INFO/DEV
9. **3 console/file log outputs** — commented out TODO (duplicated with main window settings)

## Porting Notes for Slint
- Log display = read-only ScrollView with colored Text elements
- Need color map for all 22 console colors
- EE SIO input = optional bottom section with text field + checkboxes
- Settings menu = checkbox toggles (attach, timestamps, EE SIO)
- Save = file picker, write text content
- Clear = reset content array
- Smart auto-scroll = track scroll position
- Thread-safe append via model-based approach (add items to model, not direct text manipulation)
