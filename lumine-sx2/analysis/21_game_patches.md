# 21 - Game Patch Settings Widget Analysis

## Source Files
- `GamePatchSettingsWidget.h`
- `GamePatchSettingsWidget.cpp`

## Two Classes

### 1. `GamePatchDetailsWidget` — Single Patch Row
- **Fields displayed**: Patch name, author (defaults to "Unknown"), place (applied location), description
- **Description format**: `<strong>Author:</strong> %1<br><strong>Applied:</strong> %2<br>%3`
- **Checkbox**: Tri-state for globally-toggleable patches (WS/NI), binary for normal patches
- **Callback `onEnabledStateChanged(int state)`**:
  - `Qt::Checked` → adds to `PATCH_ENABLE_CONFIG_KEY` list, removes from disable list
  - `Qt::Unchecked` → removes from enable list; if tristate, adds to `PATCH_DISABLE_CONFIG_KEY`
  - `Qt::PartiallyChecked` (tristate only) → removes from both lists (inherits global setting)
  - Saves settings, calls `g_emu_thread->reloadGameSettings()`

### 2. `GamePatchSettingsWidget` — Main Patch List

#### UI Elements
| Element | Type | Purpose |
|---------|------|---------|
| `scrollArea` | QScrollArea | Contains all patch detail widgets |
| `allCRCsCheckbox` | QCheckBox | Show patches for all CRCs of the game |
| `reload` | QPushButton | Reload patch list + notify emu thread |
| `unlabeledPatchWarning` | QLabel | Warning when unlabeled patch groups exist |
| `globalWsPatchState` | QLabel | Note when global widescreen patches are enabled |
| `globalNiPatchState` | QLabel | Note when global no-interlace patches are enabled |

#### Setting Bound to `allCRCsCheckbox`
- **Section**: `EmuCore`
- **Key**: `ShowPatchesForAllCRCs`
- **Default**: `false`
- **Behavior**: When checked, scans patch files for ALL CRCs of the game serial, not just the current disc CRC

#### Widget Help Registration
- **Title**: "Show Patches For All CRCs"
- **Recommended**: "Checked"
- **Description**: Toggles scanning patch files for all CRCs. Enables patches for game serial with different CRCs.

#### Signals/Callbacks
| Signal Source | Slot | Behavior |
|--------------|------|----------|
| `reload` clicked | `onReloadClicked()` | Calls `reloadList()` + `g_emu_thread->reloadPatches()` |
| `allCRCsCheckbox` checkStateChanged | `reloadList()` | Refreshes patch list |
| `dialog()->discSerialChanged` | `reloadList()` | Refreshes when disc serial changes |

#### `reloadList()` Logic
1. Gets `PatchInfo` via `Patch::GetPatchInfo(serial, discCRC, false, showAllCRCS, &number_of_unlabeled)`
2. Reads `PATCH_ENABLE_CONFIG_KEY` and `PATCH_DISABLE_CONFIG_KEY` string lists
3. Checks global settings: `EnableWideScreenPatches`, `EnableNoInterlacingPatches`
4. Shows/hides warnings:
   - `unlabeledPatchWarning` visible when unlabeled patches > 0
   - `globalWsPatchState` visible when global WS patches enabled
   - `globalNiPatchState` visible when global NI patches enabled
5. Disables `allCRCsCheckbox` if serial is empty
6. For each patch:
   - Normal patches: Checked if on enable list and not on disable list; else Unchecked
   - Globally-toggleable (WS/NI): Tri-state — Unchecked if on disable list, Checked if on enable list, PartiallyChecked if on neither (inherit global)
7. Empty list shows: "There are no patches available for this game."

#### `disableAllPatches()`
- Clears entire `PATCHES_CONFIG_SECTION` from settings
- Saves settings

## Key Data Structures
- `Patch::PatchInfo` fields: `name`, `author`, `description`, `place`
- `Patch::IsGloballyToggleablePatch(info)` — determines if patch is WS/NI type
- Config section: `PATCHES_CONFIG_SECTION`
- Config keys: `PATCH_ENABLE_CONFIG_KEY`, `PATCH_DISABLE_CONFIG_KEY`

## Hidden/Notable Features
1. **Tri-state checkboxes for WS/NI patches**: PartiallyChecked = inherit global setting (neither explicitly enabled nor disabled)
2. **disableAllPatches()**: Public method, likely called externally to reset all patches
3. **discSerialChanged signal**: Live updates patch list when disc serial changes in the dialog
4. **All CRCs toggle**: Allows seeing patches from all regions/versions of the same game
5. **Unlabeled patch warning**: Alerts user to patches without proper group labels
6. **Emu thread notification**: Both reloadGameSettings() and reloadPatches() called after changes
