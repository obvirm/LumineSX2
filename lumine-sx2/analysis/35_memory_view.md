# Memory View & Memory Search — Deep Analysis

Source files analyzed:
- `pcsx2-qt/Debugger/Memory/MemoryView.h`
- `pcsx2-qt/Debugger/Memory/MemoryView.cpp`
- `pcsx2-qt/Debugger/Memory/MemoryView.ui`
- `pcsx2-qt/Debugger/Memory/MemorySearchView.h`
- `pcsx2-qt/Debugger/Memory/MemorySearchView.cpp`
- `pcsx2-qt/Debugger/Memory/MemorySearchView.ui`
- `pcsx2-qt/Debugger/Memory/SavedAddressesModel.h`
- `pcsx2-qt/Debugger/Memory/SavedAddressesModel.cpp`
- `pcsx2-qt/Debugger/Memory/SavedAddressesView.h`
- `pcsx2-qt/Debugger/Memory/SavedAddressesView.cpp`
- `pcsx2-qt/Debugger/DebuggerEvents.h`

---

## 1. MemoryView — Hex Viewer

### Class: `MemoryViewTable`
Core rendering/interaction table. Owns all selection state, display mode, and endian toggle.

#### Display Modes (enum `MemoryViewType`)
| Enum | Value | Byte Width | Visual Width (hex chars) | Read Func | Format |
|------|-------|-----------|--------------------------|-----------|--------|
| `BYTE` | 0 | 1 | 2 | `cpu.Read8()` | `XX` |
| `BYTEHW` | 1 | 2 | 4 | `cpu.Read16()` | `XXXX` |
| `WORD` | 2 | 4 | 8 | `cpu.Read32()` | `XXXXXXXX` |
| `DWORD` | 3 | 8 | 16 | `cpu.Read64()` | `XXXXXXXXXXXXXXXX` |
| `FLOAT` | 4 | 4 | 14 | `cpu.Read32()` → `memcpy → float` | `QString::number(val, 'g')` right-padded to 14 chars |

#### Endian Toggle
- `littleEndian` bool, default `true`
- `convertEndian<T>()` template: if `littleEndian` returns `in`; otherwise `qToBigEndian(in)`
- Affects ALL read/write operations for BYTEHW, WORD, DWORD, FLOAT
- Persisted to/from JSON in `toJson`/`fromJson`
- Toggle via context menu: "Show as Little Endian" (checkable QAction)

#### Drawing (`DrawTable`)
- Fixed 16 bytes per row, rows = `height / rowHeight`
- Left column: address in hex (`FilledQStringFromValue(addr, 16)`)
- Center: hex values grouped by display type width, separated by `charWidth` space
- Right: ASCII text representation (unprintable → `.`)
- Color coding:
  - **Selected byte**: `QColor(0xaa, 0x22, 0x22)` — red
  - **Selected nibble underline**: `QColor(205, 165, 0)` — gold/orange
  - **Zero values**: `QColor(145, 145, 155)` — muted gray
  - **Default text**: `palette.text().color()`
  - **Selected address in text area**: `palette.highlight().color()`

#### Selection Handling
- `selectedAddress` — currently selected byte address
- `selectedIndex` — character index within float display
- `selectedText` — bool, true if user clicked ASCII text area (vs hex area)
- `selectedNibbleHI` — which nibble is active (high/low) in hex editing
- `segmentXAxis[16]` — x-pixel positions of each 16-byte segment

#### Keyboard Input (`KeyPress`)
| Key | Text Area | Hex Area |
|-----|-----------|----------|
| Letter/Number | Write ASCII byte, advance | Validate hex, insert nibble, advance |
| Backspace/Escape | Write 0, go backward | Write 0 nibble, go backward |
| Right | ForwardSelection | ForwardSelection |
| Left | BackwardSelection | BackwardSelection |
| Enter/Return | — | If FLOAT: open float input dialog |
| Up | `selectedAddress -= 0x10` | Same |
| Down | `selectedAddress += 0x10` | Same |
| PageUp | `selectedAddress -= 0x10 * rowVisible` (paged) | Same |
| PageDown | `selectedAddress += 0x10 * rowVisible` (paged) | Same |

#### Floating Point Editing
- `InsertFloatIntoSelectedHexView()`: Opens `AsyncDialogs::getText` dialog with current float as default
- Validates float input, converts to u32 via `memcpy`, applies endian, writes via `cpu.Write32()` on CPU thread

#### Hex Byte Editing
- `InsertIntoSelectedHexView(u8 value)`: Nibble-aware insert. Masks existing byte, ORs new nibble.
- Writes via `Host::RunOnCPUThread` → `cpu.Write8()` → `QtHost::RunOnUIThread` → `update()`

#### Paste (`InsertAtCurrentSelection`)
- FLOAT mode: parses text as float, writes u32
- Text area: writes UTF-8 bytes sequentially
- Hex area: decodes text as hex (`QByteArray::fromHex`), writes bytes sequentially
- Advances selection to end of pasted region

#### Navigation
- `ForwardSelection()`: Advance nibble/char; in little-endian mode, addresses go backwards within segment
- `BackwardSelection()`: Reverse of forward
- `nextAddress()` / `prevAddress()`: Static helpers for endian-aware address stepping
- `gotoAddress(u32)`: Sets `startAddress` aligned to 0x10, sets `selectedAddress`

### Class: `MemoryView` (extends `DebuggerView`)

#### Events Received
| Event | Handler |
|-------|---------|
| `DebuggerEvents::Refresh` | `update()` — repaint |
| `DebuggerEvents::GoToAddress` (filter=`NONE` or `MEMORY_VIEW`) | `gotoAddress()` + optional `switchToThisTab()` |

#### Context Menu (`openContextMenu`)
| Action | Behavior |
|--------|----------|
| **Copy Address** | Copies hex address string to clipboard |
| **Go to in [other views]** | Emits `GoToAddress` event (creates actions for disassembly etc.) |
| **Go to Address** | Opens expression dialog → evaluates → `gotoAddress()` |
| **Follow Address** | Reads u32 at `(selectedAddress & ~3)`, jumps to that address |
| **Show as Little Endian** | Checkable toggle |
| **Show as 1 byte** | Radio: BYTE mode |
| **Show as 2 bytes** | Radio: BYTEHW mode |
| **Show as 4 bytes** | Radio: WORD mode |
| **Show as 8 bytes** | Radio: DWORD mode |
| **Show as float** | Radio: FLOAT mode |
| **Add to Saved Addresses** | Emits `AddToSavedAddresses` event |
| **Copy Byte** | Copies selected byte as hex (disabled in FLOAT mode) |
| **Copy Segment** | Copies selected segment value (float format if FLOAT mode) |
| **Copy Character** | Copies selected byte as Latin-1 char (disabled in FLOAT mode) |
| **Paste** | Pastes clipboard at current selection |

#### Keyboard Shortcuts
| Key | Action |
|-----|--------|
| `G` | Open "Go to Address" dialog |
| `Ctrl+C` | Copy segment |

#### Mouse Events
- `mousePressEvent`: `m_table.SelectAt(pos)` → click-to-select
- `mouseDoubleClickEvent`: Empty (no-op)
- `wheelEvent`: Scroll up/down by 0x10 bytes per wheel tick

#### JSON Persistence
```json
{
  "startAddress": <u32>,
  "viewType": <int 0-4>,
  "littleEndian": <bool>
}
```

---

## 2. MemorySearchView — Memory Search / Cheat Finder

### Search Types (enum `SearchType`)
| Type | Index | Underlying | Read Size |
|------|-------|------------|-----------|
| `ByteType` | 0 | u8/s8 | 1 |
| `Int16Type` | 1 | u16/s16 | 2 |
| `Int32Type` | 2 | u32/s32 | 4 |
| `Int64Type` | 3 | u64/s64 | 8 |
| `FloatType` | 4 | float | 4 |
| `DoubleType` | 5 | double | 8 |
| `StringType` | 6 | QByteArray | variable |
| `ArrayType` | 7 | QByteArray (hex) | variable |

### Search Comparisons (enum `SearchComparison`)
| Comparison | Takes Input | Requires Prior Results | Description |
|------------|-------------|----------------------|-------------|
| Equals | Yes | No | Exact match (float/double: ±0.00001) |
| NotEquals | Yes | No | Not equal |
| GreaterThan | Yes | No | Value > input |
| GreaterThanOrEqual | Yes | No | Value >= input |
| LessThan | Yes | No | Value < input |
| LessThanOrEqual | Yes | No | Value <= input |
| Increased | No | Yes (filter only) | Value increased from last search |
| IncreasedBy | Yes | Yes (filter only) | Value increased by exact amount |
| Decreased | No | Yes (filter only) | Value decreased from last search |
| DecreasedBy | Yes | Yes (filter only) | Value decreased by exact amount |
| Changed | No | Yes (filter only) | Value changed from last search |
| ChangedBy | Yes | Yes (filter only) | Value changed by exact amount (up or down) |
| NotChanged | No | Yes (filter only) | Value unchanged |
| UnknownValue | No | No | First scan only, matches everything |

### UI Elements (from `MemorySearchView.ui`)
| Widget | Name | Type | Description |
|--------|------|------|-------------|
| Value input | `txtSearchValue` | QLineEdit | Search value |
| Comparison | `cmbSearchComparison` | QComboBox | Comparison operator |
| Type | `cmbSearchType` | QComboBox | Data type (8 options) |
| Hex checkbox | `chkSearchHex` | QCheckBox | Default checked; enables hex input for integer types |
| Search button | `btnSearch` | QPushButton | New search |
| Filter button | `btnFilterSearch` | QPushButton | Narrow existing results (disabled until results exist) |
| Start address | `txtSearchStart` | QLineEdit | Default `0x00` |
| End address | `txtSearchEnd` | QLineEdit | Default `0x2000000` |
| Results list | `listSearchResults` | QListWidget | Shows matched addresses |
| Results count | `resultsCountLabel` | QLabel | "N results found" or "Searching..." |

### Search Behavior
- **New search**: Clears all prior results, scans entire `start→end` range
- **Filter search**: Narrows existing results using prior values for comparison
- Runs on `QtConcurrent::run` (background thread)
- Results loaded lazily: initial 20,000, then 10,000 per scroll (95% of scrollbar)
- Debounced scroll loading via `QTimer` (100ms single-shot)
- Signed/unsigned detection: if value string starts with `-`, uses signed types

### Float/Double Comparison
- Uses epsilon of ±0.00001 for Equals/NotEquals comparisons
- GreaterThan/LessThan use the same epsilon band

### String/Array Search
- String: raw UTF-8 bytes
- Array: hex-decoded (`QByteArray::fromHex`)
- Supports Equals, NotEquals, Changed, NotChanged only
- Array search steps by 1 byte (not by element size)

### Context Menu on Results
| Action | Behavior |
|--------|----------|
| Copy Address | Copies hex address to clipboard |
| Go to in [views] | Emits `GoToAddress` event |
| Add to Saved Addresses | Emits `AddToSavedAddresses` event |
| Remove Result | Removes from list and from `m_searchResults` vector |

### Double-click on result
- `goToInMemoryView(address, true)` — jumps MemoryView to that address

### Dynamic Comparison ComboBox
- Comparisons change based on:
  - Search type (String/Array: only Equals + NotEquals + Changed + NotChanged)
  - Whether prior results exist (first scan: adds UnknownValue; with results: adds Increased/Decreased/Changed/NotChanged variants)
- Selection preserved when combo updates (if still valid)

---

## 3. SavedAddressesModel — Bookmark Table

### Data Structure
```cpp
struct SavedAddress {
    u32 address;
    QString label;
    QString description;
};
```

### Columns
| Index | Header | Resize Mode |
|-------|--------|-------------|
| 0 | MEMORY ADDRESS | ResizeToContents |
| 1 | LABEL | ResizeToContents |
| 2 | DESCRIPTION | Stretch |

### Features
- Singleton per `BreakPointCpu` (EE vs IOP)
- Editable cells (address: hex parse, label/description: free text)
- `Qt::UserRole` returns raw data (u32 address, QString label/description)
- `addRow()` / `addRow(SavedAddress)` / `removeRows()` — standard CRUD
- `loadSavedAddressFromFieldList(QStringList)` — CSV import (3 fields)
- `clear()` — reset model
- `DebuggerSettingsManager::loadGameSettings(m_model)` — loads from per-game settings file
- Auto-loads on game change if empty

---

## 4. SavedAddressesView — Bookmark UI

### Context Menu
| Action | Behavior |
|--------|----------|
| New | Adds empty row, starts editing address cell |
| Go to in [views] | Emits `GoToAddress` from selected row's address |
| Copy Address/Text | Copies cell content (address column → hex string, others → display text) |
| Copy all as CSV | Exports all rows as CSV via `AbstractItemModelToCSV` |
| Paste from CSV | Parses clipboard CSV, handles quoted fields with commas |
| Load from Settings | Clears and reloads from `DebuggerSettingsManager` |
| Save to Settings | Persists to `DebuggerSettingsManager` |
| Delete | Removes selected row |

### Event Handling
- Receives `DebuggerEvents::AddToSavedAddresses` → calls `addAddress(address)` → adds row, starts editing label
- Switches to tab if `event.switch_to_tab` is true

### Auto-load
- On `EmuThread::onGameChanged`, if model is empty, loads from game settings

---

## 5. Events Used

| Event | Direction | Data |
|-------|-----------|------|
| `Refresh` | Broadcast → MemoryView, MemorySearchView | none |
| `GoToAddress` | MemoryView receives; MemoryView/MemorySearchView emit | `u32 address`, `Filter filter`, `bool switch_to_tab` |
| `VMUpdate` | MemoryView emits on keypress | none |
| `AddToSavedAddresses` | SavedAddressesView receives; MemoryView/MemorySearchView emit | `u32 address`, `bool switch_to_tab` |

### GoToAddress Filter Values
- `NONE` — handled by any view
- `DISASSEMBLER` — only disassembly view handles it
- `MEMORY_VIEW` — only memory view handles it

---

## 6. Hidden/Advanced Features

1. **Expression evaluation in Go To**: `cpu.evaluateExpression()` — supports arbitrary expressions, not just hex addresses
2. **Endian-aware nibble selection**: In little-endian mode, visual left-to-right cursor maps to right-to-left byte addresses within a segment
3. **Lazy result loading**: Memory search results are loaded in chunks with scroll-position debouncing to prevent UI freezes
4. **Concurrent search**: `QtConcurrent::run` with `QFutureWatcher` — search runs on background thread, UI updates on finish
5. **Float epsilon comparison**: ±0.00001 tolerance for float/double equality
6. **Paste as hex decode**: In hex area mode, clipboard text is decoded as hex bytes before writing
7. **Paste as UTF-8**: In text area mode, clipboard is written as raw Latin-1 bytes
8. **Per-game saved addresses**: Persisted per game serial via DebuggerSettingsManager
9. **Singleton model**: SavedAddressesModel is singleton per CPU type (EE/IOP) — shared across all SavedAddressesView instances
10. **CSV round-trip**: Export/import saved addresses with quoted-field CSV parsing (handles commas in descriptions)
11. **Address alignment**: `startAddress` always aligned to 0x10; `GoToAddress` aligns to 0x10
12. **Color-coded zero bytes**: Zero values render in muted gray for quick visual scanning
13. **Nibble cursor underline**: Gold/orange line drawn under the active nibble position
14. **Writable hex editing**: Direct nibble-by-nibble hex editing with automatic endian handling
15. **Writable text editing**: Direct ASCII character-by-character editing in text area
16. **Float input dialog**: Enter key on float display opens input dialog with current value pre-filled
17. **Backspace writes zero**: In both hex and text modes, backspace writes 0 and moves backward
18. **Signed/unsigned auto-detect**: Search input starting with `-` triggers signed integer comparison
19. **Comparison combobox dynamic repopulation**: Options change based on whether prior results exist and the selected type
20. **Filter-only comparisons**: Increased/Decreased/Changed/NotChanged/ChangedBy/IncreasedBy/DecreasedBy disabled for initial scans
21. **Address search range**: Default 0x00 to 0x2000000 (32MB), user-configurable
22. **Hex mode checkbox**: For integer types only (disabled for float/double/string/array), default checked
