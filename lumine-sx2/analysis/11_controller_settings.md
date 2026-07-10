# Analysis 11: Controller Settings (ControllerSettingsWindow, Global Settings, LED, Mouse, Mapping)

## Files Analyzed
1. `ControllerSettingsWindow.h` — Main controller settings window class
2. `ControllerSettingsWindow.cpp` — Full implementation of controller settings window
3. `ControllerGlobalSettingsWidget.h` — Global controller settings widget + 3 sub-dialogs (LED, Mouse, Mapping)
4. `ControllerGlobalSettingsWidget.cpp` — Implementation of global settings, LED dialog, mouse dialog, mapping dialog

---

## ControllerSettingsWindow — Main Window Structure

### Categories (Sidebar Navigation)
| # | Category | Description |
|---|----------|-------------|
| 0 | Global Settings | SDL, XInput, DInput, multitap, mouse, device list |
| 1-4 | Controller Port 1–4 | Per-port binding widget (DualShock 2, etc.) |
| 5-8 | Multitap Ports A1-D2 | When multitap enabled, 8 sub-slots appear |
| 9-10 | USB Port 1–2 | USB device configuration |
| 11 | Hotkeys | Hotkey binding (only if global settings or profile has UseProfileHotkeyBindings) |

### Constants
- `MAX_PORTS = 8` (global slot count)
- `USB::NUM_PORTS` = 2 USB ports
- Multitap slot names: `A, B, C, D`
- Multitap port reorder: `{{0, 2, 3, 4, 1, 5, 6, 7}}` (reorders for visual clarity)

### Profile System
| Feature | Description |
|---------|-------------|
| Shared Profile | Default global profile (index 0) |
| Custom Profiles | User-created per-game input profiles |
| Create Profile | Copy bindings from current or create empty |
| Apply Profile | Overwrites global settings with profile bindings |
| Rename Profile | Rename + updates all game settings referencing it |
| Delete Profile | Deletes .ini file, switches back to Shared |
| Profile Hotkey Bindings | Toggle: `Pad → UseProfileHotkeyBindings` |
| Profile Path | `VMManager::GetInputProfilePath(name)` |

### Profile Operations Detail

#### Create Profile (`onNewProfileClicked`)
1. Dialog asks for profile name
2. Checks if profile already exists
3. Asks: copy bindings from current profile? (Yes/No/Cancel)
4. If Yes and editing global: also asks about copying hotkey bindings
5. Creates INISettingsInterface, copies Pad + USB configuration
6. Saves, refreshes list, switches to new profile

#### Apply Profile (`onApplyProfileClicked`)
1. Confirmation dialog (irreversible)
2. Copies Pad and USB configuration from profile to global settings
3. Commits changes, applies settings
4. Switches back to Shared view

#### Restore Defaults (`onRestoreDefaultsClicked`)
1. Confirmation dialog (irreversible, profiles preserved)
2. Calls `VMManager::SetDefaultSettings(..., controller=true, ...)` 
3. Only available when editing global settings

### Device Management
| Signal | Action |
|--------|--------|
| `onInputDevicesEnumerated` | Populates device list with all connected devices |
| `onInputDeviceConnected` | Adds device, re-enumerates vibration motors |
| `onInputDeviceDisconnected` | Removes device, re-enumerates vibration motors |
| `onVibrationMotorsEnumerated` | Stores vibration motor binding keys |

### Category Widget Creation (`createWidgets`)
- Clears all widgets and category list
- Creates `ControllerGlobalSettingsWidget` (Global Settings)
- Creates `ControllerBindingWidget` for each active port (respects multitap)
- Creates `USBDeviceWidget` for each USB port
- Creates `HotkeySettingsWidget` if applicable
- Updates list item text to show controller type name
- Multitap shows "Port 1A", "Port 1B", etc.

### Helper Methods for Settings Access
```
getBoolValue(section, key, default)    → profile or global
getIntValue(section, key, default)     → profile or global  
getStringValue(section, key, default)  → profile or global
setBoolValue(section, key, value)      → saves + reloads
setIntValue(section, key, value)       → saves + reloads
setStringValue(section, key, value)    → saves + reloads
clearSettingValue(section, key)        → saves + reloads
```

---

## ControllerGlobalSettingsWidget — Global Settings

### SDL Input Section
| Setting | Key | Default | Platform |
|---------|-----|---------|----------|
| Enable SDL Source | `InputSources/SDL` | true | All |
| SDL Enhanced Mode | `InputSources/SDLControllerEnhancedMode` | true | All |
| SDL Raw Input | `InputSources/SDLRawInput` | false | Windows only |
| SDL IOKit Driver | `InputSources/SDLIOKitDriver` | true | macOS only |
| SDL MFI Driver | `InputSources/SDLMFIDriver` | true | macOS only |

### Input Section
| Setting | Key | Default |
|---------|-----|---------|
| Enable Mouse Mapping | `UI/EnableMouseMapping` | false |
| Multitap Port 1 | `Pad/MultitapPort1` | false |
| Multitap Port 2 | `Pad/MultitapPort2` | false |

### Windows-Only Section
| Setting | Key | Default |
|---------|-----|---------|
| Enable XInput | `InputSources/XInput` | false |
| Enable DInput | `InputSources/DInput` | false |

### Profile-Only Setting
| Setting | Key | Default | When |
|---------|-----|---------|------|
| Use Profile Hotkey Bindings | `Pad/UseProfileHotkeyBindings` | false | Only when editing profile |

### Device List Widget
- `addDeviceToList(identifier, name)` — adds item with UserRole data
- `removeDeviceFromList(identifier)` — removes by identifier
- Display format: "identifier: name" or just "identifier" if same

### Dialogs Triggered
- **LED Settings** button → opens `ControllerLEDSettingsDialog`
- **Mouse Settings** button → opens `ControllerMouseSettingsDialog`

---

## ControllerLEDSettingsDialog — LED Settings

### Per-Player LED Colors
| Widget | Player | Setting Key | Default |
|--------|--------|-------------|---------|
| SDL0LED | Player 0 | `SDLExtra/Player0LED` | Player-specific color |
| SDL1LED | Player 1 | `SDLExtra/Player1LED` | Player-specific color |
| SDL2LED | Player 2 | `SDLExtra/Player2LED` | Player-specific color |
| SDL3LED | Player 3 | `SDLExtra/Player3LED` | Player-specific color |

- Color format: 6-digit hex RGB (`{:06X}`)
- Uses `ColorPickerButton` widget with `colorChanged` signal
- Default colors parsed via `SDLInputSource::ParseRGBForPlayerId()`

### PS5 Controller LED
| Setting | Key | Default |
|---------|-----|---------|
| Enable PS5 Player LED | `InputSources/SDLPS5PlayerLED` | true |

---

## ControllerMouseSettingsDialog — Mouse Settings

### Mouse Pointer Settings (all `Pad` section)
| Setting | Key | Default | Widget |
|---------|-----|---------|--------|
| Pointer X Speed | `Pad/PointerXSpeed` | 40.0 | Slider |
| Pointer Y Speed | `Pad/PointerYSpeed` | 40.0 | Slider |
| Pointer X Dead Zone | `Pad/PointerXDeadZone` | 20.0 | Slider |
| Pointer Y Dead Zone | `Pad/PointerYDeadZone` | 20.0 | Slider |
| Pointer Inertia | `Pad/PointerInertia` | 10.0 | Slider |

- Each slider has a value label that updates live
- Uses `BindWidgetToInputProfileFloat` for profile-aware binding

---

## ControllerMappingSettingsDialog — Mapping Settings

### Mapping Settings
| Setting | Key | Default |
|---------|-----|---------|
| Ignore Inversion | `InputSources/IgnoreInversion` | false |

- Simple dialog with single checkbox
- Uses `BindWidgetToInputProfileBool`

---

## Hidden/Advanced Features Discovered

1. **Input Profile System** — Complete per-game input profile management (create, rename, delete, apply, copy)
2. **Multitap Support** — Dual multitap (Port 1 and Port 2), enabling 8 controller slots total
3. **Platform-Specific Input** — SDL Raw Input (Windows), IOKit/MFI drivers (macOS), XInput/DInput (Windows)
4. **SDL Enhanced Mode** — `SDLControllerEnhancedMode` for better controller support
5. **PS5 Player LED Control** — Separate toggle for PS5 controller LED behavior
6. **Per-Player LED Colors** — RGB color picker for each of 4 player LEDs
7. **Mouse Pointer Settings** — Speed, dead zone, inertia for mouse-as-pointer (5 separate float sliders)
8. **Vibration Motor Enumeration** — Dynamically discovers vibration motors on connected devices
9. **Device Hot-Plug** — Real-time device connect/disconnect handling
10. **Ignore Inversion Setting** — For mapping that ignores axis inversion
11. **Profile Hotkey Bindings** — Profiles can optionally include their own hotkey bindings
12. **Restore Defaults** — Resets controller settings without touching input profiles
13. **Mapping Settings Dialog** — Separate dialog for global mapping options

## Settings Keys Summary

### InputSources Section
| Key | Type | Default |
|-----|------|---------|
| SDL | bool | true |
| SDLControllerEnhancedMode | bool | true |
| SDLRawInput | bool | false (Windows) |
| SDLIOKitDriver | bool | true (macOS) |
| SDLMFIDriver | bool | true (macOS) |
| XInput | bool | false (Windows) |
| DInput | bool | false (Windows) |
| SDLPS5PlayerLED | bool | true |
| IgnoreInversion | bool | false |

### Pad Section
| Key | Type | Default |
|-----|------|---------|
| MultitapPort1 | bool | false |
| MultitapPort2 | bool | false |
| UseProfileHotkeyBindings | bool | false |
| PointerXSpeed | float | 40.0 |
| PointerYSpeed | float | 40.0 |
| PointerXDeadZone | float | 20.0 |
| PointerYDeadZone | float | 20.0 |
| PointerInertia | float | 10.0 |

### SDLExtra Section
| Key | Type | Default |
|-----|------|---------|
| Player0LED | string (hex RGB) | Auto |
| Player1LED | string (hex RGB) | Auto |
| Player2LED | string (hex RGB) | Auto |
| Player3LED | string (hex RGB) | Auto |

### UI Section
| Key | Type | Default |
|-----|------|---------|
| EnableMouseMapping | bool | false |
