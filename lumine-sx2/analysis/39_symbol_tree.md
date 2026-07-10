# PCSX2 Qt Symbol Tree System - Deep Analysis

## Overview
Complete symbol tree system for the PCSX2 debugger. Provides hierarchical browsing of functions, global variables, local variables, and parameter variables with lazy-loaded children, live memory reading/writing, grouping, sorting, filtering, and type system integration.

## Files Analyzed
| File | Purpose |
|------|---------|
| SymbolTreeModel.h/.cpp | QAbstractItemModel with 6 columns, lazy loading via fetchMore/canFetchMore |
| SymbolTreeNode.h/.cpp | Tree node data: tag, symbol handle, location, type, value read/write, display string generation |
| SymbolTreeViews.h/.cpp | Base view + 4 concrete subclasses: Function, GlobalVariable, LocalVariable, ParameterVariable |
| SymbolTreeDelegates.h/.cpp | Custom item delegates for Value, Location, and Type columns with inline editing |
| SymbolTreeLocation.h/.cpp | Memory/register location abstraction with read/write operations |
| NewSymbolDialogs.h/.cpp | 4 dialog classes for creating new symbols |
| TypeString.h/.cpp | Bidirectional type<->string conversion (e.g. "int*[3]" <-> AST) |
| SymbolTreeView.ui | Layout: QTreeView + bottom panel (Refresh, Filter, +, -) |
| NewSymbolDialog.ui | Form dialog with tabs for storage type, name, address, register, size, type, function |

---

## SymbolTreeModel (QAbstractItemModel)

### Columns (6)
| Index | Name | Editable | Content |
|-------|------|----------|---------|
| 0 | NAME | No | Symbol name |
| 1 | VALUE | Yes | Live value from VM memory |
| 2 | LOCATION | Yes | Address (hex) or register name |
| 3 | SIZE | No | Size in bytes (optional, toggleable) |
| 4 | TYPE | Yes | Type string (e.g. "int", "float*") |
| 5 | LIVENESS | No | "Alive" or "Dead" based on PC in live range |

### Roles
- **DisplayRole**: All columns render text
- **ForegroundRole**: Grayed out if name doesn't match memory hash (column 0) or value is dead (column 1)
- **EDIT_ROLE** (Qt::EditRole): Write value to VM
- **UPDATE_FROM_MEMORY_ROLE** (Qt::UserRole): Read value from VM

### Lazy Loading
- `canFetchMore()`: Returns true if node has a valid type that can have children AND children haven't been fetched yet
- `fetchMore()`: Calls `populateChildren()` which resolves the physical type and generates child nodes for:
  - **ARRAY**: Element nodes `[0]`, `[1]`, ... with computed offsets
  - **POINTER/REFERENCE**: Dereferences pointer, creates single child `*name` at pointed-to address
  - **STRUCT/UNION**: Flattens fields (including nested), creates child per field with offset
- After populating, calls `readFromVM()` on each child to initialize display values

### Reset Mechanism
- `reset()`: Full model reset with new root
- `resetChildren()`: Clears children of a node, allows re-fetching
- `resetChildrenRecursive()`: Recursively clears all descendants
- `needsReset()`: True if root is null or all symbols in tree are invalid

### Temporary Type Change
- `changeTypeTemporarily()`: Parses type string, creates temporary AST node, reassigns node's type handle
- `typeFromModelIndexToString()`: Converts node's type back to string

---

## SymbolTreeNode

### Tags
| Tag | Description |
|-----|-------------|
| ROOT | Invisible root node |
| UNKNOWN_GROUP | Group node for unknown source file/section/module |
| GROUP | Group node for source file/section/module |
| OBJECT | Actual symbol node |

### Key Fields
- `symbol`: `ccc::MultiSymbolHandle` - polymorphic handle to Function/GlobalVariable/LocalVariable/ParameterVariable
- `name`, `mangled_name`: Display names
- `location`: `SymbolTreeLocation` (MEMORY address or REGISTER)
- `is_location_editable`: Whether user can edit the location
- `size`: Optional size in bytes
- `type`: `ccc::NodeHandle` pointing to AST type node
- `temporary_type`: For temporary type overrides
- `live_range`: `ccc::AddressRange` for liveness checking

### Memory Operations
- `readFromVM()`: Reads value, updates display string, liveness, hash matching
- `writeToVM()`: Writes value to VM memory, updates display
- `readValueAsVariant()`: Type-aware read (u8/u16/u32/u64/s8/.../float32/float64/enum/pointer)
- `writeValueFromVariant()`: Type-aware write
- `updateDisplayString()`: Generates display text with nested expansion:
  - Arrays: `{elem0,elem1,...}` with char array as `"string"`
  - Structs: `{.field1=val1,.field2=val2,...}` with flattening
  - Pointers: `0xADDR -> pointee_value`, NULL for 0, `"string"` for char*
  - Enums: Named constant or integer fallback
  - 128-bit values: Hex dump
  - Integers: Configurable base (bin/oct/dec/hex) with optional leading zeroes
  - Chars: Integer + `'c'` if printable

### Hash Matching
- `updateMatchesMemory()`: Compares current vs original function hash
  - Functions: Direct hash comparison
  - Global variables: Checks source file's `functions_match()` flag
  - Local/parameter variables: Checks parent function's hash
- Grayed out name = symbol has been modified in memory since load

### Liveness
- `updateLiveness()`: Checks if PC is within `[live_range.low, live_range.high)`
- Dead variables shown with grayed-out value

### Sorting
- `sortChildrenRecursively()`: Sorts by tag, then optionally by type-known status, then location, then name

### SymbolTreeDisplayOptions
- `integerBase()`: 2 (binary), 8 (octal), 10 (decimal), 16 (hex)
- `showLeadingZeroes()`: Pads display with leading zeros
- Conversion methods for string<->integer in configured base

---

## SymbolTreeViews (4 Concrete Views)

### Common Features (SymbolTreeView base class)
- **UI**: QTreeView + bottom panel with Refresh button, Filter text box, New (+) button, Delete (-) button
- **Filtering**: Real-time text filter via `filterBox` textEdited signal
- **Refresh**: Updates function hashes then rebuilds tree
- **Context menu** (right-click):
  - Copy Name
  - Copy Mangled Name (if ALLOW_MANGLED_NAME_ACTIONS)
  - Copy Location
  - Rename Symbol
  - Go To in Disassembler (if CLICK_TO_GO_TO_IN_DISASSEMBLER)
  - Show Size Column (checkable toggle)
  - Group by Module/Section/Source File (checkable, if ALLOW_GROUPING)
  - Sort by if type is known (checkable, if ALLOW_SORTING_BY_IF_TYPE_IS_KNOWN)
  - Reset Children (if ALLOW_TYPE_ACTIONS)
  - Change Type Temporarily (if ALLOW_TYPE_ACTIONS)
  - Integer Base submenu: Binary/Octal/Decimal/Hex (if ALLOW_TYPE_ACTIONS)
  - Show Leading Zeroes (checkable, if ALLOW_TYPE_ACTIONS)

### Grouping System
Three independent grouping axes, each creates hierarchical group nodes:
- **Group by Module**: Groups symbols by IOP/EE module, shows module version for IRX modules
- **Group by Section**: Groups by ELF section (.text, .data, etc.)
- **Group by Source File**: Groups by debug info source file

Grouping works via stable sort + single-pass tree construction. Groups can be nested (e.g. module > section > source file).

### Persistence (JSON)
- `toJson()`/`fromJson()`: Saves/restores showSizeColumn, groupByModule, groupBySection, groupBySourceFile, sortByIfTypeIsKnown, integerBase, showLeadingZeroes

### Flags
| Flag | Value | Used By |
|------|-------|---------|
| ALLOW_GROUPING | 0x01 | Function, GlobalVariable |
| ALLOW_SORTING_BY_IF_TYPE_IS_KNOWN | 0x02 | GlobalVariable |
| ALLOW_TYPE_ACTIONS | 0x04 | GlobalVariable, LocalVariable, ParameterVariable |
| ALLOW_MANGLED_NAME_ACTIONS | 0x08 | Function, GlobalVariable |
| CLICK_TO_GO_TO_IN_DISASSEMBLER | 0x10 | Function |

### 1. FunctionTreeView
- **Columns shown**: NAME, LOCATION (TYPE, VALUE, LIVENESS hidden)
- **Flags**: ALLOW_GROUPING, ALLOW_MANGLED_NAME_ACTIONS, CLICK_TO_GO_TO_IN_DISASSEMBLER
- **Alignment**: 4 bytes
- **Symbols**: All `ccc::Function` entries with valid addresses
- **Children**: Label nodes (internal labels within function, excluding function entry point)
- **Filter**: Case-insensitive name match
- **New dialog**: NewFunctionDialog

### 2. GlobalVariableTreeView
- **Columns shown**: NAME, LOCATION, TYPE, VALUE (LIVENESS hidden)
- **Flags**: ALLOW_GROUPING, ALLOW_SORTING_BY_IF_TYPE_IS_KNOWN, ALLOW_TYPE_ACTIONS, ALLOW_MANGLED_NAME_ACTIONS
- **Alignment**: 1 byte
- **Symbols**: 
  - All `ccc::GlobalVariable` entries with valid addresses
  - Static local variables (`ccc::LocalVariable` with `GlobalStorage`) - shown as "name (function_name)"
- **Children**: Populated lazily from type (arrays, structs, pointers)
- **Location editable**: Yes
- **New dialog**: NewGlobalVariableDialog

### 3. LocalVariableTreeView
- **Columns shown**: NAME, LOCATION, TYPE, VALUE, LIVENESS (all visible)
- **Flags**: ALLOW_TYPE_ACTIONS
- **Alignment**: 1 byte
- **Symbols**: Local variables of the function at current PC
- **Storage types**: GlobalStorage (address), RegisterStorage (register number), StackStorage (SP offset)
- **Liveness**: Full support via `live_range`
- **Auto-reset**: Resets when PC leaves the current function
- **New dialog**: NewLocalVariableDialog

### 4. ParameterVariableTreeView
- **Columns shown**: NAME, LOCATION, TYPE, VALUE (LIVENESS hidden)
- **Flags**: ALLOW_TYPE_ACTIONS
- **Alignment**: 1 byte
- **Symbols**: Parameter variables of the function at current PC
- **Storage types**: RegisterStorage, StackStorage
- **Auto-reset**: Resets when PC leaves the current function
- **New dialog**: NewParameterVariableDialog

---

## SymbolTreeDelegates (Custom Editors)

### SymbolTreeValueDelegate (Value Column)
Creates inline editors based on physical type:
| Type | Editor Widget |
|------|---------------|
| Unsigned int (8/16/32/64) | SymbolTreeIntegerLineEdit (base-aware) |
| Signed int (8/16/32/64) | SymbolTreeIntegerLineEdit (base-aware) |
| Bool | QCheckBox (immediate commit on state change) |
| Float32 | QLineEdit |
| Float64 | QLineEdit |
| Enum | QComboBox with all named constants + unnamed fallback |
| Pointer/Reference | QLineEdit (hex input) |

**Immediate commit**: CheckBox and ComboBox commit data on interaction, not on deselect.

### SymbolTreeLocationDelegate (Location Column)
- Creates QLineEdit for hex address input
- Only editable if `is_location_editable` flag is set
- Aligns address to `m_alignment` boundary
- On commit: moves symbol in database, resets children

### SymbolTreeTypeDelegate (Type Column)
- Creates QLineEdit for type string input
- Uses `stringToType()` parser
- On commit: Sets type on symbol in database, resets children
- Shows error dialog on invalid type string

### SymbolTreeIntegerLineEdit
- Custom QLineEdit respecting display options (base, leading zeroes)
- `unsignedValue()`/`signedValue()`: Parse text in configured base
- `setUnsignedValue()`/`setSignedValue()`: Format value in configured base

---

## SymbolTreeLocation

### Types
| Type | Description |
|------|-------------|
| REGISTER | EE GPR register (0-31), displayed as register name |
| MEMORY | RAM address |
| NONE | No location (sorts to bottom) |

### Operations
- `read8/16/32/64/128()`: Read from register or memory
- `write8/16/32/64/128()`: Write to register or memory
- `addOffset()`: Returns new location with offset (MEMORY only, REGISTER returns same if offset=0)
- `toString()`: Register name or hex address
- Supports `<=>` comparison for sorting

---

## New Symbol Dialogs

### NewFunctionDialog
- **Storage**: Global only
- **Fields**: Name, Address (hex), Size (3 options: fill existing function, fill empty space, custom)
- **Size options**: 
  - Fill existing function (auto-calculated from address to end of overlapping function)
  - Fill empty space (auto-calculated to next symbol)
  - Custom (4-byte aligned, max 256MB)
- **Conflict handling**: Option to shrink existing overlapping function
- **Alignment**: 4 bytes

### NewGlobalVariableDialog
- **Storage**: Global only
- **Fields**: Name, Address (hex), Type (string parsed via TypeString)
- **Alignment**: 1 byte

### NewLocalVariableDialog
- **Storage**: Global / Register / Stack (tabbed)
- **Fields**: Name, Storage, Type, Function (combo with all known functions, defaults to function at PC)
- **Stack storage**: Converts absolute offset to caller SP relative using `getStackFrameSize()`
- **Alignment**: 1 byte

### NewParameterVariableDialog
- **Storage**: Register / Stack (tabbed, no Global)
- **Fields**: Name, Storage, Type, Function
- **Stack storage**: Same caller SP conversion as local variables
- **Alignment**: 1 byte

### Common Dialog Features
- **Tab bar** for storage type selection (auto-hides if only one option)
- **Dynamic row visibility**: Only shows relevant fields per symbol type
- **Real-time validation**: All input widgets auto-trigger `parseUserInput()` on value change
- **Error display**: Red error message label, OK button disabled when invalid
- **Size field**: Radio buttons for fill-existing / fill-empty / custom

---

## TypeString System

### stringToType()
Parses type strings like `int*[3]` into AST:
- Supports: type names, pointers (`*`), references (`&`), arrays (`[N]`)
- Array subscripts parsed right-to-left (opposite of C, so `int*[3]` = pointer to array of 3 ints)
- Looks up type name in database's DataType symbols
- Returns AST with TypeName at leaf, PointerOrReference/Array wrapping

### typeToString()
Converts AST back to human-readable string:
- Traverses arrays/pointers/references building suffix
- Resolves TypeName to DataType name
- BuiltIn types use `builtin_class_to_string()`

---

## Hidden/Advanced Features

1. **Static local variables in global tree**: Local variables with GlobalStorage are shown in GlobalVariableTreeView as "name (function_name)"
2. **Function label children**: Function nodes show internal labels as children
3. **Module version display**: IRX modules show "name v1.2" format
4. **128-bit value display**: Raw hex dump for 128-bit values (EE registers)
5. **Char display**: Integers show `'c'` suffix for printable characters
6. **String dereference**: Char arrays and char* pointers show quoted strings via `stringFromPointer()`
7. **Recursive field flattening**: Struct/union fields are flattened (no nested struct nodes)
8. **Hash-based modification detection**: Names grayed when function code has been modified in memory
9. **Immediate checkbox/combobox commit**: No need to click away to save
10. **Auto-expand groups on filter**: When filter is active, all group nodes are expanded
11. **Visible-only hash updates**: Only hashes functions visible in viewport (performance optimization)
12. **Caller SP calculation**: Stack variable addresses computed relative to caller's stack frame
13. **Auto-reset on function change**: Local/parameter trees auto-reset when stepping out of function

---

## UI Layout

### SymbolTreeView
```
┌──────────────────────────────┐
│          QTreeView           │
│  (with alternating rows)     │
│                              │
├──────────────────────────────┤
│ [Refresh] [Filter...] [+] [-]│
└──────────────────────────────┘
```

### NewSymbolDialog
```
┌──────────────────────────────┐
│  [Global] [Register] [Stack] │  ← Tab bar (storage type)
├──────────────────────────────┤
│ Name:    [___________]       │
│ Address: [___________]       │  ← Hex, shown for Global
│ Register:[dropdown____]      │  ← Shown for Register
│ Stack Offset: [spinner]      │  ← Shown for Stack
│ Size: ○ Fill existing (N B)  │  ← Radio buttons
│       ○ Fill space (N B)     │
│       ○ Custom [spinner]     │
│ Existing: ○ Shrink ○ Don't   │  ← For functions
│ Type:    [___________]       │
│ Function:[dropdown____]      │  ← For local/parameter
├──────────────────────────────┤
│ [error message]  [OK][Cancel]│
└──────────────────────────────┘
```

---

## Statistics
- **Total lines**: ~2800 across 16 files
- **Classes**: 15 (SymbolTreeModel, SymbolTreeNode, SymbolTreeView + 4 subclasses, 3 delegates, 1 custom line edit, 1 location struct, 1 display options, NewSymbolDialog + 3 subclasses, TypeString)
- **Columns**: 6
- **View types**: 4 (Function, GlobalVariable, LocalVariable, ParameterVariable)
- **Type editors**: 3 (Value delegate, Location delegate, Type delegate)
- **Symbol creation dialogs**: 4 (Function, GlobalVariable, LocalVariable, ParameterVariable)
- **Supported built-in types**: 14 (u8/u16/u32/u64/s8/s16/s32/s64/f32/f64/bool8/128-bit variants)
- **Grouping axes**: 3 (Module, Section, Source File)
- **Integer bases**: 4 (Binary, Octal, Decimal, Hex)
