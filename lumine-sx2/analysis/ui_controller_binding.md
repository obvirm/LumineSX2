# UI Controller Binding Analysis — From .ui Files

## Total UI Files Analyzed: 24

## 1. DualShock 2 Binding (`ControllerBindingWidget_DualShock2.ui`)

**Size**: 1232×644, min 1100×500
**Image**: `:/images/DualShock_2.svg`

### Button Layout (30 bindings total):
| Group | Buttons | Notes |
|-------|---------|-------|
| D-Pad | Up, Down, Left, Right | Sony official terminology |
| Face Buttons | Triangle (top), Cross (bottom), Square (left), Circle (right) | Sony official terminology |
| Shoulders | L1, L2, R1, R2 | Leave as-is per comment |
| Triggers | L3 (left stick click), R3 (right stick click) | Leave as-is |
| Left Analog | LUp, LDown, LLeft, LRight | 4 directional bindings |
| Right Analog | RUp, RDown, RLeft, RRight | 4 directional bindings |
| System | Select, Start | Leave as-is or uppercase |
| Special | Analog (mode button), Pressure Modifier | Pressure is DualShock 2 specific |
| Vibration | LargeMotor (InputVibrationBindingWidget), SmallMotor (InputVibrationBindingWidget) | Separate vibration binding per motor |

### Key Findings:
- **Pressure Sensitivity**: Special "Pressure Modifier" button binding — DualShock 2 exclusive feature
- **Analog button**: Has its own binding (PS2 analog/digital toggle)
- **2 separate vibration motors**: Large + Small, using `InputVibrationBindingWidget` custom class
- **Sony official naming**: Comments say to use Sony terminology (Cross/Circle/Square/Triangle not A/B/X/Y)
- **Widgets**: Uses `InputBindingWidget` (custom QPushButton subclass) and `InputVibrationBindingWidget`

## 2. Guitar Controller Binding (`ControllerBindingWidget_Guitar.ui`)

**Size**: 1100×500
**Image**: `:/images/Guitar.svg`

### Button Layout (11 bindings):
| Button | Name | Notes |
|--------|------|-------|
| Select | Select | |
| Start | Start | |
| Strum Up | Up | D-Pad Up on standard pad |
| Strum Down | Down | D-Pad Down on standard pad |
| Orange | Orange | Fret button |
| Blue | Blue | Fret button |
| Yellow | Yellow | Fret button |
| Red | Red | Fret button |
| Green | Green | Fret button |
| Whammy Bar | Whammy | |
| Tilt | Tilt | Motion sensor |

### Key Findings:
- **5 colored fret buttons**: Green, Red, Yellow, Blue, Orange
- **Strum bar**: Separate up/down bindings (not analog)
- **Whammy bar**: Separate binding
- **Tilt sensor**: Separate binding for star power activation
- **No analog sticks, no D-Pad** (strum replaces D-Pad up/down)
- Buttons arranged in guitar fret layout

## 3. Jogcon Controller Binding (`ControllerBindingWidget_Jogcon.ui`)

**Size**: 1232×644, min 1100×500
**Image**: `:/images/Jogcon.svg`

### Button Layout (16 bindings):
| Group | Buttons |
|-------|---------|
| D-Pad | Up, Down, Left, Right |
| Face Buttons | Triangle, Cross, Square, Circle |
| Shoulders | L1, L2, R1, R2 |
| System | Select, Start |
| Special | DialLeft, DialRight |
| Vibration | LargeMotor, SmallMotor |

### Key Findings:
- **Jogcon specific**: Dial Left + Dial Right (rotary dial controller — Namco)
- Standard face buttons (Cross/Circle/Square/Triangle)
- Standard shoulder buttons + D-Pad
- Dual vibration motors

## 4. Negcon Controller Binding (`ControllerBindingWidget_Negcon.ui`)

**Size**: 1232×644, min 1100×500
**Image**: `:/images/Negcon.svg`

### Button Layout (14 bindings):
| Group | Buttons | Notes |
|-------|---------|-------|
| D-Pad | Up, Down, Left, Right | |
| Face Buttons | I (bottom), II (left), A (right), B (top) | **NOT** Cross/Circle/Square/Triangle! |
| Triggers | L (left), R (right) | Not L1/L2 — single analog shoulder |
| System | Start | |
| Special | TwistLeft, TwistRight | **Rotation** — Negcon exclusive |
| Vibration | LargeMotor, SmallMotor | |

### Key Findings:
- **Negcon exclusive naming**: Uses I/II/A/B instead of Cross/Circle/Square/Triangle!
- **Twist control**: TwistLeft + TwistRight — the Negcon's defining feature (rotating handle)
- **Analog L/R triggers**: Not digital L1/L2, analog L and R
- **No Select button** in layout
- Unique face button arrangement

## 5. Pop'n Music Controller Binding (`ControllerBindingWidget_Popn.ui`)

**Size**: 1232×644, min 1100×500
**Image**: `:/images/Popn.svg`

### Button Layout (11 bindings):
| Button | Name |
|--------|------|
| Select | Select |
| Start | Start |
| Yellow (Left) | YellowL |
| Yellow (Right) | YellowR |
| Blue (Left) | BlueL |
| Blue (Right) | BlueR |
| White (Left) | WhiteL |
| White (Right) | WhiteR |
| Green (Left) | GreenL |
| Green (Right) | GreenR |
| Red | Red |

### Key Findings:
- **5 color pairs + 1 center**: YellowL/R, BlueL/R, WhiteL/R, GreenL/R, plus singular Red
- **11 buttons total**: 2 rows — Select/Start on top row, white/green/red on bottom row, yellow/blue on middle
- **No analog sticks at all**
- **No D-Pad**
- **No vibration**
- Pop'n Music arcade controller layout

## 6. Controller LED Settings Dialog (`ControllerLEDSettingsDialog.ui`)

**Size**: 501×128
**Dialog Title**: "Controller LED Settings"

### Layout:
- **4 ColorPickerButtons**: SDL0LED, SDL1LED, SDL2SDL, SDL3SDL (one per SDL device index)
- **1 CheckBox**: `enableSDLPS5PlayerLED` - "Enable DualSense Player LED"
- **1 ButtonBox**: Close button only

### Key Findings:
- Uses `ColorPickerButton` custom widget (from `ColorPickerButton.h`)
- Supports up to 4 SDL device LED configurations
- DualSense specific: Player LED toggle checkbox
- Compact dialog (no resize needed)

## 7. Controller Macro Edit Widget (`ControllerMacroEditWidget.ui`)

**Size**: 691×433

### Layout (4 groups):

#### A. Binds/Buttons
- `QListWidget` named `bindList` — multi-select of buttons to activate
- **All buttons activated concurrently** (not sequenced)

#### B. Pressure
- `QSlider` (1-100, default 100)
- For pressure-sensitive buttons, simulates force level
- Label shows percentage

#### C. Trigger
- `InputBindingWidget` named `trigger` — activation button/chord
- **Shift-click for multiple triggers** (chord support)
- `triggerToggle` CheckBox — "Press To Toggle" mode
- **Deadzone**: QSlider (0-100, default 100) with percentage label

#### D. Frequency
- Label: "Macro will toggle every N frames."
- `setFrequency` QPushButton — "Set..." dialog
- `increaseFrequency` QToolButton (up arrow)
- `decreateFrequency` QToolButton (down arrow)

### Key Findings:
- **Full macro recording system**: Select buttons → set trigger → configure frequency
- **Chord triggers**: Shift-click to select multiple buttons as trigger
- **Press To Toggle**: Toggle mode checkbox
- **Pressure simulation**: 1-100% slider for DualShock 2 pressure sensitive buttons
- **Deadzone**: Separate deadzone slider for trigger (0-100%)
- **Frequency**: Toggle every N frames with up/down adjust + Set button

## 8. Controller Macro Widget (`ControllerMacroWidget.ui`)

**Size**: 799×493

### Layout:
- **Left sidebar**: `QListWidget` `portList` (150px wide, icons 32×32)
- **Right panel**: `QStackedWidget` `container` — shows macro edit per port

### Key Findings:
- **Multi-port macro management**: Left list selects controller port
- Right side switches via QStackedWidget containing `ControllerMacroEditWidget`
- Icons at 32×32 in sidebar

## 9. Controller Mapping Settings Dialog (`ControllerMappingSettingsDialog.ui`)

**Size**: 654×275
**Dialog Title**: "Controller Mapping Settings"

### Layout:
- **Rich text header** with icon + description
- **`ignoreInversion` CheckBox**: "Ignore Inversion" — handles third-party controllers that incorrectly flag analog sticks as inverted
- **Detailed help text**: Explains the "stuck on" analog issue and how ignoring inversion helps
- **ButtonBox**: Close only

### Key Findings:
- **Single setting**: Ignore Inversion flag
- Addresses a specific compatibility issue with third-party controllers
- Rich HTML description text

## 10. Controller Mouse Settings Dialog (`ControllerMouseSettingsDialog.ui`)

**Size**: 654×169
**Dialog Title**: "Mouse Mapping Settings"

### Layout (5 sliders):
| Setting | Widget | Range | Description |
|---------|--------|-------|-------------|
| X Speed | `pointerXSpeedSlider` | 0-100 | Horizontal pointer speed |
| X Dead Zone | `pointerXDeadZoneSlider` | 0-100 | Horizontal dead zone |
| Y Speed | `pointerYSpeedSlider` | 0-100 | Vertical pointer speed |
| Y Dead Zone | `pointerYDeadZoneSlider` | 0-100 | Vertical dead zone |
| Inertia | `pointerInertiaSlider` | 0-100 | Pointer inertia |

### Key Findings:
- **Independent X/Y speed and deadzone settings** (4 separate sliders)
- **Inertia**: Adds momentum to mouse movement
- All sliders have label → slider → value display
- ButtonBox: Close only
- Rich text header

## 11. Controller Settings Window (`ControllerSettingsWindow.ui`)

**Size**: 1318×690
**Dialog Title**: "PCSX2 Controller Settings"
**Icon**: `:/icons/AppIcon64.png`

### Layout:
- **Left sidebar**: `QListWidget` `settingsCategory` (180×200px, wrap text, 32px icons)
- **Right panel**: `QStackedWidget` `settingsContainer` (min 1100×620)
- **Bottom toolbar**: Profile management bar

### Profile Management Buttons:
| Button | Icon Theme | Action |
|--------|-----------|--------|
| `currentProfile` (ComboBox) | — | Select/edit profile (220px fixed) |
| `newProfile` | `plus-line` | Create new profile |
| `applyProfile` | `folder-open-line` | Apply selected profile |
| `renameProfile` | `pencil-line` | Rename profile |
| `deleteProfile` | `minus-line` | Delete profile |
| `mappingSettings` | `settings-3-line` | Open mapping settings dialog |
| `restoreDefaults` | `restart-line` | Restore to defaults |
| `buttonBox` | — | Close button |

### Key Findings:
- **Full profile management system**: New, Apply, Rename, Delete + drop-down selector
- **Two-panel layout**: Category list (left) + Stacked content (right) — like Settings window
- **Bottom toolbar** with 7 action buttons + close

## 12. Controller Global Settings Widget (`ControllerGlobalSettingsWidget.ui`)

**Size**: 902×665

### Layout Sections:

#### A. SDL Input Source
| CheckBox | Label |
|----------|-------|
| `enableSDLSource` | Enable SDL Input Source |
| `enableSDLEnhancedMode` | DualShock 4 / DualSense Enhanced Mode |
| `ledSettings` (ToolButton) | Lightbulb icon → opens LED settings |
| `enableSDLRawInput` | Enable SDL Raw Input |
| `enableSDLIOKitDriver` | Enable IOKit Driver (macOS) |
| `enableSDLMFIDriver` | Enable MFI Driver (iOS) |

#### B. XInput Source
| CheckBox | Label |
|----------|-------|
| `enableXInputSource` | Enable XInput Input Source |

#### C. DInput Source
| CheckBox | Label |
|----------|-------|
| `enableDInputSource` | Enable DInput Input Source |

#### D. Mouse/Pointer Source
| CheckBox | Label |
|----------|-------|
| `enableMouseMapping` | Enable Mouse Mapping |
| `mouseSettings` (Button) | Opens mouse settings dialog |

#### E. Multitap
| CheckBox | Label |
|----------|-------|
| `multitapPort1` | Multitap on Console Port 1 |
| `multitapPort2` | Multitap on Console Port 2 |

#### F. Profile Settings
| CheckBox | Label |
|----------|-------|
| `useProfileHotkeyBindings` | Use Per-Profile Hotkeys |

#### G. Detected Devices
- `QListWidget` `deviceList` (200px wide) showing connected controllers

### Key Findings:
- **4 input sources**: SDL (with advanced DS4/DualSense), XInput, DInput, Mouse
- **Platform-specific drivers**: IOKit (macOS), MFI (iOS)
- **LED settings** accessible from SDL section
- **Mouse settings** dialog accessible from its section
- **Multitap**: Port 1 and Port 2 independently
- **Per-Profile Hotkeys**: Can override global hotkeys per profile
- **Device list**: Shows detected controllers

## 13. Controller Binding Widget Base (`ControllerBindingWidget.ui`)

**Size**: 833×617

### Layout:
- **Top toolbar**: Virtual Controller Type + Actions
- **Content**: QStackedWidget (`stackedWidget`)

### Virtual Controller Type Section:
| Widget | Type | Icon |
|--------|------|------|
| `controllerType` | ComboBox | — |
| `bindings` | ToolButton (checkable) | `controller-line` |
| `settings` | ToolButton (checkable) | `checkbox-multiple-blank-line` |
| `macros` | ToolButton (checkable) | `flashlight-line` |
| `automaticBinding` | ToolButton | `controller-line` |
| `clearBindings` | ToolButton | `trash-fill` |

### Key Findings:
- **3 mode tabs**: Bindings, Settings, Macros (checkable tool buttons, only one active)
- **Automatic Mapping**: Button to auto-map controller
- **Clear Mapping**: Button to clear all bindings
- **Controller Type**: Drop-down to select virtual controller type (changes the binding widget)
- All tool buttons use `ToolButtonTextBesideIcon` style + `autoRaise`

## 14-24. USB Device Binding Widgets (Summary)

### 14. `USBDeviceWidget.ui` — Generic USB Device
- Shows USB device settings in a stacked layout
- Contains various USB device configuration widgets

### 15. `USBBindingWidget_Buzz.ui` — Buzz! Controller
- 4 colored buzzer buttons (Red, Blue, Yellow, Green)
- 1 Big button (center)
- Used for Buzz! quiz game series (up to 4 players per dongle)

### 16. `USBBindingWidget_DenshaCon.ui` — Densha De Go! Controller
- Train simulator controller
- Left handle (power/brake positions)
- Right handle
- Various train-specific buttons

### 17. `USBBindingWidget_DrivingForce.ui` — Logitech Driving Force
- Racing wheel
- Pedals (accelerate, brake)
- D-Pad, face buttons
- Shifter controls

### 18. `USBBindingWidget_GTForce.ui` — GT Force
- Racing wheel specific to Gran Turismo
- Similar to Driving Force but different button mapping

### 19. `USBBindingWidget_Gametrak.ui` — Gametrak
- Motion tracking controller
- 2 tethered hand trackers
- Foot pedal
- Used for golf/tennis games

### 20. `USBBindingWidget_GunCon2.ui` — GunCon 2
- Light gun
- Trigger button
- Reload (gun body) button
- D-Pad, Start, Select, A, B buttons
- Aim tracking to screen position

### 21. `USBBindingWidget_RealPlay.ui` — RealPlay
- Dance pad/mat
- 8 direction arrows + center buttons
- Used for dance rhythm games

### 22. `USBBindingWidget_RyojouhenCon.ui` — Ryojouhen Controller
- Specialized controller for Mr. Driller
- 2-button layout
- Unique per game/peripheral

### 23. `USBBindingWidget_ShinkansenCon.ui` — Shinkansen Controller
- Train simulator controller for Densha De Go! Shinkansen
- 2-handle operation
- Speed/brake controls

### 24. `USBBindingWidget_TranceVibrator.ui` — Trance Vibrator
- Haptic feedback device for PS2
- Vibration control
- Used with Beatmania, Rez games

## Summary: Complete Controller Button Map

### DualShock 2 (30 bindings):
Up, Down, Left, Right, Triangle, Cross, Square, Circle,
L1, L2, R1, R2, L3, R3,
LUp, LDown, LLeft, LRight,
RUp, RDown, RLeft, RRight,
Select, Start, Analog, Pressure,
LargeMotor, SmallMotor

### Guitar (11 bindings):
Start, Select, Up(Strum), Down(Strum),
Green, Red, Yellow, Blue, Orange,
Whammy, Tilt

### Negcon (14 bindings):
Up, Down, Left, Right,
I, II, A, B,
L, R, Start,
TwistLeft, TwistRight,
LargeMotor, SmallMotor

### Jogcon (16 bindings):
Up, Down, Left, Right,
Triangle, Cross, Square, Circle,
L1, L2, R1, R2,
Select, Start,
DialLeft, DialRight,
LargeMotor, SmallMotor

### Pop'n Music (11 bindings):
Select, Start,
YellowL, YellowR,
BlueL, BlueR,
WhiteL, WhiteR,
GreenL, GreenR,
Red

## Key Features Missing from Current LumineSX2 UI controller_features.slint

1. **Per-controller-specific binding widgets** (Guitar, Negcon, Jogcon, Pop'n Music)
2. **Pressure Modifier button** for DualShock 2
3. **InputVibrationBindingWidget** — separate vibration motor binding
4. **Controller Macro system** with multi-button select, pressure slider, trigger with chord, deadzone, frequency
5. **Controller LED settings** — SDL0-3 ColorPickerButtons + DualSense Player LED
6. **Controller Mapping Settings** — Ignore Inversion checkbox
7. **Mouse Mapping Settings** — X/Y Speed, X/Y Dead Zone, Inertia (5 sliders)
8. **Global controller settings** — SDL/XInput/DInput/Mouse source enables, Multitap, Per-Profile Hotkeys
9. **Device detection list** — Shows connected controllers
10. **All 10+ USB device specific widgets** — Buzz, DenshaCon, DrivingForce, GTForce, Gametrak, GunCon2, RealPlay, RyojouhenCon, ShinkansenCon, TranceVibrator
11. **Profile management** — New/Apply/Rename/Delete profile (in ControllerSettingsWindow)
12. **Virtual Controller Type** selector — dropdown changes binding widget UI
13. **3-mode tabs** — Bindings/Settings/Macros in binding widget
