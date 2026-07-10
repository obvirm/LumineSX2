# 27 — Shortcut Creation Dialog

**Files**: `ShortcutCreationDialog.h`, `ShortcutCreationDialog.cpp`

## Overview
Dialog for creating desktop/start menu shortcuts to launch games with specific options. Windows uses `.lnk` via COM `IShellLink`, Linux uses `.desktop` files.

## Constructor Parameters
- `title` — Game title (displayed in window title)
- `path` — Game file path

## UI Elements (from setupUi)

### Shortcut Destination
| Element | Type | Notes |
|---------|------|-------|
| `shortcutDesktop` | QCheckBox | Create on Desktop |
| `shortcutStartMenu` | QCheckBox | Create in Start Menu (Windows) / Application Launcher (Linux) |
| `portableModeToggle` | QCheckBox | Add `-portable` arg. Disabled in Flatpak/container |
| `dialogButtons` | QDialogButtonBox | OK/Cancel |

### Boot Options
| Element | Type | Notes |
|---------|------|-------|
| `bootOptionToggle` | QCheckBox | Enable boot option |
| `bootOptionDropdown` | QPushButton (0=FastBoot, 1=SlowBoot) | Maps to `-fastboot` / `-slowboot` |

### ELF Override
| Element | Type | Notes |
|---------|------|-------|
| `overrideBootELFToggle` | QCheckBox | Enable ELF override |
| `overrideBootELFPath` | QLineEdit | Path to ELF file |
| `overrideBootELFButton` | QPushButton | Browse for `.elf` file. Maps to `-elf <path>` |

### Game Arguments
| Element | Type | Notes |
|---------|------|-------|
| `gameArgsToggle` | QCheckBox | Enable game args |
| `gameArgs` | QLineEdit | Custom game args. Maps to `-gameargs <args>` |

### Save State Loading
| Element | Type | Notes |
|---------|------|-------|
| `loadStateIndexToggle` | QCheckBox | Load state by slot index |
| `loadStateIndex` | QSpinBox | 1 to NUM_SAVE_STATE_SLOTS. Maps to `-state <n>` |
| `loadStateFileToggle` | QCheckBox | Load state from file |
| `loadStateFilePath` | QLineEdit | Path to `.p2s` file |
| `loadStateFileBrowse` | QPushButton | Browse for `.p2s` file. Maps to `-statefile <path>` |

### Display Options
| Element | Type | Notes |
|---------|------|-------|
| `fullscreenMode` | QCheckBox | Enable fullscreen option |
| `fullscreenModeDropdown` | QPushButton (0=Fullscreen, 1=NoFullscreen) | Maps to `-fullscreen` / `-nofullscreen` |
| `bigPictureModeToggle` | QCheckBox | Maps to `-bigpicture` |

### Fast Forward Options
| Element | Type | Notes |
|---------|------|-------|
| `fastForwardOptionToggle` | QCheckBox | Enable FF option |
| `fastForwardTurboOption` | QRadioButton | Maps to `-turbo` |
| `fastForwardUnlimitedOption` | QRadioButton | Maps to `-unlimited` |

**Radio group**: Turbo and Unlimited are mutually exclusive. Default: Turbo checked.

### Custom Arguments
| Element | Type | Notes |
|---------|------|-------|
| `customArgsInput` | QLineEdit | Free-form custom CLI args appended to shortcut |

### Icon Customization
| Element | Type | Notes |
|---------|------|-------|
| `iconPreview` | QLabel | Shows icon preview pixmap |
| `iconPath` | QLineEdit | Path to custom icon |
| `browseIconButton` | QPushButton | Browse for icon. Windows: `.ico`. Linux: `.png .jpg .svg .webp` |
| `resetIconButton` | QPushButton | Reset to default PCSX2 app icon |

## Conditional Enable/Disable Logic
All option groups follow pattern: checkbox toggles enable state of child widgets.

## CLI Arguments Generated
| Option | Argument |
|--------|----------|
| Portable mode | `-portable` |
| ELF override | `-elf <path>` |
| Game args | `-gameargs <text>` |
| Fast boot | `-fastboot` |
| Slow boot | `-slowboot` |
| State slot | `-state <n>` |
| State file | `-statefile <path>` |
| Fullscreen | `-fullscreen` |
| No fullscreen | `-nofullscreen` |
| Big picture | `-bigpicture` |
| Turbo | `-turbo` |
| Unlimited | `-unlimited` |

Final arg format: `{combined_cli_args} {custom_args} -- {game_path}`

## Platform: Windows `.lnk`
- Uses `IShellLink` COM + `IPersistFile` for `.lnk` creation
- Desktop: `SHGetKnownFolderPath(FOLDERID_Desktop)`
- Start Menu: `SHGetKnownFolderPath(FOLDERID_Programs)` → creates `PCSX2` subfolder
- Icon: Custom or fallback to `resources/icons/AppIconLarge.ico`
- Checks for duplicate shortcut name

## Platform: Linux `.desktop`
- Creates `.desktop` file with `[Desktop Entry]` format
- Desktop: `$XDG_DESKTOP_DIR` or `~/Desktop/`
- Application Launcher: `$XDG_DATA_HOME/applications/` or `~/.local/share/applications/`
- Flatpak: Sets `executable_path = "flatpak run net.pcsx2.PCSX2"`, icon = `net.pcsx2.PCSX2`
- Non-flatpak: Copies icon to `~/.local/share/icons/hicolor/512x512/apps/`
- Sets file permissions to `S_IRWXU` (700)
- Prompts save destination via `QFileDialog::getSaveFileName`

## Validation
- Name must be non-empty
- Filename sanitized via `Path::SanitizeFileName`
- Invalid filename chars checked via `Path::IsValidFileName`
- Icon file existence verified
- CLI arg escaping checked for losslessness

## EscapeShortcutCommandLine
- **Windows**: Wraps in double quotes, escapes `\` and `"`
- **Linux**: Wraps in double quotes, escapes `"`, `` ` ``, `\`, `$`, `%` (double-escape for `\` and `$`). Returns `lossless=false` if space chars present

## Container/Flatpak Detection
- `std::getenv("container")` → disables portable mode and shortcut creation checkboxes
- Linux: `is_flatpak` boolean derived from same env var

## Hidden Features for Slint UI
1. **Big Picture mode shortcut** — `-bigpicture` arg
2. **ELF override** — boot custom ELF instead of game
3. **Game arguments** — pass custom args to PS2 game
4. **State file loading** — load specific `.p2s` file (not just slot index)
5. **Portable mode flag** — `-portable` in shortcut
6. **No-fullscreen option** — not just fullscreen, also explicit windowed mode
7. **Custom icon** — per-game icon with preview
8. **CLI arg escaping** — platform-aware with lossless check
9. **Start menu subfolder** — `PCSX2` subfolder in Start Menu Programs
10. **Duplicate shortcut check** — prevents overwriting existing `.lnk`
