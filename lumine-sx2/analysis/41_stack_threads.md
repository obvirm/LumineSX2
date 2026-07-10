# Agent 41 — Stack View & Thread View

## StackView

### Class
- `StackView` extends `DebuggerView`
- Uses `Ui::StackView` (Qt Designer `.ui` file)
- Owns a `StackModel*` and a `Ui::StackView m_ui`

### UI Elements
- `m_ui.stackList` — QTableView displaying stack frames
  - Context menu policy: `Qt::CustomContextMenu`
  - Horizontal header: sections movable
  - Column resize modes set per-column from `StackModel::HeaderResizeModes`

### StackModel Columns (6)
| Column   | Header Label    | Resize Mode       | Display Data                     | UserRole Data |
|----------|-----------------|-------------------|----------------------------------|---------------|
| ENTRY    | "ENTRY"         | ResizeToContents  | `stackFrame.entry` (hex 16)      | raw u32 entry |
| ENTRY_LABEL | "LABEL"      | Stretch           | symbol name from SymbolGuardian  | symbol name   |
| PC       | "PC"            | ResizeToContents  | `stackFrame.pc` (hex 16)         | raw u32 pc    |
| PC_OPCODE| "INSTRUCTION"   | Stretch           | disasm at pc                     | disasm string |
| SP       | "STACK POINTER" | ResizeToContents  | `stackFrame.sp` (hex 16)         | raw u32 sp    |
| SIZE     | "SIZE"          | ResizeToContents  | `stackFrame.stackSize` (number)  | stackSize u32 |

### Data Source
- `MipsStackWalk::StackFrame` vector from `m_cpu.StackTrace(*thread)`
- Only shows stack for the **RUN** thread (`ThreadStatus::THS_RUN`)
- Refreshed on every `DebuggerEvents::VMUpdate`

### Context Menu Actions
1. **Copy** — copies current cell text to clipboard
2. *(separator)*
3. **Copy all as CSV** — converts entire model to CSV via `QtUtils::AbstractItemModelToCSV`

### Double-Click Behavior
| Column Clicked     | Navigation Target                          |
|--------------------|--------------------------------------------|
| ENTRY or ENTRY_LABEL | goToInDisassembler(entry address, true)  |
| SP                 | goToInMemoryView(sp address, true)         |
| PC or default      | goToInDisassembler(pc address, true)       |

### Hidden Features / Notes
- Stack only shows frames for **one thread** (the currently running one). Comment: *"Hopefully in the near future we can get a stack frame for each thread"*
- No sorting support (unlike ThreadView)
- No proxy model (direct model to view)

---

## ThreadView

### Class
- `ThreadView` extends `DebuggerView` with `MONOSPACE_FONT` flag
- Uses `Ui::ThreadView` (Qt Designer `.ui` file)
- Owns `ThreadModel*` and `QSortFilterProxyModel*`

### UI Elements
- `m_ui.threadList` — QTableView displaying threads
  - Context menu policy: `Qt::CustomContextMenu`
  - **Sorting enabled** via `QSortFilterProxyModel`
  - Default sort: `ID` column ascending
  - Horizontal header: sections movable
  - Vertical header: ResizeToContents
  - Column resize modes set per-column from `ThreadModel::HeaderResizeModes`

### ThreadModel Columns (7)
| Column    | Header Label  | Resize Mode     | Display Data                          | UserRole Data         |
|-----------|---------------|-----------------|---------------------------------------|-----------------------|
| ID        | "ID"          | ResizeToContents| `thread->TID()`                       | TID                   |
| PC        | "PC"          | ResizeToContents| hex16 PC (if RUN: cpu.getPC())        | raw u32 PC            |
| ENTRY     | "ENTRY"       | ResizeToContents| hex16 entry point                     | raw u32 entry         |
| PRIORITY  | "PRIORITY"    | ResizeToContents| `thread->Priority()` as number        | priority u32          |
| STATE     | "STATE"       | Stretch         | localized state string                | state enum as u32     |
| WAIT_TYPE | "WAIT TYPE"   | Stretch         | localized wait type string            | wait enum as u32      |
| WAIT_ID   | "WAIT ID"     | Stretch         | hex16 wait id                         | waitId as string      |

### Thread States (ThreadStatus enum → display strings)
| Enum               | Display String   |
|--------------------|------------------|
| THS_BAD            | "BAD"            |
| THS_RUN            | "RUN"            |
| THS_READY          | "READY"          |
| THS_WAIT           | "WAIT"           |
| THS_SUSPEND        | "SUSPEND"        |
| THS_WAIT_SUSPEND   | "WAIT SUSPEND"   |
| THS_DORMANT        | "DORMANT"        |
| *(unknown)*        | "INVALID"        |

### Wait States (WaitState enum → display strings)
| Enum      | Display String |
|-----------|----------------|
| NONE      | "NONE"         |
| SEMA      | "SEMAPHORE"    |
| SLEEP     | "SLEEP"        |
| DELAY     | "DELAY"        |
| EVENTFLAG | "EVENTFLAG"    |
| MBOX      | "MBOX"         |
| VPOOL     | "VPOOL"        |
| FIXPOOL   | "FIXPOOL"      |
| *(unknown)* | "INVALID"    |

### Data Source
- `m_cpu.GetThreadList()` returns `std::vector<std::unique_ptr<BiosThread>>`
- Refreshed on:
  - `DebuggerEvents::Refresh` (only if VM is NOT paused)
  - `DebuggerEvents::VMUpdate` (always)

### Sort/Filter
- `QSortFilterProxyModel` with `Qt::UserRole` as sort role
- Sort role values: ID=TID number, PC=raw address, PRIORITY=number, STATE=enum u32, WAIT_TYPE=enum u32
- Sorting enabled with click on column headers
- Default sort: ID ascending

### Context Menu Actions
1. **Copy** — copies current cell text (mapped through proxy to source model)
2. *(separator)*
3. **Copy all as CSV** — converts entire proxy model to CSV

### Double-Click Behavior
| Column Clicked | Navigation Target                    |
|----------------|--------------------------------------|
| ENTRY          | goToInDisassembler(entry, true)      |
| PC or default  | goToInDisassembler(pc, true)         |

### Hidden Features / Notes
- Thread PC for RUNNING thread uses `cpu.getPC()` (live PC) instead of stored PC — real-time accuracy
- Proxy model is used only for sorting, not filtering (no filter string exposed)
- No "switch to thread" or "suspend/resume" actions in context menu — view-only
- Both Refresh and VMUpdate events trigger refresh, but Refresh is gated on VM not being paused
