# Controller Binding Widget — Deep Analysis

## Source Files
- `ControllerBindingWidget.h` / `.cpp` — Main controller port binding widget
- `ControllerBindingWidget.ui` — Main layout (StackedWidget + toolbar)
- `ControllerBindingWidget_DualShock2.ui` — DualShock 2 button layout
- `ControllerBindingWidget_Guitar.ui` — Guitar layout
- `ControllerBindingWidget_Jogcon.ui` — Jogcon layout
- `ControllerBindingWidget_Negcon.ui` — Negcon layout
- `ControllerBindingWidget_Popn.ui` — Pop'n Music layout
- `ControllerMacroWidget.ui` — Macro list + container
- `ControllerMacroEditWidget.ui` — Single macro editor

---

## 1. ControllerBindingWidget (Main)

### Header Toolbar Layout
```
┌─────────────────────────────────────────────────────────────────┐
│ Virtual Controller Type: [ComboBox ▼]  [Bindings] [Settings] [Macros] ... [Auto Map] [Clear Map] │
├─────────────────────────────────────────────────────────────────┤
│ StackedWidget (swaps between: Bindings / Settings / Macros)     │
└─────────────────────────────────────────────────────────────────┘
```

### UI Elements
| Element | Widget | Name | Purpose |
|---------|--------|------|---------|
| Controller Type | `QComboBox` | `controllerType` | Select controller type (DualShock2, Guitar, Jogcon, Negcon, Pop'n, NotConnected, etc.) |
| Bindings Tab | `QToolButton` | `bindings` | Switch to bindings view (checkable toggle) |
| Settings Tab | `QToolButton` | `settings` | Switch to controller-specific settings (checkable toggle, disabled if no settings) |
| Macros Tab | `QToolButton` | `macros` | Switch to macro editor (checkable toggle, disabled if no bindings) |
| Automatic Mapping | `QToolButton` | `automaticBinding` | Show device auto-mapping menu |
| Clear Mapping | `QToolButton` | `clearBindings` | Clear all bindings with confirmation |

### Config Key Structure
- Section: `Pad{port+1}` (e.g. `Pad1`, `Pad2`)
- Port number: 0-indexed (`m_port_number = port`)
- Multitap support: `Pad.MultitapPort1` / `Pad.MultitapPort2`
  - When multitap enabled: "Controller Port 1A", "Controller Port 1B", etc. (A/B/C/D slots)
  - When disabled: "Controller Port 1", "Controller Port 2"

### Signals/Callbacks
| Signal | Handler | Action |
|--------|---------|--------|
| `controllerType.currentIndexChanged` | `onTypeChanged()` | Rebuild stacked widget for new controller type |
| `bindings.clicked` | `onBindingsClicked()` | Show bindings page |
| `settings.clicked` | `onSettingsClicked()` | Show settings page |
| `macros.clicked` | `onMacrosClicked()` | Show macros page |
| `automaticBinding.clicked` | `onAutomaticBindingClicked()` | Show device menu |
| `clearBindings.clicked` | `onClearBindingsClicked()` | Clear with confirmation |

### Controller Types (via `Pad::GetControllerTypeNames()`)
- `NotConnected`
- `DualShock2` (default for PS2)
- `Guitar`
- `Jogcon`
- `Negcon`
- `Popn`
- And others from PadTypes.h

### Stacked Widget Pages
1. **Bindings** — `ControllerBindingWidget_Base` subclass (DualShock2, Guitar, Jogcon, Negcon, Popn, or generic)
2. **Settings** — `ControllerCustomSettingsWidget` (controller-specific settings from `cinfo->settings`)
3. **Macros** — `ControllerMacroWidget` (up to `NUM_MACRO_BUTTONS_PER_CONTROLLER` macros)

---

## 2. Automatic Mapping

### Flow
1. User clicks "Automatic Mapping" button
2. `QMenu` popup shows list of detected devices from `m_dialog->getDeviceList()`
3. Menu format: `"{device_name}: {display_name}"` or just `"{device_name}"` if same
4. On device selection: `doDeviceAutomaticBinding(device_name)`
5. Calls `InputManager::GetGenericBindingMapping(device)` → returns vector of `<GenericInputBinding, string>` pairs
6. Calls `Pad::MapController(settings, port, mapping)` to write bindings
7. Commits settings + calls `g_emu_thread->applySettings()`
8. Refreshes UI via `onTypeChanged()`

### Error Handling
- Empty mapping → `QMessageBox::critical` with "No generic bindings were generated for device"
- No devices → Menu shows disabled "No devices available" item

---

## 3. Clear Bindings

### Flow
1. Confirmation dialog: "Are you sure you want to clear all bindings for this controller? This action cannot be undone."
2. If global settings: Uses `Host::GetSettingsLock()` → `Pad::ClearPortBindings()` → `Host::CommitBaseSettingChanges()`
3. If profile settings: `Pad::ClearPortBindings(profileSettings)` → `Save()` → `reloadInputBindings()`
4. Refreshes UI

---

## 4. DualShock 2 Binding Layout

### 3-Column Layout
```
┌─────────────┐  ┌──────────────┐  ┌─────────────┐
│ D-Pad       │  │ L2 [R2]     │  │ Face Buttons│
│  Up         │  │ L1 [R1]     │  │  Triangle   │
│  Left Right │  │ [Start]     │  │  Square  ○  │
│  Down       │  │ [Select]    │  │  Cross      │
├─────────────┤  ├──────────────┤  ├─────────────┤
│ Left Analog │  │ [DualShock  │  │ Right Analog│
│  LUp        │  │  2 Image]   │  │  RUp        │
│  LLeft LRight│  │             │  │  RLeft RRight│
│  LDown      │  │             │  │  RDown      │
├─────────────┤  ├──────────────┤  ├─────────────┤
│ Large Motor │  │ L3 [Pressure│  │ Small Motor │
│ [vibration] │  │   Modifier] │  │ [vibration] │
│             │  │ [Analog]    │  │             │
└─────────────┘  └──────────────┘  └─────────────┘
```

### Complete DualShock 2 Bindings (28 total)
| Widget Name | Display Name | Type | Section |
|-------------|-------------|------|---------|
| **D-Pad** ||||
| `Up` | Up | Button | D-Pad |
| `Down` | Down | Button | D-Pad |
| `Left` | Left | Button | D-Pad |
| `Right` | Right | Button | D-Pad |
| **Left Analog** ||||
| `LUp` | Up | HalfAxis/Axis | Left Analog |
| `LDown` | Down | HalfAxis/Axis | Left Analog |
| `LLeft` | Left | HalfAxis/Axis | Left Analog |
| `LRight` | Right | HalfAxis/Axis | Left Analog |
| **Right Analog** ||||
| `RUp` | Up | HalfAxis/Axis | Right Analog |
| `RDown` | Down | HalfAxis/Axis | Right Analog |
| `RLeft` | Left | HalfAxis/Axis | Right Analog |
| `RRight` | Right | HalfAxis/Axis | Right Analog |
| **Shoulder/Triggers** ||||
| `L1` | L1 | Button | Shoulder |
| `L2` | L2 | Button (pressure) | Shoulder |
| `R1` | R1 | Button | Shoulder |
| `R2` | R2 | Button (pressure) | Shoulder |
| **Face Buttons** ||||
| `Triangle` | Triangle | Button (pressure) | Face |
| `Circle` | Circle | Button (pressure) | Face |
| `Cross` | Cross | Button (pressure) | Face |
| `Square` | Square | Button (pressure) | Face |
| **Special** ||||
| `Start` | Start | Button | Special |
| `Select` | Select | Button | Special |
| `L3` | L3 | Button | Special (stick click) |
| `R3` | R3 | Button | Special (stick click) |
| `Pressure` | Pressure Modifier | Axis | Special |
| `Analog` | Analog | Button | Special (toggle analog mode) |
| **Vibration** ||||
| `LargeMotor` | Large Motor | Vibration | Vibration |
| `SmallMotor` | Small Motor | Vibration | Vibration |

### Pressure Sensitivity
- All face buttons (Triangle, Circle, Cross, Square) and L2/R2 support pressure-sensitive input
- `Pressure` modifier binding exists for reduced pressure mode
- Widget type: `InputBindingWidget` (button/halfaxis/axis binding)
- Vibration type: `InputVibrationBindingWidget` (motor binding)

---

## 5. Other Controller Binding Layouts

### Guitar Controller (11 bindings)
| Widget | Display | Type |
|--------|---------|------|
| `Green` | Green | Button |
| `Red` | Red | Button |
| `Yellow` | Yellow | Button |
| `Blue` | Blue | Button |
| `Orange` | Orange | Button |
| `Up` | Up | Button |
| `Down` | Down | Button |
| `Select` | Select | Button |
| `Start` | Start | Button |
| `Whammy` | Whammy | Axis |
| `Tilt` | Tilt | Axis |

### Jogcon Controller (18 bindings)
| Widget | Display | Type |
|--------|---------|------|
| Standard PS2 face buttons (Triangle, Circle, Cross, Square) | Face | Button |
| D-Pad (Up, Down, Left, Right) | D-Pad | Button |
| Shoulder (L1, L2, R1, R2) | Shoulder | Button |
| `Start` / `Select` | Special | Button |
| `DialLeft` | Dial Left | Axis |
| `DialRight` | Dial Right | Axis |
| `LargeMotor` / `SmallMotor` | Vibration | Vibration |

### Negcon Controller (15 bindings)
| Widget | Display | Type |
|--------|---------|------|
| D-Pad (Up, Down, Left, Right) | D-Pad | Button |
| `Start` | Start | Button |
| `A` | A | Button |
| `B` | B | Button |
| `I` | I | Button (pressure) |
| `II` | II | Button (pressure) |
| `L` | L | Button (pressure) |
| `R` | R | Button (pressure) |
| `TwistLeft` | Twist Left | Axis |
| `TwistRight` | Twist Right | Axis |
| `LargeMotor` / `SmallMotor` | Vibration | Vibration |

### Pop'n Music Controller (11 bindings)
| Widget | Display | Type |
|--------|---------|------|
| `WhiteL` | White Left | Button |
| `GreenL` | Green Left | Button |
| `Red` | Red | Button |
| `BlueL` | Blue Left | Button |
| `YellowL` | Yellow Left | Button |
| `YellowR` | Yellow Right | Button |
| `BlueR` | Blue Right | Button |
| `GreenR` | Green Right | Button |
| `WhiteR` | White Right | Button |
| `Select` / `Start` | Special | Button |

---

## 6. Controller Macro System

### Macro Widget (ControllerMacroWidget)
- **16 macros per controller** (`Pad::NUM_MACRO_BUTTONS_PER_CONTROLLER`)
- List on left (`QListWidget`), stacked editors on right (`QStackedWidget`)
- Each list item shows: "Macro {N}\n{Summary}" (e.g. "Macro 1\nTriangle/Cross")
- Icon: `flashlight-line`

### Macro Editor (ControllerMacroEditWidget)
```
┌─ Binds/Buttons ──────────────────────────┐
│ Select buttons to trigger with this macro │
│ [☑ Triangle] [☐ Cross] [☐ Square] ...    │
├─ Pressure ───────────────────────────────┤
│ [═══════════════════●══] 100%            │
├─ Trigger ────────────────────────────────┤
│ Select trigger (single button or chord)   │
│ [InputBindingWidget for trigger]          │
│ [☐ Press To Toggle]  Deadzone: [══●═] 50%│
├─ Frequency ──────────────────────────────┤
│ "Macro will not repeat."                  │
│ [Set...] [↑] [↓]                         │
└──────────────────────────────────────────┘
```

### Macro Properties
| Property | Config Key | Widget | Range |
|----------|-----------|--------|-------|
| Bindings | `Macro{N}Binds` | `QListWidget` (checkable) | All controller buttons (excl. Motor) |
| Pressure | `Macro{N}Pressure` | `QSlider` | 1–100% (normalized to 0.0–1.0) |
| Trigger | `Macro{N}` | `InputBindingWidget` | Any input (button/chord) |
| Toggle Mode | `Macro{N}Toggle` | `QCheckBox` | bool |
| Deadzone | `Macro{N}Deadzone` | `QSlider` | 0–100% |
| Frequency | `Macro{N}Frequency` | `QPushButton` + arrows | 0 = no repeat, N = toggle every N frames |

### Macro Bind Storage Format
- Stored as single string joined by `&`: `"Triangle & Cross"`
- Loaded by splitting on `&`, matching against `cinfo->bindings`

### Frequency Controls
- `setFrequency` button → `QInputDialog::getInt` (min: 0, max: INT_MAX)
- `increaseFrequency` → increment by 1
- `decreateFrequency` → decrement by 1 (min: 0)
- Display: "Macro will not repeat." (if 0) or "Macro will toggle buttons every N frames."

---

## 7. Custom Settings Widget (ControllerCustomSettingsWidget)

### Supported Setting Types
| SettingInfo::Type | Qt Widget | Config Binding |
|-------------------|-----------|----------------|
| `Boolean` | `QCheckBox` | `BindWidgetToInputProfileBool` |
| `Integer` | `QSpinBox` (min/max/step/format) | `BindWidgetToInputProfileInt` |
| `IntegerList` | `QComboBox` | `BindWidgetToInputProfileInt` |
| `Float` | `QDoubleSpinBox` (with multiplier) | `BindWidgetToInputProfileFloat` |
| `String` | `QLineEdit` | `BindWidgetToInputProfileString` |
| `StringList` | `QComboBox` (with `get_options` callback or static options) | `BindWidgetToInputProfileString` |
| `Path` | `QLineEdit` + `QPushButton` ("Browse...") → `QFileDialog::getOpenFileName` | `BindWidgetToInputProfileString` |

### Layout
- All settings in a `QGridLayout` inside a `QScrollArea`
- Each setting has: label + widget + description label
- Bottom: "Restore Default Settings" button (with `restart-line` icon)
- Restores all settings to their `*DefaultValue()` from `SettingInfo`

### Float Format Parsing
- Extracts prefix/suffix from format strings like `"%.2f%%"` → prefix=`""`, suffix=`"%"`, decimals=2
- `multiplier` applied to min/max/step/default values

---

## 8. USB Device Widget (USBDeviceWidget)

### Header Toolbar
```
┌─────────────────────────────────────────────────────────────────┐
│ USB Port 1: [Device Type ▼] [Subtype ▼]                        │
│ [Bindings] [Settings]            [Automatic Mapping] [Clear]    │
├─────────────────────────────────────────────────────────────────┤
│ StackedWidget (Bindings / Settings)                             │
└─────────────────────────────────────────────────────────────────┘
```

### Config Key Structure
- Section: `USB{port+1}` (e.g. `USB1`, `USB2`)
- Subtype: `{device_type}_subtype` (e.g. `Pad_subtype`)

### Supported USB Devices (with icons)
| Type | Display Name | Icon |
|------|-------------|------|
| `Pad` | Wheel Device | `wheel-line` |
| `Msd` | Mass Storage | `msd-line` |
| `singstar` | Singstar | `singstar-line` |
| `logitech_usbmic` | Logitech USB Mic | `mic-line` |
| `headset` | Logitech Headset | `headset-line` |
| `hidkbd` | HID Keyboard | `keyboard-2-line` |
| `hidmouse` | HID Mouse | `mouse-line` |
| `RBDrumKit` | Rock Band Drums | `drum-line` |
| `BuzzDevice` | Buzz Controller | `buzz-controller-line` |
| `TranceVibrator` | Trance Vibrator | `trance-vibrator-line` |
| `webcam` | EyeToy | `eyetoy-line` |
| `beatmania` | BeatMania | `keyboard-2-line` |
| `seamic` | SEGA Seamic | `seamic-line` |
| `printer` | Printer | `printer-line` |
| `Keyboardmania` | KeyboardMania | `keyboardmania-line` |
| `guncon2` | GunCon 2 | `guncon2-line` |
| `DJTurntable` | DJ Hero | `dj-hero-line` |
| `Gametrak` | Gametrak | `gametrak-line` |
| `RealPlay` | RealPlay | `realplay-sphere-line` |
| `TrainController` | Train Controller | `train-line` |

### USB Binding Widget Templates
| Device | Subtype | Template UI |
|--------|---------|-------------|
| `Pad` | 0 (Driving Force) | `USBBindingWidget_DrivingForce` |
| `Pad` | 3 (GT Force) | `USBBindingWidget_GTForce` |
| `BuzzDevice` | any | `USBBindingWidget_Buzz` |
| `TrainController` | 0 | `USBBindingWidget_DenshaCon` |
| `TrainController` | 1 | `USBBindingWidget_ShinkansenCon` |
| `TrainController` | 2 | `USBBindingWidget_RyojouhenCon` |
| `Gametrak` | any | `USBBindingWidget_Gametrak` |
| `guncon2` | any | `USBBindingWidget_GunCon2` |
| `RealPlay` | any | `USBBindingWidget_RealPlay` |
| `TranceVibrator` | any | `USBBindingWidget_TranceVibrator` |
| (others) | — | Dynamic `createWidgets()` (Axes + Buttons groups) |

### USB Dynamic Binding Layout
- **Axes group**: 2-column grid, each axis in its own `QGroupBox` with `InputBindingWidget`
- **Buttons group**: 2 or 4-column grid (4 if no axes), each button in `QGroupBox`
- Supports: `Axis`, `HalfAxis`, `Button`, `Pointer`, `Device`, `Motor` binding types

---

## 9. ControllerBindingWidget_Base (Generic)

### `initBindingWidgets()` Logic
1. Gets `Pad::ControllerInfo` for current controller type
2. Iterates `cinfo->bindings`:
   - `Axis` / `HalfAxis` / `Button` / `Pointer` / `Device` → finds `InputBindingWidget` by object name, calls `widget->initialize(sif, bind_type, config_section, binding_name)`
3. Handles vibration capabilities:
   - `LargeSmallMotors` → finds `LargeMotor` + `SmallMotor` widgets
   - `SingleMotor` → finds `Motor` widget
   - `NoVibration` → skip

### Icons per Controller Type
| Type | Icon |
|------|------|
| DualShock2 | `controller-line` |
| Guitar | `guitar-line` |
| Jogcon | `jogcon-line` |
| Negcon | `negcon-line` |
| Popn | `Popn-line` |
| Generic | `controller-strike-line` |

---

## 10. Hidden/Non-Obvious Features

### 1. Multitap Support
- When `MultitapPort1`/`MultitapPort2` enabled in Pad settings, ports become 1A-1D, 2A-2D
- Each slot gets its own controller binding widget

### 2. Per-Profile Settings
- Supports both global settings (`Host::GetSettingsLock()`) and per-game profile settings
- `isEditingGlobalSettings()` determines lock/save strategy
- Per-game settings auto-save via `QtHost::SaveGameSettings()`

### 3. Macro Toggle Mode
- `Macro{N}Toggle` — "Press To Toggle" checkbox
- When enabled, pressing trigger toggles macro on/off instead of hold-to-activate

### 4. Macro Frequency (Turbo)
- Frame-based frequency: "Macro will toggle buttons every N frames"
- 0 = no repeat (single press)
- Set via dialog, up/down arrows, or direct input

### 5. Pressure-Sensitive Macros
- Macros can simulate reduced pressure (1–100%)
- Affects all pressure-sensitive buttons (face buttons, L2, R2)

### 6. Macro Deadzone
- Deadzone slider (0–100%) controls activation threshold
- Affects analog trigger input for macro activation

### 7. DualShock 2 Pressure Modifier
- `Pressure` is a separate axis binding
- When held, reduces force on all pressure-sensitive buttons
- Combined with macro pressure for fine control

### 8. Analog Toggle
- `Analog` button toggles between digital and analog mode
- Physical button on original DualShock 2 controller

### 9. Controller-Specific Settings (ControllerCustomSettingsWidget)
- Dynamic settings from `cinfo->settings` span
- Supports 7 setting types: Boolean, Integer, IntegerList, Float, String, StringList, Path
- Path type includes "Browse..." file dialog
- "Restore Default Settings" button at bottom

### 10. USB Subtype System
- Some USB devices have subtypes (e.g. Pad → Driving Force / GT Force)
- Subtype changes trigger full page rebuild
- Stored as `{device_type}_subtype` in config

### 11. Context-Aware Header Buttons
- `automaticBinding` and `clearBindings` only enabled when on Bindings tab
- `settings` disabled when controller has no custom settings
- `macros` disabled when controller has no bindings

### 12. Device List Menu
- Auto-mapping shows menu of all detected input devices
- Format: `"{device_name}: {device_display_name}"` when names differ
- Each device generates a complete binding mapping via `InputManager::GetGenericBindingMapping()`

### 13. InputBindingWidget Behavior
- Each binding button shows current binding text
- Clicking starts binding listen mode
- Supports multi-binding via shift-click (for triggers/chords)
- Widget types: `InputBindingWidget` (standard), `InputVibrationBindingWidget` (vibration motor)

---

## 11. Config Key Summary

### Per-Port Config (`Pad{N}`)
```
[Pad1]
Type = DualShock2
Up = keyboard/Up
Down = keyboard/Down
...
LargeMotor = SDL-0/+LeftMotor
SmallMotor = SDL-0/+RightMotor
Pressure = SDL-0/+LeftTrigger
Analog = keyboard/Space

Macro1Binds = Triangle & Cross
Macro1Pressure = 80
Macro1 = SDL-0/+X
Macro1Toggle = false
Macro1Deadzone = 0
Macro1Frequency = 0
```

### Per-USB Config (`USB{N}`)
```
[USB1]
Type = guncon2
guncon2_subtype = 0
guncon2_trigger = SDL-0/+Button0
```
