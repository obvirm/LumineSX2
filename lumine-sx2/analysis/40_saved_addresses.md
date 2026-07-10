# SavedAddressesView — Deep Analysis

## Files Analyzed
- `pcsx2-qt/Debugger/Memory/SavedAddressesView.h`
- `pcsx2-qt/Debugger/Memory/SavedAddressesView.cpp`
- `pcsx2-qt/Debugger/Memory/SavedAddressesModel.h`
- `pcsx2-qt/Debugger/Memory/SavedAddressesModel.cpp`
- `pcsx2-qt/Debugger/Memory/SavedAddressesView.ui`

## Overview
Debugger view for bookmarking memory addresses with labels and descriptions. Persists via `DebuggerSettingsManager`. Singleton per CPU type (R5900 vs R3000A).

## Data Model (SavedAddressesModel)

### Struct: SavedAddress
```cpp
struct SavedAddress {
    u32 address;      // hex address
    QString label;    // short label
    QString description; // free-text description
};
```

### Columns (enum HeaderColumns)
| Index | Name          | Header Text        | ResizeMode       |
|-------|---------------|--------------------|------------------|
| 0     | ADDRESS       | "MEMORY ADDRESS"   | ResizeToContents |
| 1     | LABEL         | "LABEL"            | ResizeToContents |
| 2     | DESCRIPTION   | "DESCRIPTION"      | Stretch          |

### Model Features
- **Singleton per CPU**: `getInstance(DebugInterface&)` — one model per `BreakPointCpu` (R5900/R3000A)
- **Editable**: All cells are `Qt::ItemIsEditable`
- **Address parsing**: Hex input parsed via `toUInt(&ok, 16)`, rejects invalid hex
- **UserRole role**: Returns raw `u32` address (for programmatic access vs display hex)

## View Features (SavedAddressesView)

### UI Layout
- Single `QTableView` (`savedAddressesList`) with no margins/spacing
- Column headers are **movable** (`setSectionsMovable(true)`)
- Auto-resize columns on data change

### Context Menu Actions (7 items)

| Action              | Condition              | Description                                      |
|---------------------|------------------------|--------------------------------------------------|
| **New**             | Always                 | Adds empty row, starts editing ADDRESS column    |
| **Go To Address**   | Row selected           | Fires `DebuggerEvents::GoToAddress` (disasm view) |
| **Copy Address/Text** | Row selected         | Copies cell value to clipboard (address or text) |
| **Copy all as CSV** | Has rows               | Exports entire table to clipboard as CSV         |
| **Paste from CSV**  | Always                 | Imports CSV from clipboard                       |
| **Load from Settings** | CPU alive           | Clears model, reloads from `DebuggerSettingsManager` |
| **Save to Settings** | CPU alive             | Saves model to `DebuggerSettingsManager`         |
| **Delete**          | Row selected           | Removes selected row                             |

### CSV Paste Format
- Expects header line (skipped automatically)
- Each line: quoted values separated by commas
- Regex: `"([^"]|\\.)*"` — handles escaped quotes
- Column count must match exactly 3 (ADDRESS, LABEL, DESCRIPTION)
- Address must be valid hex

### Event Handlers

1. **`DebuggerEvents::AddToSavedAddresses`**
   - Received from other debugger views (e.g., MemoryView)
   - Adds address to model
   - Optionally switches to this tab (`event.switch_to_tab`)

2. **`EmuThread::onGameChanged`**
   - On game load, if model is empty, loads from game settings
   - Auto-populates saved addresses from per-game config

### Persistence
- Uses `DebuggerSettingsManager::loadGameSettings(model)` / `saveGameSettings(model)`
- Per-game settings (loaded/saved per game title)

## Hidden/Non-Obvious Features

1. **Singleton model per CPU type** — R5900 and R3000A have separate saved address lists
2. **Auto-load on game change** — first time game loads, saved addresses auto-populated
3. **CSV round-trip** — full export/import via clipboard for sharing addresses
4. **GoToAddress cross-view** — clicking "Go To Address" fires event that DisassemblyView receives
5. **switch_to_tab** — when adding from MemoryView, can auto-focus this tab
6. **Header columns movable** — user can reorder columns by dragging
7. **UserRole data access** — address stored as raw u32 in UserRole for programmatic use

## Integration Points
- **MemoryView**: Can fire `AddToSavedAddresses` event
- **DisassemblyView**: Receives `GoToAddress` event
- **DebuggerSettingsManager**: Persistence layer
- **EmuThread**: Game change notifications

## UI Element Count
- 1 QTableView
- 7 context menu actions
- 3 columns
- 0 toolbar buttons
- 0 keyboard shortcuts (beyond standard Qt table nav)
