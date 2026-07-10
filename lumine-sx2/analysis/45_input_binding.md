# Analysis 45 — Input Binding System (InputBindingDialog + InputBindingWidget)

## Files Analyzed
- `pcsx2-qt/Settings/InputBindingDialog.h` / `.cpp`
- `pcsx2-qt/Settings/InputBindingWidget.h` / `.cpp`

---

## Architecture Overview

Two complementary classes for controller input binding:

| Class | Role | Base Class | Usage |
|-------|------|------------|-------|
| **InputBindingWidget** | Single-line binding button (QPushButton) | QPushButton | Inline in controller config pages |
| **InputBindingDialog** | Multi-binding dialog (QDialog) | QDialog | Opened from widget via Shift+Click or multi-bind click |
| **InputVibrationBindingWidget** | Vibration motor selector | QPushButton | Vibration motor assignment |

---

## InputBindingWidget — Core Binding Button

### Constructor & Initialization
```cpp
InputBindingWidget(QWidget* parent, SettingsInterface* sif,
    InputBindingInfo::Type bind_type, std::string section_name, std::string key_name)
```
- **Fixed size**: 225px min/max width
- **Signals connected**:
  - `clicked → onClicked()`
  - `g_emu_thread->onInputDeviceConnected → onInputDeviceConnected()`
  - `g_emu_thread->onInputDeviceDisconnected → onInputDeviceDisconnected()`

### State
```cpp
SettingsInterface* m_sif;                    // null = base settings, non-null = per-game
InputBindingInfo::Type m_bind_type;          // Button, Axis, HalfAxis, Pointer, etc.
std::string m_section_name;                  // e.g., "Pad1"
std::string m_key_name;                      // e.g., "Cross", "LeftStickX"
std::vector<std::string> m_bindings_settings; // raw binding strings
std::vector<std::string> m_bindings_ui;       // prettified display strings
std::vector<InputBindingKey> m_new_bindings;  // keys being captured
std::vector<std::pair<InputBindingKey, std::pair<float, float>>> m_value_ranges;
QTimer* m_input_listen_timer;                 // countdown timer
u32 m_input_listen_remaining_seconds;
QPoint m_input_listen_start_position;         // for mouse move threshold
bool m_mouse_mapping_enabled;
```

### Behaviors

#### `onClicked()` — Left Click Handler
- **If >1 binding exists**: Opens the multi-binding dialog (`openDialog()`)
- **If ≤1 binding**: Starts listening for input (5-second timeout)

#### `mouseReleaseEvent()` — Right Click Handler
- **Right click**: Clears binding entirely (`clearBinding()`)

#### `event()` — Shift+Click
- **Shift + Left click**: Opens dialog regardless of binding count

#### Text Display (`updateText()`)
- **0 bindings**: Empty text, tooltip "No bindings registered"
- **1 binding**: Shows binding text (truncated to 35 chars + "...")
- **>1 bindings**: Shows "N bindings" count
- Tooltip always shows all bindings + help text
- Ampersands (`&`) are escaped for Qt display

#### Tooltips
```
Line 1: Binding text (or "No bindings registered")
Line 2: "Left click to assign a new button"
Line 3: "Shift + left click for additional bindings"
Line 4: "Right click to clear binding" (only when bindings exist)
```

### Input Listening System

#### `startListeningForInput(u32 timeout_in_seconds)`
1. Clears `m_new_bindings` and `m_value_ranges`
2. Checks mouse mapping enabled
3. Records start cursor position
4. Creates 1-second repeating timer
5. Updates text: "Push Button/Axis... [N]"
6. `grabKeyboard()` + `grabMouse()` + `setMouseTracking(true)`
7. `installEventFilter(this)` — captures ALL keyboard/mouse events
8. `hookInputManager()` — intercepts ALL input devices

#### `stopListeningForInput()`
1. Calls `reloadBinding()` (refreshes from settings)
2. Deletes timer
3. Clears `m_new_bindings`
4. Releases keyboard/mouse hooks
5. Removes event filter

#### Timeout Countdown (`onInputListenTimerTimeout`)
- Decrements counter each second
- Updates button text: "Push Button/Axis... [N]"
- At 0: stops listening (cancels)

### Event Filter — Capturing Input

Handles these events during listening:

| Event | Action |
|-------|--------|
| **KeyRelease / MouseButtonRelease** | Commits binding, stops listening |
| **KeyPress** | Adds host keyboard key to `m_new_bindings` |
| **MouseButtonPress / DblClick** | Adds pointer button to `m_new_bindings` |
| **Wheel** | Adds WheelX/WheelY axis (with Negate modifier for negative) |
| **MouseMove** (if mouse mapping on) | Adds X/Y pointer axis if moved ≥50px from start |

### inputManagerHookCallback — Gamepad/Controller Input

This is the core input detection for gamepads, called via `InputManager::SetHook()`:

#### Detection Logic
1. **Value tracking**: Tracks `initial_value`, `min_value` per key in `m_value_ranges`
2. **Reverse threshold**: If initial axis value > 0.5, it's a pedal (resting position is pressed)
3. **For existing keys in m_new_bindings**:
   - If value returns near center (abs < 0.5): commit binding, stop
   - For pedals: if (initial - current) ≤ 0.25: commit
   - If pedal went full range (initial > 0.5, min ≤ -0.5): sets `FullAxis` modifier
4. **For new keys**:
   - If abs value ≥ 0.5 (or < 0.5 for reverse): adds to `m_new_bindings`
   - Sets `Negate` modifier if value < 0
   - Sets `invert` flag if reverse threshold

#### Modifiers Applied
- `InputModifier::None` — default positive direction
- `InputModifier::Negate` — negative axis direction
- `InputModifier::FullAxis` — pedal went full range

### Settings Persistence

#### `setNewBinding()`
- Converts `m_new_bindings` to string via `InputManager::ConvertInputBindingKeysToString()`
- **If m_sif (per-game)**: `m_sif->SetStringValue()` → `m_sif->Save()` → `g_emu_thread->reloadGameSettings()`
- **If base**: `Host::SetBaseStringSettingValue()` → `Host::CommitBaseSettingChanges()` → `g_emu_thread->reloadInputBindings()`
- **Replaces** existing binding (single binding mode)

#### `clearBinding()`
- **If m_sif**: `m_sif->DeleteValue()` → save → reloadGameSettings
- **If base**: `Host::RemoveBaseSettingValue()` → commit → reloadInputBindings
- Calls `reloadBinding()` to refresh UI

#### `reloadBinding()`
- Reads `GetStringList` from settings
- Prettifies each binding via `InputManager::PrettifyInputBinding()`
- Stores both raw (settings) and prettified (UI) versions
- Calls `updateText()`

### Device Hot-plug
- `onInputDeviceConnected()`: Calls `reloadBinding()` (refreshes display names)
- `onInputDeviceDisconnected()`: Calls `reloadBinding()` (refreshes display names)

---

## InputBindingDialog — Multi-Binding Dialog

### UI Elements (from .ui file)
- `title` — QLabel showing "Bindings for [section] [key]"
- `bindingList` — QListWidget showing all bindings
- `addBinding` — QPushButton "Add"
- `removeBinding` — QPushButton "Remove"
- `clearBindings` — QPushButton "Clear All"
- `buttonBox` — QDialogButtonBox with "Close"
- `status` — QLabel for countdown "Push Button/Axis... [N]"
- `sensitivityWidget` — container for sensitivity/deadzone (only shown for Button/Axis/HalfAxis)
- `sensitivity` — QSlider (0-100, bound to `{key}Scale`)
- `sensitivityValue` — QLabel showing "N%"
- `deadzone` — QSlider (0-100, bound to `{key}Deadzone`)
- `deadzoneValue` — QLabel showing "N%"

### Constructor
```cpp
InputBindingDialog(SettingsInterface* sif, InputBindingInfo::Type bind_type,
    std::string section_name, std::string key_name,
    std::vector<std::string> bindings_settings,
    std::vector<std::string> bindings_ui, QWidget* parent)
```

### Sensitivity & Deadzone (only for Button/Axis/HalfAxis)
- `sensitivity`: Bound to `{key}Scale` (normalized 100.0, default 1.0)
- `deadzone`: Bound to `{key}Deadzone` (normalized 100.0, default 0.0)
- Display: `"%N%"` format
- **Hidden for non-button/axis types**: Widget is deleted from layout

### Multi-Binding Operations

#### `onAddBindingButtonClicked()`
- If listening: stops current listen
- Starts listening with 5-second timeout

#### `onRemoveBindingButtonClicked()`
- Removes selected row from `m_bindings_settings` and `m_bindings_ui`
- Deletes list item
- Saves to settings

#### `onClearBindingsButtonClicked()`
- Clears all bindings
- Clears list widget
- Saves to settings

### Input Listening (same as Widget but for Dialog)
- Same event filter logic (keyboard, mouse, wheel, mouse move)
- Same inputManagerHookCallback (gamepad detection with pedal support)
- Adds to list instead of replacing
- **Duplicate detection**: `addNewBinding()` checks if binding already exists before adding

### Settings Persistence

#### `saveListToSettings()`
- **If m_sif (per-game)**:
  - Non-empty: `SetStringList` → `Save` → `reloadGameSettings`
  - Empty: `DeleteValue` → `Save` → `reloadGameSettings`
- **If base**:
  - Non-empty: `Host::SetBaseStringListSettingValue` → `CommitBaseSettingChanges` → `reloadInputBindings`
  - Empty: `Host::RemoveBaseSettingValue` → `CommitBaseSettingChanges` → `reloadInputBindings`

### Binding Display Names (`ReloadBindNames`)
- Called on device connect/disconnect
- Re-prettifies all binding strings
- Updates list widget display

---

## InputVibrationBindingWidget — Vibration Motor Selector

### Constructor
```cpp
InputVibrationBindingWidget(QWidget* parent, ControllerSettingsWindow* dialog,
    std::string section_name, std::string key_name)
```
- **Fixed size**: 225px min/max width
- Reads current binding from `Host::GetBaseStringSettingValue()`
- Prettifies for display

### `onClicked()` — Opens QInputDialog
1. Gets available vibration motors from `m_dialog->getVibrationMotors()`
2. Prettifies motor names for display
3. If current binding not in list: appends it
4. If no motors detected: shows error QMessageBox
5. Opens `QInputDialog` with combo box (non-editable)
6. On OK: saves selected motor to settings

### `clearBinding()` — Right Click
- Removes setting value
- Commits changes
- Reloads input bindings
- Clears text

### `mouseReleaseEvent()`
- Right click: clears binding
- Other: normal QPushButton behavior

---

## Hidden/Non-Obvious Features

### 1. Pedal (Reverse Threshold) Detection
- If a controller axis starts at >0.5 (resting position is pressed), it's treated as a pedal
- Pedals require moving from resting → fully pressed → back to resting
- If pedal goes full range (initial > 0.5, min ≤ -0.5): `FullAxis` modifier is set
- Threshold for "near resting": (initial - current) ≤ 0.25

### 2. Multi-Binding Support
- Widget: single binding (replaces on each capture)
- Dialog: multiple bindings (adds to list)
- Widget opens dialog when >1 binding exists
- Shift+Click always opens dialog

### 3. Mouse Mapping Toggle
- Controlled by `UI/EnableMouseMapping` setting
- When disabled: Mouse movement events are ignored during binding
- When enabled: Mouse X/Y axes are bindable (50px threshold)

### 4. Device Hot-plug Rebinding
- Both widget and dialog listen for `onInputDeviceConnected` / `onInputDeviceDisconnected`
- Automatically refreshes binding display names when devices change
- Handles cases where device names change (e.g., driver update)

### 5. Duplicate Prevention
- Dialog: Checks if binding string already exists before adding
- Widget: No duplicate check (single binding mode)

### 6. Per-Game vs Base Settings
- `m_sif` null = base settings (global)
- `m_sif` non-null = per-game settings
- Different save paths:
  - Base: `Host::SetBase*SettingValue` → `CommitBaseSettingChanges` → `reloadInputBindings`
  - Per-game: `m_sif->SetStringValue` → `m_sif->Save` → `reloadGameSettings`

### 7. Binding String Format
- Raw: `InputManager::ConvertInputBindingKeysToString()` — machine-readable
- UI: `InputManager::PrettifyInputBinding()` — human-readable
- Stored in settings: raw format
- Displayed in UI: prettified format

### 8. Input Hook System
- `InputManager::SetHook()` — global hook that intercepts ALL input
- Returns `InputInterceptHook::CallbackResult::StopProcessingEvent` — prevents input from reaching emulator
- Used during binding capture to prevent input from being processed as gameplay

### 9. Value Range Tracking
- Tracks `initial_value` and `min_value` per key in `m_value_ranges`
- Used to detect pedal behavior and full-axis movement
- Reset when starting new listen

### 10. Text Ellipsis
- Single binding: truncated to 35 characters + "..."
- Ampersands escaped (`&` → `&&`)
- Multi-binding: shows count "N bindings"

---

## UI Elements Summary

### InputBindingWidget (QPushButton)
| Element | Description |
|---------|-------------|
| Text | Binding name or "N bindings" or empty |
| Tooltip | Full binding list + help text |
| Left click | Bind single or open dialog |
| Shift+click | Always open dialog |
| Right click | Clear binding |

### InputBindingDialog (QDialog)
| Element | Description |
|---------|-------------|
| title | "Bindings for [section] [key]" |
| bindingList | QListWidget of all bindings |
| addBinding | "Add" button |
| removeBinding | "Remove" button |
| clearBindings | "Clear All" button |
| buttonBox | "Close" button |
| status | "Push Button/Axis... [N]" |
| sensitivity | QSlider (Button/Axis/HalfAxis only) |
| sensitivityValue | "N%" label |
| deadzone | QSlider (Button/Axis/HalfAxis only) |
| deadzoneValue | "N%" label |

### InputVibrationBindingWidget (QPushButton)
| Element | Description |
|---------|-------------|
| Text | Motor name or empty |
| Left click | Open motor selector dialog |
| Right click | Clear binding |

---

## Signals & Callbacks

| Signal/Callback | Source | Handler |
|-----------------|--------|---------|
| `clicked` | QPushButton | `onClicked()` |
| `QTimer::timeout` | Timer | `onInputListenTimerTimeout()` |
| `onInputDeviceConnected` | EmuThread | `onInputDeviceConnected()` |
| `onInputDeviceDisconnected` | EmuThread | `onInputDeviceDisconnected()` |
| `InputManager hook` | InputManager | `inputManagerHookCallback()` |
| `sensitivity::valueChanged` | QSlider | `onSensitivityChanged()` |
| `deadzone::valueChanged` | QSlider | `onDeadzoneChanged()` |

---

## Slint Implementation Requirements

For the Slint UI, the input binding system needs:

1. **Binding Button Component** — Shows current binding, click to listen, right-click to clear
2. **Binding Dialog Component** — Multi-binding list with add/remove/clear
3. **Input Detection** — Keyboard, mouse, gamepad capture during listen mode
4. **Countdown Timer** — "Push Button/Axis... [N]" display
5. **Sensitivity/Deadzone Sliders** — Per-binding adjustment (Button/Axis/HalfAxis only)
6. **Vibration Motor Selector** — Dropdown for motor assignment
7. **Device Hot-plug** — Refresh bindings when devices connect/disconnect
8. **Per-Game Settings** — Support for both base and per-game binding storage

### Critical Implementation Notes
- Must hook into `InputManager` for gamepad detection
- Mouse mapping is conditional on `UI/EnableMouseMapping` setting
- Pedal detection requires tracking initial/min values over time
- Duplicate prevention in multi-bind mode
- Text ellipsis at 35 characters
- Ampersand escaping for Qt display
- Different save paths for base vs per-game settings
