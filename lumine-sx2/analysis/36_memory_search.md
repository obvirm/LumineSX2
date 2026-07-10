# Memory Search View — Deep Analysis

## Source Files
- `pcsx2-qt/Debugger/Memory/MemorySearchView.h` (145 lines)
- `pcsx2-qt/Debugger/Memory/MemorySearchView.cpp` (420 lines)
- `pcsx2-qt/Debugger/Memory/MemorySearchView.ui` (160 lines)

---

## Class Hierarchy

```
DebuggerView (QWidget subclass with CPU access)
  └── MemorySearchView : public DebuggerView
```

Inherits `DebuggerView` which provides:
- `cpu()` — `DebugInterface&` for reading/writing PS2 memory
- `goToInMemoryView(address, bool)` — navigates Memory View to address
- `receiveEvent<T>()` — event-based communication
- `createEventActions<T>()` — context menu action factory

---

## Search Types (8 types)

| Enum Value | UI Label | Template Type | Size | Hex Support |
|---|---|---|---|---|
| `ByteType` (0) | "1 Byte (8 bits)" | `u8`/`s8` | 1 byte | ✅ |
| `Int16Type` (1) | "2 Bytes (16 bits)" | `u16`/`s16` | 2 bytes | ✅ |
| `Int32Type` (2) | "4 Bytes (32 bits)" | `u32`/`s32` | 4 bytes | ✅ |
| `Int64Type` (3) | "8 Bytes (64 bits)" | `u64`/`s64` | 8 bytes | ✅ |
| `FloatType` (4) | "Float" | `float` | 4 bytes | ❌ |
| `DoubleType` (5) | "Double" | `double` | 8 bytes | ❌ |
| `StringType` (6) | "String" | `QByteArray` (UTF-8) | variable | ❌ |
| `ArrayType` (7) | "Byte Array" | `QByteArray` (hex) | variable | ❌ |

**Hidden behavior**: Hex checkbox auto-disabled for Float/Double/String/ArrayType (indices ≥ 4).

---

## Search Comparisons (15 operators)

| Enum | Label | Takes Input | Requires Prior Results | Available for Array/String |
|---|---|---|---|---|
| `Equals` | "Equals" | ✅ | ❌ | ✅ |
| `NotEquals` | "Not Equals" | ✅ | ❌ (Array/String: needs prior) | ✅ |
| `GreaterThan` | "Greater Than" | ✅ | ❌ | ❌ |
| `GreaterThanOrEqual` | "Greater Than Or Equal" | ✅ | ❌ | ❌ |
| `LessThan` | "Less Than" | ✅ | ❌ | ❌ |
| `LessThanOrEqual` | "Less Than Or Equal" | ✅ | ❌ | ❌ |
| `Increased` | "Increased" | ❌ | ✅ | ❌ |
| `IncreasedBy` | "Increased By" | ✅ | ✅ | ❌ |
| `Decreased` | "Decreased" | ❌ | ✅ | ❌ |
| `DecreasedBy` | "Decreased By" | ✅ | ✅ | ❌ |
| `Changed` | "Changed" | ❌ | ✅ | ✅ |
| `ChangedBy` | "Changed By" | ✅ | ✅ | ❌ |
| `NotChanged` | "Not Changed" | ❌ | ✅ | ✅ |
| `UnknownValue` | "Unknown Initial Value" | ❌ | ❌ (only first search) | ❌ |
| `Invalid` | "" | — | — | — |

**Dynamic combo box**: `updateSearchComparisonSelections()` rebuilds the comparison combo based on:
1. Current search type
2. Whether prior results exist
3. Type of prior results (must match current type for filter comparisons)

---

## UI Elements

### Top Grid (row 0-2, col 0-3)

| Widget | Name | Type | Purpose |
|---|---|---|---|
| `txtSearchValue` | QLineEdit | Row 0, Col 1 | Search value input |
| `btnSearch` | QPushButton | Row 0, Col 2-3 | "Search" — new search |
| `cmbSearchComparison` | QComboBox | Row 1, Col 1 | Comparison operator |
| `btnFilterSearch` | QPushButton | Row 1, Col 2-3 | "Filter Search" — refine results |
| `cmbSearchType` | QComboBox | Row 2, Col 1 | Data type selector |
| `chkSearchHex` | QCheckBox | Row 2, Col 3 | Hex mode (default: checked) |

### Address Range Grid

| Widget | Name | Default |
|---|---|---|
| `txtSearchStart` | QLineEdit | "0x00" |
| `txtSearchEnd` | QLineEdit | "0x2000000" (32 MB) |

### Results Area

| Widget | Name | Purpose |
|---|---|---|
| `listSearchResults` | QListWidget | Scrollable result list (addresses only) |
| `resultsCountLabel` | QLabel | "N results found" / "Searching..." (hidden by default) |

### Tab Order
`txtSearchValue` → `cmbSearchComparison` → `cmbSearchType` → `chkSearchHex` → `txtSearchStart` → `txtSearchEnd` → `btnSearch` → `btnFilterSearch` → `listSearchResults`

---

## Signals/Callbacks

| Signal Source | Slot | Trigger |
|---|---|---|
| `btnSearch.clicked` | `onSearchButtonClicked()` | New search |
| `btnFilterSearch.clicked` | `onSearchButtonClicked()` | Filter existing results |
| `listSearchResults.itemDoubleClicked` | Lambda | Navigate to Memory View |
| `listSearchResults.verticalScrollBar().valueChanged` | `onSearchResultsListScroll()` | Lazy load trigger |
| `listSearchResults.customContextMenuRequested` | `onListSearchResultsContextMenu()` | Right-click menu |
| `cmbSearchType.currentIndexChanged` | `onSearchTypeChanged()` | Type change → update comparisons, clear results |
| `cmbSearchComparison.currentIndexChanged` | `onSearchComparisonChanged()` | Toggle value input enabled |
| `QTimer.timeout` (100ms) | `loadSearchResults()` | Debounced lazy load |
| `DebuggerEvents::Refresh` | `update()` | CPU state refresh |

---

## Search Algorithm

### New Search (btnSearch)
1. Validate start < end addresses
2. Validate search value matches type
3. Validate comparison is not filter-only (Changed, Increased, etc.)
4. Run `QtConcurrent::run(startWorker)` on background thread
5. `startWorker` dispatches by type to `searchWorker<T>` or `searchWorkerByteArray`
6. Iterates `addr = start` to `end`, step by `sizeof(T)` (or 1 for byte arrays)
7. Calls `cpu->isValidAddress(addr)` + `cpu->Read<T>(addr)` for each
8. Matches via `handleSearchComparison<T>()`
9. Returns `std::vector<SearchResult>`
10. On finish: clears UI, loads first 20,000 results, updates comparison selections

### Filter Search (btnFilterSearch)
1. Same validation as new search
2. Runs same worker but passes existing `m_searchResults`
3. Worker uses `std::remove_if` to eliminate non-matching entries
4. Updates in-place (no full rescan)

### Float Comparison Precision
- Float: ±0.00001f tolerance for Equals/GreaterThan/LessThan
- Double: ±0.00001f tolerance (note: uses `float` literal `0.00001f` for double — **potential bug**)

### Signed/Unsigned Handling
- Integer types: detects sign from `-` prefix in value string
- Routes to `searchWorker<s8/s16/s32/s64>` or `searchWorker<u8/u16/u32/u64>`
- Hex mode: `chkSearchHex` → `searchHex ? 16 : 10` base for parsing

---

## Lazy Loading / Virtual Scrolling

| Setting | Value |
|---|---|
| `m_initialResultsLoadLimit` | 20,000 |
| `m_numResultsAddedPerLoad` | 10,000 |
| Scroll threshold | 95% of scrollbar max |
| Debounce timer | 100ms, single-shot |

### Flow
1. `onSearchResultsListScroll(value)` — called on scrollbar change
2. If >95% scrolled AND more results pending → start 100ms timer
3. Timer fires `loadSearchResults()`
4. Adds next batch of `QListWidgetItem` with address text (hex padded)
5. Each item stores address in `Qt::UserRole` data

---

## Context Menu Actions

| Action | Handler | Event Dispatched |
|---|---|---|
| "Copy Address" | `contextCopySearchResultAddress()` | — (clipboard) |
| "Go to in Memory View" | Via `createEventActions<DebuggerEvents::GoToAddress>` | `GoToAddress` |
| "Add to Saved Addresses" | Via `createEventActions<DebuggerEvents::AddToSavedAddresses>` | `AddToSavedAddresses` |
| "Remove Result" | `contextRemoveSearchResult()` | — (local erase) |

---

## Integration Points

### Events Dispatched (outgoing)
- `DebuggerEvents::GoToAddress{address}` — opens Memory View at address
- `DebuggerEvents::AddToSavedAddresses{address}` — adds to Saved Addresses panel

### Events Received (incoming)
- `DebuggerEvents::Refresh` — triggers UI update when CPU state changes

### Saved Addresses Integration
- Context menu "Add to Saved Addresses" fires `AddToSavedAddresses` event
- Address copied with `FilledQStringFromValue(address, 16)` (zero-padded hex)

---

## SearchResult Data Model

```cpp
class SearchResult {
    u32 address;       // PS2 memory address
    QVariant value;    // Typed value (or QByteArray for arrays/strings)
    SearchType type;   // Original search type
    
    bool isIntegerValue();   // Byte/Int16/Int32/Int64
    bool isFloatValue();     // Float
    bool isDoubleValue();    // Double
    bool isArrayValue();     // Array/String
    u32 getAddress();
    SearchType getType();
    QByteArray getArrayValue();  // Only for array types
    template<T> T getValue();    // Typed access via QVariant
};
```

---

## SearchComparisonLabelMap (Bidirectional)

Maps between enum values and translated UI strings. Used to:
1. Populate combo box with human-readable labels
2. Convert selected label back to enum for comparison logic
3. Preserve selection when combo is rebuilt (after type change or new search)

---

## Hidden Behaviors / Edge Cases

1. **Type change clears results**: Changing search type clears `m_searchResults` and disables filter button
2. **Filter-only comparisons**: Changed/ChangedBy/Decreased/DecreasedBy/Increased/IncreasedBy/NotChanged require prior results
3. **Unknown Value**: Only available for initial search (no prior results), always matches
4. **Array/String limitations**: Only Equals/NotEquals/Changed/NotChanged available
5. **Value validation cascade**: Integer types fall through size checks (Byte → Int16 → Int32 → Int64)
6. **Async search**: Uses `QtConcurrent::run` with `QFutureWatcher` — search runs on thread pool
7. **Result removal**: Context menu "Remove Result" erases from both `m_searchResults` vector and QListWidget
8. **Double-click navigation**: `goToInMemoryView(address, true)` — second param likely "focus" flag
9. **Progressive disclosure**: Results load 20k initially, then 10k on each scroll-to-bottom
10. **Float precision**: Uses ±0.00001f epsilon — approximate equality, not exact match
11. **Byte array search**: Steps by 1 byte (not sizeof), skips ahead by pattern length on match
12. **ChangedBy**: Checks both increase AND decrease by the value (bidirectional change)
13. **Signed detection**: Simple `-` prefix check — works for decimal, potentially wrong for hex

---

## Potential Bugs in Source

1. **Double precision**: Uses `0.00001f` (float literal) for double comparison — should be `0.00001`
2. **Signed hex parsing**: `-` prefix detection with hex base may misparse
3. **Value size cascade**: Missing `break` in switch — intentional fallthrough for size validation but fragile
4. **Thread safety**: `m_searchResults` accessed from worker thread and UI thread without visible locking

---

## Statistics
- **Total lines**: ~725 (h + cpp + ui)
- **Search types**: 8
- **Comparison operators**: 15 (including Invalid)
- **UI widgets**: 9 (2 text inputs, 2 combos, 2 buttons, 1 checkbox, 1 list, 1 label)
- **Context menu actions**: 4
- **Events dispatched**: 2 (GoToAddress, AddToSavedAddresses)
- **Events received**: 1 (Refresh)
- **Background threading**: QtConcurrent::run
- **Lazy loading**: 20k initial, 10k per batch, 95% scroll trigger, 100ms debounce
