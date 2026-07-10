# Disassembly View — PCSX2 Qt Deep Analysis

## Files Analyzed
- `pcsx2-qt/Debugger/DisassemblyView.h` — Class declaration
- `pcsx2-qt/Debugger/DisassemblyView.cpp` — Full implementation (~750 lines)
- `pcsx2-qt/Debugger/DisassemblyView.ui` — Qt Designer UI layout
- **Note:** `DisassemblyViewColors.h/.cpp` do NOT exist — color logic is inline in DisassemblyView.cpp

---

## Overview
Custom-painted QWidget (`paintEvent`) that renders MIPS disassembly with:
- Monospace font
- Custom row rendering (not a QListView/QTableView)
- Title/header row
- Alternating row backgrounds
- Selection highlighting
- Branch line drawing (visual arrows)
- Breakpoint markers
- Symbol/function coloring

---

## UI Structure

### Layout
- **Title row** (row 0): Column headers — "Location", optional "Bytes", "Instruction"
- **Visible rows** (1..N): Disassembly lines computed from `m_visibleStart` address
- **Row height**: `fontMetrics.height() + 2`
- **Visible rows count**: `(widget_height / rowHeight) - 1` (minus title)

### Display Modes
| Mode | Columns | Toggle |
|------|---------|--------|
| Default | NR | Location | Instruction | — |
| With Bytes | NR | Location | Bytes | Instruction | `m_showInstructionBytes` / Key `I` |

- **NR** column: 2 chars, shows "NR" for no-return functions
- **Location**: 8-char hex address OR elided symbol name
- **Bytes**: 8-char hex opcode (when enabled)
- **Instruction**: mnemonic + args (e.g., `addiu $a0, $sp, 0x10`)
- **Conditional annotation**: `# true` / `# false` for conditional branches at PC
- **PC indicator**: `<--` when address == current PC

---

## Context Menu (Right-Click)

### Copy Submenu
| Action | Shortcut | Description |
|--------|----------|-------------|
| Copy Address | — | Copies hex address(es) to clipboard |
| Copy Instruction Hex | — | Copies raw opcode hex to clipboard |
| Copy Instruction Text | `C` | Copies disassembled text to clipboard |
| Copy Function Name | — | Only shown if selected address is a function start |

### Edit Submenu
| Action | Shortcut | Description |
|--------|----------|-------------|
| Paste Instruction Text | — | Assembles clipboard lines, replaces selected instructions |
| Restore Instruction(s) | — | Only shown if NOPed instructions exist; restores originals |
| Assemble new Instruction(s) | `M` | Opens text input dialog pre-filled with current instruction |
| NOP Instruction(s) | — | Replaces selected instructions with NOP (0x00000000) |

### Execution Submenu
| Action | Shortcut | Description |
|--------|----------|-------------|
| Run to Cursor | — | Adds temp breakpoint at cursor, resumes execution |
| Jump to Cursor | `J` | Sets PC to selected address |
| Toggle Breakpoint | `B` / `Space` | Toggles breakpoint at selected address |
| Follow Branch | `Right` | If instruction is branch/jump, navigates to target |

### Navigation Submenu
| Action | Shortcut | Description |
|--------|----------|-------------|
| Go to Address | `G` | Opens expression dialog, evaluates and navigates |
| Go to PC on Pause | — | Checkable toggle; auto-scroll to PC when VM pauses |
| *(Cross-debugger navigation)* | — | Creates `GoToAddress` events for other debugger tabs |

### Function Submenu
| Action | Description |
|--------|-------------|
| Add Function | Opens `NewFunctionDialog` with default name `func_XXXXXXXX` |
| Rename Function | Opens text dialog to rename existing function |
| Remove Function | Deletes function from symbol database, merges with previous function |
| Stub (NOP) Function | Replaces first 2 instructions with `jr ra; nop` |
| Restore Function | Restores stubbed function's first 2 instructions |

### View Submenu
| Action | Shortcut | Description |
|--------|----------|-------------|
| Show Instruction Bytes | `I` | Checkable toggle; shows/hides opcode hex column |

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Up` | Move selection up 1 instruction |
| `Down` | Move selection down 1 instruction |
| `Page Up` | Move selection up by visible rows |
| `Page Down` | Move selection down by visible rows |
| `Shift+Up/Down` | Extend selection range |
| `G` | Go to Address dialog |
| `J` | Jump to cursor (set PC) |
| `C` | Copy instruction text |
| `B` / `Space` | Toggle breakpoint |
| `M` | Assemble instruction |
| `Right` | Follow branch |
| `Left` | Go to PC |
| `I` | Toggle instruction bytes |

---

## Mouse Interaction

| Action | Behavior |
|--------|----------|
| Left Click | Select instruction at row |
| Shift+Left Click | Extend selection |
| Right Click | Select (if single) + open context menu |
| Double Click | Toggle breakpoint at row |
| Wheel Up/Down | Scroll disassembly by 1 instruction |

---

## Color Coding

### Function Colors (6-color palette)
Functions are colored by `(function_address >> 4) % 6`:

**Light Theme:**
| Index | Color |
|-------|-------|
| 0 | `#FA3434` (red) |
| 1 | `#206b6b` (teal) |
| 2 | `#858534` (olive) |
| 3 | `#378c37` (green) |
| 4 | `#783278` (purple) |
| 5 | `#21214a` (dark blue) |

**Dark Theme:**
| Index | Color |
|-------|-------|
| 0 | `#e05555` (red) |
| 1 | `#55e0e0` (cyan) |
| 2 | `#e8e855` (yellow) |
| 3 | `#55e055` (green) |
| 4 | `#e055e0` (magenta) |
| 5 | `#C2C2F5` (lavender) |

### Other Colors
| Element | Color |
|---------|-------|
| Title row | `palette().button()` lighter + desaturated |
| Selection | `palette().highlight()` |
| Alternating rows | `palette().base()` / `palette().alternateBase()` |
| Breakpoint (enabled) | Green `■` (U+25A0) |
| Breakpoint (disabled) | `☒` (U+2612) |
| Branch line (selected) | `#FF257AFA` (blue) |
| Branch line (normal) | `#FFFF3020` (red) |
| Border | `palette().shadow()` |
| Invalid address | "NOT VALID ADDRESS" text |
| Default (no function) | `palette().text()` |

---

## Branch Line Drawing

Visual arrows drawn on right side of view for branch/jump instructions:
- Max 3 lines (with bytes shown) or 5 lines (without bytes)
- Lines positioned at right edge, offset by count × 10px
- **LINE_UP**: Arrow pointing upward (backward branch)
- **LINE_DOWN**: Arrow pointing downward (forward branch)
- Handles partially visible branches (start/end above/below viewport)
- Blue highlight when branch involves selected instruction

---

## Breakpoint Markers
- Painted as first character on row (overlays NR column area)
- **Enabled**: Green filled square `■`
- **Disabled**: Crossed box `☒`
- Temporary breakpoints (from Run to Cursor) are NOT shown

---

## Data Management

### NOP/Restore System
- `m_nopedInstructions`: `map<address, original_value>` — stores original instructions before NOP
- `setInstructions(start, end, value)`: Writes to CPU memory on CPU thread, stores originals
- `AddressCanRestore()`: Checks if any selected address has stored originals
- Restore: Writes originals back, removes from map

### Stub/Restore Function System
- `m_stubbedFunctions`: `map<address, tuple<instr1, instr2>>` — stores first 2 instructions
- Stub: Writes `jr ra` (0x03E00008) + `nop` (0x00000000)
- Restore: Writes back stored instructions

### Serialization (toJson/fromJson)
- `startAddress`: Visible start address
- `goToPCOnPause`: Auto-scroll on pause toggle
- `showInstructionBytes`: Bytes column toggle

---

## Events

### Receives
| Event | Handler |
|-------|---------|
| `Refresh` | Calls `update()` to repaint |
| `GoToAddress` | Navigates to address, optionally switches tab |
| `onVMActuallyPaused` | Auto-scrolls to PC if `m_goToProgramCounterOnPause` |

### Emits
| Event | Context |
|-------|---------|
| `GoToAddress` | Cross-debugger navigation from context menu |
| `VMUpdate` | After instruction modification (NOP, assemble, paste, stub, restore) |

---

## Implementation Details for Slint Port

1. **Custom painting**: This is a fully custom-painted widget — NOT a list/table. In Slint, this would be a Canvas or custom component with manual drawing.

2. **CPU thread safety**: All memory reads/writes go through `Host::RunOnCPUThread`. The Rust backend must handle this.

3. **Expression evaluation**: `contextGoToAddress` evaluates arbitrary expressions (e.g., `pc+0x100`, symbol names). Needs expression parser.

4. **Cross-view communication**: Uses `DebuggerEvents::GoToAddress` with filter (NONE, DISASSEMBLER, MEMORY, etc.) to navigate between debugger tabs.

5. **Symbol integration**: Heavily uses `SymbolGuardian` for function names, symbols, no-return detection. Needs symbol database access.

6. **Selection model**: Supports single selection and range selection (Shift+click). Start/end address tracking.

7. **Auto-scroll**: Scrolls view when selection moves past visible area.

8. **Font**: Uses `MONOSPACE_FONT` constant from DebuggerView base class.
