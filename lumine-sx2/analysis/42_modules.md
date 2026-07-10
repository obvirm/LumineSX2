# Analysis 42: ModuleView & ModuleModel — IOP Module List

## Source Files
- `pcsx2-qt/Debugger/ModuleView.h`
- `pcsx2-qt/Debugger/ModuleView.cpp`
- `pcsx2-qt/Debugger/ModuleModel.h`
- `pcsx2-qt/Debugger/ModuleModel.cpp`

## Overview
Debugger view that lists all loaded IOP (R3000) modules. Displays ELF sections (text/data/bss), entry points, and GP register values. Provides navigation into disassembler and memory views.

> **⚠ R3000 ONLY** — Uses `IopMod` from `BiosDebugData.h`. This is the IOP subsystem, NOT the EE (R5900). No EE module list exists in the debugger.

---

## Data Source

### `IopMod` struct (from `BiosDebugData.h`)
```cpp
struct IopMod {
    std::string name;
    u16 version;
    u32 entry;       // Entry point address
    u32 gp;          // Global Pointer register value
    u32 text_addr;   // Text section start
    u32 text_size;
    u32 data_size;
    u32 bss_size;
};
```

### Data retrieval
- `m_cpu.GetModuleList()` → `std::vector<IopMod>`
- Stored locally in `m_modules` (copy on each refresh)

---

## Table Columns (ModuleModel::ModuleColumns)

| # | Enum | Header | Display Format | UserRole (raw) | Resize Mode |
|---|------|--------|---------------|-----------------|-------------|
| 0 | NAME | "NAME" | `mod->name` | string | ResizeToContents |
| 1 | VERSION | "VERSION" | `"major.minor"` (version>>8, version&0xff) | u16 raw | Stretch |
| 2 | ENTRY | "ENTRY" | 8-digit hex (FilledQStringFromValue, base 16) | u32 addr | Stretch |
| 3 | GP | "GP" | 8-digit hex | u32 addr | Stretch |
| 4 | TEXT_SECTION | "TEXT" | `[start - end]` hex range | u32 text_addr | ResizeToContents |
| 5 | DATA_SECTION | "DATA" | `[start - end]` hex range | u32 data_addr | ResizeToContents |
| 6 | BSS_SECTION | "BSS" | `[start - end]` hex range or empty if bss_size==0 | u32 bss_addr or 0 | ResizeToContents |

### Section address calculation
- **Text**: `text_addr` to `text_addr + text_size - 1`
- **Data**: `text_addr + text_size` to `text_addr + text_size + data_size - 1`
- **BSS**: `text_addr + text_size + data_size` to `text_addr + text_size + data_size + bss_size - 1` (empty string if `bss_size == 0`)

---

## UI Elements

### ModuleView (inherits DebuggerView)
- **Table view** (`m_ui.moduleList`): `QTableView` with `QSortFilterProxyModel`
- **Monospace font**: Constructor passes `MONOSPACE_FONT`
- **Movable columns**: `horizontalHeader()->setSectionsMovable(true)`
- **Auto-fit rows**: `verticalHeader()->setSectionResizeMode(ResizeToContents)`
- **Custom context menu policy**: `Qt::CustomContextMenu`

---

## Context Menu (right-click)

### Actions
1. **Copy** — Copies the cell data at current selection index (`m_model->data(currentIndex())`)
2. **Separator**
3. **Copy all as CSV** — Exports entire table via `QtUtils::AbstractItemModelToCSV(m_ui.moduleList->model())`

### Behavior
- Menu only shown if selection exists (`hasSelection()`)
- Menu has `WA_DeleteOnClose` for auto-cleanup
- Shown at cursor position via `mapToGlobal(pos)`

---

## Double-Click Navigation

| Column Clicked | Action | Target |
|----------------|--------|--------|
| ENTRY | `goToInDisassembler(addr, true)` | Disassembly at entry point |
| GP | `goToInMemoryView(addr, true)` | Memory view at GP address |
| TEXT_SECTION | `goToInDisassembler(addr, true)` | Disassembly at text section start |
| DATA_SECTION | `goToInMemoryView(addr, true)` | Memory view at data section start |
| BSS_SECTION | `goToInMemoryView(addr, true)` | Memory view at BSS start (only if bss_size != 0) |
| NAME, VERSION | No action (falls through) |

> **Note**: `goToInDisassembler` and `goToInMemoryView` are inherited from `DebuggerView`.

---

## Auto-Refresh Events

| Event | Condition | Action |
|-------|-----------|--------|
| `DebuggerEvents::Refresh` | VM not paused | `m_model->refreshData()` |
| `DebuggerEvents::VMUpdate` | Always | `m_model->refreshData()` |

### refreshData()
```cpp
void ModuleModel::refreshData() {
    beginResetModel();
    m_modules = m_cpu.GetModuleList();  // Full copy from DebugInterface
    endResetModel();
}
```

---

## Hidden Features / Notes

1. **BSS guard**: Double-click on BSS does nothing if `bss_size == 0` (avoids navigating to garbage address)
2. **Version format**: Displayed as `"major.minor"` where major = upper byte, minor = lower byte of u16
3. **CSV export**: Full table export including all columns — useful for external analysis
4. **UserRole data**: All columns provide raw numeric address data via `Qt::UserRole` for navigation, not just display
5. **No filtering/sorting UI**: Despite using `QSortFilterProxyModel`, no filter input or sort toggle is exposed in the UI — the proxy model is set but has no visible controls
6. **Columns are movable**: User can drag-reorder columns
7. **R3000-only limitation**: No equivalent EE module view exists. If we want to show EE modules, we'd need a separate implementation using EE-specific debug data

---

## Implementation Checklist for Slint Port

- [ ] Table with 7 columns (Name, Version, Entry, GP, Text, Data, BSS)
- [ ] Hex formatting for addresses (8-digit)
- [ ] Range display for sections: `[start - end]`
- [ ] Empty BSS cell when bss_size==0
- [ ] Version display as major.minor
- [ ] Right-click context menu: Copy, Copy all as CSV
- [ ] Double-click: Entry/Text→Disassembler, GP/Data/BSS→MemoryView
- [ ] Auto-refresh on VM update events
- [ ] Monospace font
- [ ] Movable/resizable columns (if Slint supports)
- [ ] BSS guard: no action on double-click if size is 0
