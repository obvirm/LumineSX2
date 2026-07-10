# 37 — Breakpoints System (BreakpointDialog, BreakpointModel, BreakpointView)

## Files Analyzed
| File | Path |
|------|------|
| BreakpointDialog.h | pcsx2-qt/Debugger/Breakpoints/BreakpointDialog.h |
| BreakpointDialog.cpp | pcsx2-qt/Debugger/Breakpoints/BreakpointDialog.cpp |
| BreakpointModel.h | pcsx2-qt/Debugger/Breakpoints/BreakpointModel.h |
| BreakpointModel.cpp | pcsx2-qt/Debugger/Breakpoints/BreakpointModel.cpp |
| BreakpointView.h | pcsx2-qt/Debugger/Breakpoints/BreakpointView.h |
| BreakpointView.cpp | pcsx2-qt/Debugger/Breakpoints/BreakpointView.cpp |

---

## 1. Core Data Type

```cpp
using BreakpointMemcheck = std::variant<BreakPoint, MemCheck>;
```

Two kinds of breakpoints stored as a **variant**:
- **BreakPoint** — execution breakpoint (stop when PC hits address)
- **MemCheck** — memory watchpoint (stop/log when memory read/written)

---

## 2. BreakpointModel (QAbstractTableModel)

### 2.1 Columns (8 total)

| Index | Enum | Header | ResizeMode | Editable | Description |
|-------|------|--------|------------|----------|-------------|
| 0 | ENABLED | "X" | ResizeToContents | ✅ Checkable | Checkbox to enable/disable |
| 1 | TYPE | "TYPE" | ResizeToContents | ❌ | "Execute" or "Read", "Write", "Write(C)", "Read, Write" |
| 2 | OFFSET | "OFFSET" | ResizeToContents | ❌ | Hex address |
| 3 | DESCRIPTION | "DESCRIPTION" | ResizeToContents | ✅ Editable | User description string |
| 4 | SIZE_LABEL | "SIZE / LABEL" | Stretch | ❌ | For BreakPoint: function name at address. For MemCheck: hex size |
| 5 | OPCODE | "INSTRUCTION" | Stretch | ❌ | Disassembled instruction (BreakPoint only, "--" for MemCheck) |
| 6 | CONDITION | "CONDITION" | ResizeToContents | ✅ Editable | Expression string (e.g. "v0 == 0x100") |
| 7 | HITS | "HITS" | ResizeToContents | ❌ | Hit count (MemCheck only, "--" for BreakPoint) |

### 2.2 Roles

| Role | Value | Purpose |
|------|-------|---------|
| DisplayRole | Qt::DisplayRole | Translated display text |
| EditRole | Qt::EditRole | Editable field values (CONDITION, DESCRIPTION) |
| DataRole | Qt::UserRole | Raw numeric data (addresses, flags, sizes) |
| ExportRole | Qt::UserRole + 1 | CSV-exportable values (translation-agnostic) |
| CheckStateRole | Qt::CheckStateRole | Checkbox state for ENABLED column |

### 2.3 Singleton Pattern

```cpp
static BreakpointModel* getInstance(DebugInterface& cpu);
static std::map<BreakPointCpu, BreakpointModel*> s_instances;
```

One model per CPU type (EE, IOP). Lazy-created on first access.

### 2.4 Data Refresh Flow

```
refreshData() → Host::RunOnCPUThread → CBreakPoints::GetBreakpoints() + GetMemChecks()
  → QtHost::RunOnUIThread → beginResetModel() / endResetModel()
```

All data reads happen on CPU thread, then dispatched to UI thread.

### 2.5 Auto-load on Game Change

```cpp
// Only for EE CPU
connect(g_emu_thread, &EmuThread::onGameChanged, this, [this](const QString& title) {
    if (title.isEmpty()) return;
    if (rowCount() == 0) DebuggerSettingsManager::loadGameSettings(this);
});
```

When a game loads and no breakpoints exist, auto-load from saved settings.

### 2.6 Inline Editing

| Column | Edit Mechanism | Action |
|--------|---------------|--------|
| ENABLED | Checkbox toggle | `CBreakPoints::ChangeBreakPoint()` or `ChangeMemCheck()` toggle MEMCHECK_BREAK |
| CONDITION | Text edit | Parse expression → `ChangeBreakPointAddCond()` / `ChangeMemCheckAddCond()`. Empty = remove condition |
| DESCRIPTION | Text edit | `ChangeBreakPointDescription()` / `ChangeMemCheckDescription()` |

All mutations go through `Host::RunOnCPUThread`.

### 2.7 CSV Import/Export

**Export format**: `ExportRole` outputs translation-agnostic values. Columns pipe-separated with quote-wrapping for values containing commas.

**Import**: `loadBreakpointFromFieldList(QStringList)` — parses 8 fields:
- `TYPE == MEMCHECK_INVALID` → BreakPoint
- `TYPE < MEMCHECK_INVALID` → MemCheck with `memCond = static_cast<MemCheckCondition>(type)`
- Validates all fields, logs errors to Console, skips on failure

---

## 3. BreakpointDialog (QDialog)

### 3.1 Two Modes

| Constructor | Purpose Mode | Behavior |
|-------------|-------------|----------|
| `BreakpointDialog(parent, cpu, model)` | CREATE | Empty form, creates new BreakPoint or MemCheck |
| `BreakpointDialog(parent, cpu, model, bp_mc, rowIndex)` | EDIT | Pre-fills form from existing breakpoint |

### 3.2 UI Elements (from code references)

| Widget | Type | Purpose |
|--------|------|---------|
| `rdoExecute` | QRadioButton | Select execution breakpoint type |
| `rdoMemory` | QRadioButton | Select memory watchpoint type |
| `grpType` | QGroupBox | Type selection group |
| `grpMemory` | QGroupBox | Memory-specific options group |
| `txtAddress` | QLineEdit | Start address (hex, expression-evaluated) |
| `txtSize` | QLineEdit | Size in bytes (MemCheck only) |
| `txtDescription` | QLineEdit | User description |
| `txtCondition` | QLineEdit | Conditional expression |
| `chkEnable` | QCheckBox | Enable/disable breakpoint |
| `chkRead` | QCheckBox | MEMCHECK_READ flag |
| `chkWrite` | QCheckBox | MEMCHECK_WRITE flag |
| `chkChange` | QCheckBox | MEMCHECK_WRITE_ONCHANGE flag |
| `chkLog` | QCheckBox | MEMCHECK_LOG result flag |

### 3.3 Radio Toggle Logic

```cpp
void onRdoButtonToggled() {
    bool isExecute = rdoExecute->isChecked();
    grpMemory->setEnabled(!isExecute);  // Disable memory options for exec BP
    chkLog->setEnabled(!isExecute);     // Log only for memory watchpoints
}
```

### 3.4 Accept (Save) Flow

1. **Validate address** via `m_cpu->evaluateExpression()` — shows warning on failure
2. For BreakPoint: set `addr`, `description`, `enabled`, condition
3. For MemCheck: validate address + size, set `start/end`, `memCond` flags, `result` flags, condition
4. **Condition parsing**: `m_cpu->initExpression()` → PostfixExpression — shows warning on failure
5. If EDIT mode: `removeRows(rowIndex, 1)` first
6. `insertBreakpointRows(0, 1, {bp_mc})` to add

### 3.5 MemCheck Flags

| Flag | Constant | Meaning |
|------|----------|---------|
| Read | `MEMCHECK_READ` | Trigger on read |
| Write | `MEMCHECK_WRITE` | Trigger on write |
| Write on Change | `MEMCHECK_WRITE_ONCHANGE` | Only trigger if value actually changed |
| Break | `MEMCHECK_BREAK` | Pause execution |
| Log | `MEMCHECK_LOG` | Log to console |

Combined: `MEMCHECK_READWRITE = MEMCHECK_READ | MEMCHECK_WRITE`

---

## 4. BreakpointView (DebuggerView)

### 4.1 Setup

```cpp
breakpointList->setContextMenuPolicy(Qt::CustomContextMenu);
breakpointList->setModel(m_model);
breakpointList->horizontalHeader()->setSectionsMovable(true); // Drag-reorder columns
```

### 4.2 Context Menu Actions

| Action | Condition | Behavior |
|--------|-----------|----------|
| **New** | `cpu().isAlive()` | Opens BreakpointDialog in CREATE mode |
| **Edit** | Selection exists | Opens BreakpointDialog in EDIT mode for selected row |
| **Copy** | Single selection | Copies current cell text to clipboard |
| **Delete** | Selection exists | Removes all selected rows (reverse order to preserve indices) |
| **Copy all as CSV** | `rowCount() > 0` | Copies all breakpoints as CSV to clipboard (ExportRole, translation-agnostic) |
| **Paste from CSV** | `cpu().isAlive()` | Parses clipboard CSV, imports all breakpoints |
| **Load from Settings** | EE CPU only | Clears model, reloads from DebuggerSettingsManager |
| **Save to Settings** | EE CPU only | Saves current breakpoints to DebuggerSettingsManager |

### 4.3 Double-Click Behavior

```cpp
void onDoubleClicked(const QModelIndex& index) {
    if (index.column() == OFFSET)
        goToInDisassembler(data(index, DataRole).toUInt(), true);
}
```

Double-clicking OFFSET column jumps to that address in the disassembly view.

### 4.4 CSV Parsing

Uses regex `R"("([^"]|\\.)*")` to match quote-wrapped values, handles escaped quotes. Skips header line.

---

## 5. DebuggerSettingsManager Integration

- `loadGameSettings(model)` — loads breakpoints from per-game settings file
- `saveGameSettings(model)` — persists breakpoints to per-game settings file
- Only available for **EE CPU** type (not IOP)

---

## 6. Key Architecture Patterns

### 6.1 Thread Safety

All CBreakPoints mutations go through `Host::RunOnCPUThread`. Data reads also go to CPU thread, then dispatch results back via `QtHost::RunOnUIThread`.

### 6.2 Variant-Based Polymorphism

`std::variant<BreakPoint, MemCheck>` avoids inheritance hierarchy. `std::get_if<T>()` used everywhere for type-safe access.

### 6.3 Singleton per CPU

```cpp
static std::map<BreakPointCpu, BreakpointModel*> s_instances;
```

One model instance per CPU type, lazily created. Shared across all BreakpointView instances for the same CPU.

### 6.4 Editable In-Place

CONDITION and DESCRIPTION columns are directly editable in the table via `setData()`. ENABLED column uses checkbox delegation.

---

## 7. Hidden Features & Edge Cases

1. **Expression evaluation for addresses**: Addresses aren't just hex — they support expressions like `"0x100000 + v0 * 4"` via `evaluateExpression()`
2. **Condition expressions**: Full postfix expression engine, e.g. `"$v0 == 0x100"`, `"$a0 != 0"`
3. **Write-on-change**: `MEMCHECK_WRITE_ONCHANGE` only triggers if the value actually changed, not just written
4. **Column reordering**: `setSectionsMovable(true)` allows drag-reordering columns
5. **CSV with quotes**: Import handles commas in values via quote-wrapping
6. **Auto-load on game start**: Breakpoints auto-restored from per-game settings when game loads
7. **Batch delete**: Multi-selection delete processes rows in reverse order
8. **Export role**: Separate from DisplayRole to keep CSV export translation-agnostic
9. **Size/Label dual purpose**: Column 4 shows function name for BreakPoints, hex size for MemChecks
10. **Hit counting**: MemCheck tracks `numHits` for profiling memory access patterns
