# Game Cheat Settings - Deep Analysis

## Source Files
- `pcsx2-qt/Settings/GameCheatSettingsWidget.h`
- `pcsx2-qt/Settings/GameCheatSettingsWidget.cpp`
- `pcsx2-qt/Settings/GameCheatSettingsWidget.ui`

## UI Layout (from .ui)

```
┌──────────────────────────────────────────────────────────┐
│ ⚠ Warning label (word wrap)                              │
├──────────────────────────────────────────────────────────┤
│ [✓ Enable Cheats]  [Search...                         ]  │
├──────────────────────────────────────────────────────────┤
│ TreeView: cheatList                                      │
│  ┌────────────────┬──────────┬─────────────────────────┐ │
│  │ Name           │ Author   │ Description             │ │
│  ├────────────────┼──────────┼─────────────────────────┤ │
│  │ □ GroupName    │          │                         │ │
│  │   □ Cheat1     │ Author1  │ Description text...     │ │
│  │   □ Cheat2     │ Author2  │ Description text...     │ │
│  │ □ Ungrouped    │ Author3  │ Description text...     │ │
│  └────────────────┴──────────┴─────────────────────────┘ │
├──────────────────────────────────────────────────────────┤
│ [Enable All] [Disable All] [✓ All CRCs]  Applied: ...  [Reload Cheats] │
└──────────────────────────────────────────────────────────┘
```

Widget default size: 700×600

## Columns (3-column QTreeView)

| Column | Content | Width |
|--------|---------|-------|
| 0 | Name (with checkbox) | 320px fixed |
| 1 | Author | 100px fixed |
| 2 | Description | fill remaining (-1) |

Column widths are dynamically set via `QtUtils::ResizeColumnsForTreeView` on resize.

## Data Model

### PatchInfo struct
```cpp
struct PatchInfo {
    std::string name;           // Full hierarchical name: "Group\CheatName"
    std::string description;
    std::string author;
    std::optional<patch_place_type> place;  // Only if all lines in group have same place
};
```

### Name Parsing
- `GetNameParentPart()` → returns part before last `\` (group/folder name)
- `GetNamePart()` → returns part after last `\` (cheat display name)
- Hierarchical: names use `\` as separator → tree structure
- Example: `"GameSpeed\FastRun"` → parent="GameSpeed", name="FastRun"

### Place Types (patch_place_type)
| Value | Meaning |
|-------|---------|
| 0 | PPT_ONCE_ON_LOAD - Applied once when game loads |
| 1 | PPT_CONTINUOUSLY - Applied every frame |
| 2 | PPT_COMBINED_0_1 - Both on-load and continuous |
| 3 | PPT_ON_LOAD_OR_WHEN_ENABLED - Applied on load or when toggled on |

### Custom Roles
| Role | Value | Purpose |
|------|-------|---------|
| NAME_ROLE | Qt::UserRole | Stores full cheat name string |
| PLACE_ROLE | Qt::UserRole + 1 | Stores patch_place_type as int |

### Config Storage
- Section: `Patch::CHEATS_CONFIG_SECTION` (string list)
- Key: `Patch::PATCH_ENABLE_CONFIG_KEY`
- Storage: StringList of enabled cheat names
- Methods: `AddToStringList()`, `RemoveFromStringList()`, `GetStringList()`

## Features

### 1. Enable Cheats Toggle
- `QCheckBox` bound to `"EmuCore"/"EnableCheats"` setting (default: false)
- When unchecked: entire cheat list, buttons, search, allCRCs are disabled
- Signal: `checkStateChanged` → `updateListEnabled()`

### 2. Search/Filter
- `QLineEdit` with placeholder "Search..."
- `QSortFilterProxyModel` with:
  - `setRecursiveFilteringEnabled(true)` — shows parent when child matches
  - `setAutoAcceptChildRows(true)` — keeps tree structure during filter
  - `setFilterCaseSensitivity(Qt::CaseInsensitive)`
- Filter type: `setFilterFixedString(text)` (substring match)
- On text change: auto `expandAll()` to show filtered results

### 3. Cheat List (QTreeView + QStandardItemModel)
- Three columns: Name, Author, Description
- Each cheat row:
  - Checkable (Qt::ItemIsUserCheckable)
  - Never has children (Qt::ItemNeverHasChildren)
  - Enabled (Qt::ItemIsEnabled)
  - No edit triggers, no selection
- Tree structure from `\`-separated names
- `rootIsDecorated` hidden when no groups exist (saves whitespace)
- Double-click on non-0 column of parent: toggle expand/collapse
- Double-click on leaf item: toggle check state

### 4. Enable All / Disable All
- **Hidden feature**: Disconnects `itemChanged` signal during batch operation
  - Prevents N individual saves → single save at end
  - Reconnects signal after batch
- `setStateRecursively()`:
  - Walks entire tree (null parent = root level)
  - For items with NAME_ROLE data: sets check state + calls setCheatEnabled
  - For items without NAME_ROLE (groups): recurses into children
- After batch: single `Save()` + `reloadGameSettings()`

### 5. All CRCs Checkbox
- `QCheckBox` bound to `"EmuCore"/"ShowCheatsForAllCRCs"` setting
- When toggled: triggers `reloadList()` to re-fetch patches
- Disabled when no game serial is loaded
- Widget help text:
  - Title: "Show Cheats For All CRCs"
  - Description: "Toggles scanning patch files for all CRCs of the game. With this enabled available patches for the game serial with different CRCs will also be loaded."

### 6. Reload Cheats Button
- Calls `reloadList()` to rebuild tree
- Also calls `g_emu_thread->reloadPatches()` to reload on emu thread
- Ensures both UI and emulation are in sync

### 7. Hover Info (Applied label)
- Event filter on tree viewport: `MouseMove` + `Leave`
- On hover over checkable item:
  - Reads PLACE_ROLE from item
  - Shows "Applied: [place string]" in `appliedLabel`
  - Uses `Patch::PlaceToString()` to convert
- On leave or hover over non-checkable: clears label

### 8. Unlabelled Codes Notice
- `Patch::GetPatchInfo()` returns count via `num_unlabelled_codes` out-param
- If > 0: adds informational row: "N unlabelled patch codes will automatically activate."
- This row is NOT checkable (plain text)

### 9. Disable All Cheats (standalone method)
- `disableAllCheats()`:
  - Clears entire `CHEATS_CONFIG_SECTION`
  - Saves immediately
  - Called externally (likely from parent settings window)

## Signals & Slots Summary

| Signal | Slot | Behavior |
|--------|------|----------|
| enableCheats.checkStateChanged | updateListEnabled | Enable/disable all UI elements |
| cheatList.doubleClicked | onCheatListItemDoubleClicked | Toggle cheat or expand/collapse group |
| model.itemChanged | onCheatListItemChanged | Sync checkbox state to config |
| reloadCheats.clicked | onReloadClicked | Rebuild list + reload emu patches |
| enableAll.clicked | setStateForAll(true) | Check all cheats |
| disableAll.clicked | setStateForAll(false) | Uncheck all cheats |
| allCRCsCheckbox.checkStateChanged | onReloadClicked | Rebuild list with different CRC filter |
| searchText.textChanged | proxy.setFilterFixedString | Filter tree + expand all |
| dialog.discSerialChanged | reloadList | Rebuild list for new game |
| viewport.MouseMove | onCheatListItemHovered | Show "Applied:" label |
| viewport.Leave | onCheatListItemHovered(QModelIndex()) | Clear "Applied:" label |

## Warning Text (from .ui)
```
Activating cheats can cause unpredictable behavior, crashing, soft-locks,
or broken saved games. Use cheats at your own risk, the PCSX2 team will
provide no support for users who have enabled cheats.
```

## Tab Order (focus chain)
1. enableCheats
2. searchText
3. cheatList
4. enableAll
5. disableAll
6. allCRCsCheckbox
7. reloadCheats

## Performance Optimization
- `setStateForAll()`: Disconnects `itemChanged` during batch to prevent N×Save
- Proxy model: recursive filtering + auto child rows for tree-aware filtering
- Column resize on window resize (responsive layout)

## Hidden/Advanced Features
1. **Batch toggle disconnect pattern** — prevents cascading saves
2. **Hierarchical tree from flat names** — `\` separator → tree nodes
3. **Root decoration auto-hide** — no groups = no expand arrow
4. **Multi-CRC scanning** — loads cheats for different CRCs of same serial
5. **Applied place tooltip** — shows when patch applies (on load vs continuous)
6. **Unlabelled code counter** — auto-activating codes without names
7. **Description tooltip** — full description on hover over description column
