# RegisterView — Complete Feature Analysis

**Source**: `pcsx2-qt/Debugger/RegisterView.h`, `RegisterView.cpp`, `RegisterView.ui`  
**Parent class**: `DebuggerView` (which extends `QWidget`)

---

## Register Categories (EE)

From `DebugInterface.h` enum:

| Index | Category | Register Size | Description |
|-------|----------|---------------|-------------|
| 0 | `EECAT_GPR` | 128-bit | General Purpose Registers (ee.r.zero … ee.r.ra, plus `hi`, `lo`) |
| 1 | `EECAT_CP0` | 32-bit | Coprocessor 0 (Status, Cause, EPC, BadVAddr, etc.) |
| 2 | `EECAT_FPR` | 32-bit (displayed as float optionally) | Floating Point Registers (f0–f31) |
| 3 | `EECAT_FCR` | 32-bit | FP Control Register (FCR0, FCR31) |
| 4 | `EECAT_VU0F` | 128-bit | VU0 Float registers (VF00–VF31) |
| 5 | `EECAT_VU0I` | 32-bit | VU0 Integer registers (VI00–VI15) |
| 6 | `EECAT_GSPRIV` | GS privileged registers | GIF_STAT, GIF_CNT, etc. |

## Register Categories (IOP)

| Index | Category | Description |
|-------|----------|-------------|
| 0 | `IOPCAT_GPR` | IOP General Purpose (zero, at, v0–v1, a0–a3, t0–t9, s0–s7, k0–k1, gp, sp, fp, ra, hi, lo) |

---

## UI Structure

### Qt Designer Form (RegisterView.ui)
- **QVBoxLayout** (spacing=0, margins=0)
  - **QTabBar** `registerTabs` — tab bar at top, populated dynamically with category names
  - **QSpacerItem** — vertical spacer fills remaining space

### Custom Paint Rendering
The register list is **NOT a QListView or QTableWidget** — it's entirely custom-painted via `QPainter` in `paintEvent()`. This means:
- No standard model/view; all rendering is manual
- Rows are drawn as alternating colored rectangles (base/alternateBase palette)
- Values are drawn as text at calculated pixel positions
- Selection highlighting uses `palette().highlight()` color

---

## Features

### 1. Tabbed Register Categories
- **Tab bar** at top of view, one tab per `getRegisterCategoryCount()`
- Tab names from `getRegisterCategoryName(i)` — e.g., "EE", "COP0", "FPR", "FCR", "VU0f", "VU0i", "GS"
- Tab change resets `m_rowStart = 0` (scrolls to top)
- Connected to `update()` to trigger repaint

### 2. Register Display Rendering
- **Row height**: `fontMetrics().height() + 2` pixels
- **Visible rows**: calculated from render height / row height
- **Alternating row colors**: `palette().base()` and `palette().alternateBase()`
- **Register name**: drawn left-aligned, 1 char-width from left edge
- **Value positioning**: dynamic calculation of longest register name to determine value X position

### 3. 128-bit Register Display (GPR, VU0F)
- **4 columns** for 32-bit segments (W, Z, Y, X for VU0F)
- Columns equally divided across available width
- **Header row** drawn for VU0F showing W/Z/Y/X labels with highlight background
- Segment order: `_u32[3]` (W/high), `_u32[2]` (Z), `_u32[1]` (Y), `_u32[0]` (X/low)
- Selected segment highlighted with `palette().highlight()` color

### 4. 64-bit Register Display (FPR non-float mode)
- Displays `reg.lo` as 16-digit hex

### 5. 32-bit Register Display (CP0, FCR, VU0I, GS)
- Displays `_u32[0]` as 8-digit hex

### 6. Float Display Mode (FPR)
- **Toggle**: context menu "Show as Float" checkbox
- Renders `std::bit_cast<float>(_u32[0])` via `QString::number()`
- Persisted in JSON state (`showFPRFloat`)

### 7. Float Display Mode (VU0F)
- **Toggle**: context menu "Show as Float" checkbox
- Each of 4 segments rendered as float independently
- `std::bit_cast<float>(_u32[3 - column])` per segment
- Persisted in JSON state (`showVU0FFloat`)

### 8. Mouse Selection
- **Single click**: selects register row (calculates row from Y position)
- **128-bit registers**: also selects field segment based on X position click
- Uses `inRange()` lambda to detect which of 4 field columns was clicked
- Stores `m_selectedRow` (register index) and `m_selected128Field` (0–3)

### 9. Mouse Double-Click → Edit Value
- **Double-click on 128-bit register**: opens "Change Segment" dialog
- **Double-click on non-128 register**: opens "Change Value" dialog
- Guard: only works if `cpu().isAlive()` and `m_selectedRow <= m_rowEnd`

### 10. Mouse Wheel Scrolling
- **Wheel up**: `m_rowStart -= 1` (if `m_rowStart > 0`)
- **Wheel down**: `m_rowStart += 1` (if `m_rowEnd < registerCount`)
- One row per wheel tick

### 11. Context Menu Actions

#### For FPR category:
| Action | Behavior |
|--------|----------|
| ✅ Show as Float | Toggle `m_showFPRFloat`, repaint |

#### For VU0F category:
| Action | Behavior |
|--------|----------|
| ✅ Show as Float | Toggle `m_showVU0FFloat`, repaint |

#### For 128-bit registers (GPR, VU0F):
| Action | Behavior |
|--------|----------|
| Copy Top Half | Copies `val.hi` (upper 64 bits) as 16-char hex to clipboard |
| Copy Bottom Half | Copies `val.lo` (lower 64 bits) as 16-char hex to clipboard |
| Copy Segment | Copies selected 32-bit segment; if float mode, copies as float string |
| Change Top Half | Dialog to edit upper 64 bits (hex input) |
| Change Bottom Half | Dialog to edit lower 64 bits (hex input) |
| Change Segment | Dialog to edit selected 32-bit segment; float mode accepts float input |

#### For non-128 registers (CP0, FCR, VU0I, GS, FPR):
| Action | Behavior |
|--------|----------|
| Copy Value | Copies register value; if float mode (FPR), copies as float string |
| Change Value | Dialog to edit value (hex input); float mode accepts float input |

#### Always present:
| Action | Behavior |
|--------|----------|
| Go to Address in Disassembly | Uses register value as address, creates `GoToAddress` event → navigates DisassemblyView |

### 12. Value Change Dialog (`fetchNewValue`)
- Uses `AsyncDialogs::getText()` — async non-blocking input dialog
- **Title**: "Change {register_name}"
- **Hex mode**: parses input as base-16 (`toULongLong(&ok, 16)`)
- **Float mode** (when `CAT_SHOW_FLOAT && segment`): parses as float, bit-cast to u32
- **Validation**: shows warning dialog on invalid input
- Calls `cpu().setRegister()` then broadcasts `VMUpdate` event to refresh all debugger views

### 13. Go-to-Address from Register Value
- Takes register value (selected segment for 128-bit, `_u32[0]` for others)
- Validates with `cpu().isValidAddress(addr)`
- If invalid, shows warning dialog: "This register holds an invalid address."
- Returns `DebuggerEvents::GoToAddress` consumed by parent DebuggerWindow to navigate disassembly

### 14. JSON State Persistence
```json
{
  "showVU0FFloat": true/false,
  "showFPRFloat": true/false
}
```
- Saved via `toJson()`, restored via `fromJson()`

### 15. Refresh Event Handling
- Listens for `DebuggerEvents::Refresh` → calls `update()` to repaint with current register values

---

## Internal State

| Field | Type | Purpose |
|-------|------|---------|
| `m_renderStart` | `QPoint` | Top-left of register rendering area (below tab bar) |
| `m_rowStart` | `s32` | First visible register index (scroll offset) |
| `m_rowEnd` | `s32` | Last visible register index |
| `m_rowHeight` | `s32` | Pixel height of one register row |
| `m_fieldStartX[4]` | `s32[4]` | X pixel positions of 4 segments (for 128-bit) |
| `m_fieldWidth` | `s32` | Width in pixels of one segment column |
| `m_selectedRow` | `s32` | Currently selected register index |
| `m_selected128Field` | `s32` | Selected segment (0–3) for 128-bit registers |
| `m_showVU0FFloat` | `bool` | Display VU0F registers as float |
| `m_showFPRFloat` | `bool` | Display FPR registers as float |

---

## Signals & Slots

| Signal/Slot | Source | Purpose |
|-------------|--------|---------|
| `customContextMenuRequested(QPoint)` | QWidget built-in | Opens context menu |
| `registerTabs::currentChanged(int)` | QTabBar | Resets scroll to top on tab change |
| `Refresh` event | DebuggerView event bus | Repaints register values |

---

## Summary for UI Implementation

### Settings Page Design
The RegisterView is a **debugger tool**, not a settings page. It would be a **standalone debugger panel** in a future Debugger tab. Key requirements:

1. **Tab bar** at top with register categories (7 for EE, 1 for IOP)
2. **Scrollable register list** — custom painted, alternating row colors
3. **Selection** — click to select row + segment
4. **Edit** — double-click or context menu → input dialog (hex/float)
5. **Clipboard** — copy value/top/bottom/segment
6. **Go-to** — navigate to address held in register
7. **Float toggle** — per-category, persisted

### Slint Implementation Notes
- Tab bar → `TabBar` or custom horizontal button row
- Register list → `ListView` with custom row component (name + value columns)
- 128-bit display → 4-column value layout with W/Z/Y/X headers
- Context menu → long-press or action button row
- Value editing → modal dialog with text input
- Float toggle → checkbox in each category tab
