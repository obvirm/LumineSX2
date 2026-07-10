# 24. DisplayWidget (DisplaySurface) — Deep Analysis

## Files Analyzed
- `pcsx2-qt/DisplayWidget.h`
- `pcsx2-qt/DisplayWidget.cpp`

## Architecture

`DisplaySurface` extends `QWindow`. It is the actual rendering surface for the PS2 game.
A `QWidget` container is created via `createWindowContainer()` for compatibility (popups, geometry save/restore).

---

## Features Extracted

### 1. Display Surface Management
- **createWindowContainer(QWidget* parent)**: Wraps QWindow in QWidget container for popup/parent compatibility.
  - Container gets `eventFilter` installed, `StrongFocus` policy.
- **getWindowInfo()**: Returns `WindowInfo` (surface width, height, scale). Caches `m_last_window_*`.
- **saveGeometry() / restoreGeometry()**: Uses container if available, else creates dummy QWidget to copy geometry (QWindow lacks these methods natively).

### 2. Relative Mouse Mode
- **updateRelativeMode(bool enabled)**: Two strategies:
  - **Windows**: `ClipCursor()` (clips cursor to window rect) OR warping movement. Prefers ClipCursor when raw input is active.
  - **Non-Windows**: Warp-based — saves cursor start pos, warps cursor to center on each move, reads delta from center offset.
- **m_relative_mouse_enabled**: Tracks state.
- **m_clip_mouse_enabled** (Windows-only): Tracks clip cursor state.
- **m_relative_mouse_start_pos**: Position to restore cursor when exiting relative mode.
- **m_relative_mouse_center_pos**: Center of window for warp delta calculation.
- On disable: restores cursor to original start position, releases mouse grab.

### 3. Cursor Hiding
- **updateCursor(bool hidden)**: Sets `Qt::BlankCursor` when hidden, `unsetCursor()` when shown.
- Tracks `m_cursor_hidden` to avoid redundant updates.

### 4. Fullscreen Toggle on Double-Click
- Triggered on `QEvent::MouseButtonDblClick` with `Qt::LeftButton`.
- **Conditions** (all must be true):
  - VM is valid (`QtHost::IsVMValid()`)
  - No ImGui fullscreen UI active window (`!FullscreenUI::HasActiveWindow()`)
  - Either: VM not paused AND left mouse button has no bindings, OR VM paused AND ImGui doesn't want mouse input
  - Setting `UI/DoubleClickTogglesFullscreen` is true (default: true)
- Action: `g_emu_thread->toggleFullscreen()`

### 5. Keyboard Forwarding
- **handleKeyInputEvent()**: Handles `KeyPress` / `KeyRelease`.
  - If ImGui wants text input: forwards text (minus backspace char) via `ImGuiManager::AddTextInput()`.
  - Skips auto-repeat events.
  - **Windows fake-key workaround**: Tracks keys pressed with modifiers in `m_keys_pressed_with_modifiers` vector. Discards spurious duplicate press events from Windows (e.g., Shift+F1 → release Shift → Windows sends fake F1 Press again).
  - Dispatches to `InputManager::InvokeEvents()` on CPU thread via `Host::RunOnCPUThread()`.
- **eventFilter()**: Intercepts key events on the container widget.
  - On Windows: calls `requestActivate()` to refocus child window (NVIDIA overlay steals focus).
  - macOS FocusIn: activates parent MainWindow when display window gets focus (toolbar display).

### 6. Mouse Events (Absolute & Relative)
- **QEvent::MouseMove**:
  - Absolute mode: Scales position by `devicePixelRatio()`, calls `InputManager::UpdatePointerAbsolutePosition()`.
  - Relative mode: Gets cursor pos, computes delta from center, resets cursor to center.
    - Windows: uses `GetCursorPos()` / `SetCursorPos()` (higher precision than Qt at DPI scaling).
    - Non-Windows: uses `QCursor::pos()` / `QCursor::setPos()`.
- **MouseButtonPress / MouseButtonDblClick / MouseButtonRelease**:
  - Dispatches button index via `InputManager::InvokeEvents()` on CPU thread.
  - Uses `std::countr_zero()` to get button index from button bitmask.

### 7. Mouse Wheel
- **QEvent::Wheel**: Reads `angleDelta()`.
  - Horizontal: `InputPointerAxis::WheelX`, clamped to [-1, 1].
  - Vertical: `InputPointerAxis::WheelY`, clamped to [-1, 1].
  - Delta divided by `QtUtils::MOUSE_WHEEL_DELTA` for normalization.

### 8. Drag & Drop
- **QEvent::DragEnter**: Emits `dragEnterEvent` signal (forwards to MainWindow for file drops).
- **QEvent::Drop**: Emits `dropEvent` signal.
- Both call `QWindow::event(event)` first, then emit the signal, then return `event->isAccepted()`.

### 9. Resize Debounce
- **Problem**: Qt spams resize events (sometimes several per ms). Vulkan swapchain resize takes 15-25ms.
- **Solution**: `m_resize_debounce_timer` — 100ms single-shot `QTimer` with `Qt::PreciseTimer`.
- On `Resize` or `DevicePixelRatioChange`:
  - Computes scaled dimensions: `width * dpr`, `height * dpr` (minimum 1px).
  - Only triggers if dimensions or scale actually changed (dedup).
  - Stores pending values, restarts debounce timer.
- **onResizeDebounceTimer()**: Emits `windowResizedEvent(width, height, scale)`.
- Also updates center pos (for relative mouse mode) on resize.

### 10. Window State Events
- **Close**: If VM running and not fullscreen → `requestShutdown(prompt=true, allow_cancel=false, save_state=false)`. If fullscreen → `requestExit(prompt=true)`. Always cancels the close event (`event->ignore()`).
- **WindowStateChange**: If old state was minimized → emits `windowRestoredEvent()`.
- **Move**: Updates center position for relative mouse.

### 11. Focus Management
- **setFocus()**: If container exists, focuses container. Otherwise `requestActivate()`.
- **isFullScreen()**: Checks parent window state if in container, else own window state.

---

## Hidden / Subtle Features

1. **NVIDIA Overlay Focus Steal** (Windows): eventFilter calls `requestActivate()` on every key event because NVIDIA overlay defocuses the child window without defocusing parent.

2. **macOS Toolbar Focus**: When display window gets FocusIn, activates MainWindow so macOS shows the toolbar.

3. **Windows Fake Key Workaround**: Maintains `m_keys_pressed_with_modifiers` list to suppress spurious press events when modifier keys are released before the main key.

4. **DPI Precision**: Uses WinAPI `GetCursorPos`/`SetCursorPos` on Windows instead of Qt's `QCursor::pos()` for higher precision at non-100% DPI scaling.

5. **ClipCursor vs Warp**: Two relative mouse strategies — ClipCursor (for raw input path) vs cursor warping (default). Current code has `clip_cursor = enabled && false` — **ClipCursor is disabled** (hardcoded false).

6. **Geometry Save/Restore Workaround**: QWindow lacks saveGeometry/restoreGeometry, so creates temporary dummy QWidget to serialize/deserialize geometry.

7. **ChildWindowRemoved cleanup**: eventFilter detects `QEvent::ChildWindowRemoved` and nulls `m_container` pointer to avoid dangling reference.

8. **Fullscreen toggle guard**: Double-click fullscreen is skipped if left mouse button has any input bindings (prevents accidental fullscreen when button is mapped for gameplay).

---

## Signals (for MainWindow / EmuThread connection)
| Signal | Args | Purpose |
|--------|------|---------|
| `windowResizedEvent` | `u32 w, u32 h, float scale` | Debounced resize notification |
| `windowRestoredEvent` | — | Window restored from minimized |
| `dragEnterEvent` | `QDragEnterEvent*` | File drag enter (forwarded) |
| `dropEvent` | `QDropEvent*` | File drop (forwarded) |

---

## Member Variables
| Variable | Type | Purpose |
|----------|------|---------|
| `m_relative_mouse_start_pos` | `QPoint` | Cursor pos before entering relative mode |
| `m_relative_mouse_center_pos` | `QPoint` | Window center for delta calculation |
| `m_relative_mouse_enabled` | `bool` | Relative mouse mode active |
| `m_clip_mouse_enabled` | `bool` | ClipCursor mode (Windows only) |
| `m_cursor_hidden` | `bool` | Cursor visibility state |
| `m_keys_pressed_with_modifiers` | `vector<int>` | Tracks keys for fake-key workaround |
| `m_last_window_width/height/scale` | `u32/u32/float` | Last known dimensions (dedup) |
| `m_resize_debounce_timer` | `QTimer*` | 100ms debounce for resize |
| `m_pending_window_width/height/scale` | `u32/u32/float` | Pending dimensions for debounce |
| `m_container` | `QWidget*` | Container widget reference |
