# 38 — Debugger Docking System Analysis

## Overview
PCSX2 uses **KDDockWidgets** (a Qt docking framework) for its debugger window layout system. This provides a fully dockable, tabbable, splittable panel system with per-CPU layouts (EE vs IOP).

## Architecture

### Core Classes
| Class | File | Role |
|-------|------|------|
| `DockManager` | DockManager.h/cpp | Top-level manager: owns all layouts, handles switching, CRUD, menus, lock state |
| `DockLayout` | DockLayout.h/cpp | Single layout: owns widgets, geometry, toolbar state, freeze/thaw lifecycle |
| `DockTables` | DockTables.h/cpp | Static registry of debugger view types + 2 default layouts (R5900, R3000) |
| `DockMenuBar` | DockMenuBar.h | Custom menu bar with layout tab switcher |

### External Dependency
- **KDDockWidgets** — `kddockwidgets/MainWindow.h`, `DockWidget.h`, `LayoutSaver`, `DockRegistry`
- Frontend: `KDDockWidgets::FrontendType::QtWidgets`

---

## Features

### 1. Layout CRUD
| Operation | Method | Details |
|-----------|--------|---------|
| Create (default) | `createLayout(name, cpu, is_default, base_name)` | Creates from R5900/R3000 template |
| Create (blank) | `createLayout(name, cpu, is_default)` | Empty layout |
| Create (clone) | `createLayout(name, cpu, is_default, layout_to_clone)` | Clones geometry, widgets, toolbars |
| Delete | `deleteLayout(index)` | Switches away first, deletes file, adjusts indices |
| Save | `save(index)` / `saveCurrentLayout()` / `saveLayouts()` | JSON to disk with atomic temp-file rename |
| Load | `loadLayouts()` | Scans `*.json` in DebuggerLayouts folder, sorts by last-session index |
| Edit | `editLayoutClicked(index)` | Dialog: rename + change CPU target |
| Reset | `resetLayoutClicked(index)` / `resetDefaultLayouts()` / `resetAllLayouts()` | Recreate from default template |
| Switch | `switchToLayout(index, blink_tab)` | Freeze old → thaw new |
| Switch by CPU | `switchToLayoutWithCPU(cpu, blink_tab)` | Auto-selects layout matching given CPU |
| Reorder | `layoutSwitcherTabMoved(from, to)` | Swap layouts in vector, update files |

### 2. Layout Switcher (Tab Bar)
- Tab bar in menu bar showing all layouts by name
- Tab click → `switchToLayout`
- Tab move (drag reorder) → `layoutSwitcherTabMoved`
- Context menu (right-click tab): **Edit Layout**, **Reset Layout**, **Delete Layout**
- `+` button → `newLayoutClicked` (opens LayoutEditorDialog)
- Tab blink animation via `m_menu_bar->startBlink`

### 3. New Layout Dialog (LayoutEditorDialog)
Three creation modes:
- **DEFAULT_LAYOUT**: Start from R5900 or R3000 template
- **BLANK_LAYOUT**: Empty canvas
- **CLONE_LAYOUT**: Clone current layout's geometry
- Name validator prevents conflicts
- CPU selector (EE/IOP)

### 4. Per-CPU Layouts
- Each layout has a `BreakPointCpu` target: `BREAKPOINT_EE` or `BREAKPOINT_IOP`
- `switchToLayoutWithCPU(cpu)` auto-switches to matching layout
- `cpu()` accessor returns current layout's CPU
- Individual dock widgets can override CPU via `cpu_override` field

### 5. Layout Lock
- `m_layout_locked` (default: `true`)
- **Locked**: Cannot drag dock widgets, cannot close tabs, toolbars immovable (except floating)
- **Unlocked**: Full drag/drop, tab close buttons visible, toolbars movable
- Setting persisted: `Debugger/UserInterface/LayoutLocked`
- Lock button in menu bar
- `updateToolBarLockState()` syncs all toolbars

### 6. Drop Indicators
Two styles configured via `Debugger/UserInterface/DropIndicatorStyle`:
- **Classic** — KDDockWidgets default
- **Segmented** — Alternative with `DockSegmentedDropIndicatorOverlay`
- Also **Minimalistic** alias for Segmented
- Drop indicators inhibited when layout is locked
- Minimum drag distance: `max(QApplication::startDragDistance(), 32)`

### 7. Theme / Visual
- `updateTheme()` — propagates style sheet changes to all debugger views
- Updates KDDockWidgets `QProxyStyle` on all `TabBar` instances
- `DockViewFactory` custom view factory
- `DockSegmentedDropIndicatorOverlay` for custom drop indicator rendering
- `DropIndicators.h` — custom drop indicator overlay

### 8. Autosave
- `QTimer` fires every **60 seconds** → `saveCurrentLayout()`

### 9. Freeze/Thaw Lifecycle
- **Freeze** (`DockLayout::freeze()`):
  1. Saves toolbar state (`QMainWindow::saveState()`)
  2. Serializes dock geometry via `KDDockWidgets::LayoutSaver::serializeLayout()`
  3. Destroys all dock widgets (releases content ownership first)
- **Thaw** (`DockLayout::thaw()`):
  1. Restores toolbar state (or shows default toolbars for base layout)
  2. Restores dock geometry via `KDDockWidgets::LayoutSaver::restoreLayout()`
  3. Validates all widgets were restored; deletes orphans
  4. Falls back to `setupDefaultLayout()` on restore failure

### 10. Dock Widget Factory
- `dockWidgetFactory(name)` — static callback registered with KDDockWidgets
- Called during layout restore to create `DockWidget` wrappers for existing `DebuggerView` widgets
- `dragAboutToStart` — static callback to inhibit drops when locked; always allows floating window movement

### 11. File Format (JSON)
```json
{
  "format": "PCSX2 Debugger User Interface Layout",
  "versionMajor": 2,
  "versionMinor": 0,
  "defaultLayoutHash": 1234567890,
  "name": "R5900",
  "target": "EE",
  "index": 0,
  "isDefault": true,
  "nextId": 42,
  "baseLayout": "R5900",
  "toolbars": "<base64-encoded QMainWindow state>",
  "dockWidgets": [
    {
      "uniqueName": "DisassemblyView-0",
      "id": 0,
      "type": "DisassemblyView",
      "target": "EE",
      // ... widget-specific JSON via toJson()
    }
  ],
  "geometry": { /* KDDockWidgets LayoutSaver JSON */ }
}
```
- Stored in `EmuFolders::DebuggerLayouts/*.json`
- Atomic save: write to `.tmp` then rename to `.json`
- Old file deleted on rename
- `defaultLayoutHash` detects when defaults changed → triggers reset

### 12. Default Layouts (DockTables)
| Layout | CPU | Views |
|--------|-----|-------|
| **R5900** | EE | Disassembly, Memory, Breakpoints, Threads, Stack, Saved Addresses, Globals, Locals, Parameters, Registers, Functions, Memory Search |
| **R3000** | IOP | Same as R5900 but replaces Global/Local/Parameter trees with ModuleView; same group structure |

### 13. Registered Debugger View Types (13 types)
| Type | Display Name | Preferred Location |
|------|-------------|-------------------|
| `BreakpointView` | Breakpoints | BOTTOM_MIDDLE |
| `DisassemblyView` | Disassembly | TOP_RIGHT |
| `FunctionTreeView` | Functions | TOP_LEFT |
| `GlobalVariableTreeView` | Globals | BOTTOM_MIDDLE |
| `LocalVariableTreeView` | Locals | BOTTOM_MIDDLE |
| `MemorySearchView` | Memory Search | TOP_LEFT |
| `MemoryView` | Memory | BOTTOM_MIDDLE |
| `ModuleView` | Modules | BOTTOM_MIDDLE |
| `ParameterVariableTreeView` | Parameters | BOTTOM_MIDDLE |
| `RegisterView` | Registers | TOP_LEFT |
| `SavedAddressesView` | Saved Addresses | BOTTOM_MIDDLE |
| `StackView` | Stack | BOTTOM_MIDDLE |
| `ThreadView` | Threads | BOTTOM_MIDDLE |

### 14. Multi-Instance Views
- Views with `supportsMultipleInstances() == true` can be duplicated
- "Add Another..." submenu in Windows menu lists duplicatable view types
- Each instance gets unique name: `{Type}-{nextId}` (e.g., `MemoryView-5`)
- `createDebuggerView(type)` — creates new instance, inserts at preferred location
- `destroyDebuggerView(unique_name)` — removes instance
- `recreateDebuggerView(unique_name)` — hot-replace widget (preserves name, ID, CPU override, display name, primary state)

### 15. Primary View System
- Each view type has exactly one **primary** instance
- `setPrimaryDebuggerView(widget, is_primary)` — enforces single primary per type
- If removing primary, auto-promotes another instance of same type
- Validated on load: `validatePrimaryDebuggerViews()`

### 16. Windows Menu
- Lists all open debugger views as checkable toggles
- Checked = view visible; unchecked = view destroyed
- Lists destroyed view types (uncheckable) that can be re-created
- Sorted alphabetically with suffix numbers for duplicates

### 17. Tools Menu
- Lists all `QToolBar` children of debugger window
- Checkable toggle for toolbar visibility

### 18. Name Conflict Detection
- `hasNameConflict(name, layout_index)` — case-insensitive comparison of sanitized file names
- Prevents duplicate layout names
- Auto-suffixes loaded layouts with `#2`, `#3`, etc. on conflict

### 19. KDDockWidgets Configuration Flags
- `Flag_HideTitleBarWhenTabsVisible` — hides title bar when tabbed
- `Flag_AlwaysShowTabs` — always show tab bar
- `Flag_AllowReorderTabs` — drag to reorder tabs
- `Flag_TitleBarIsFocusable` — clicking title bar focuses the widget
- `InternalFlag_DisableTranslucency` — for compositing-disabled fallback

### 20. No Layouts State
- When all layouts deleted, shows `NoLayoutsWidget` with "Create Default Layouts" button
- Adds placeholder dock widget titled "No Layouts"
- Button triggers `resetAllLayouts()`

### 21. Unique Name Generation
- Pattern: `{ClassName}-{counter}` (e.g., `MemoryView-0`, `BreakpointView-3`)
- Counter persisted in layout JSON (`nextId`)
- Collision-safe: increments until unique

### 22. Layout Versioning
- `versionMajor` — breaking changes → file rejected
- `versionMinor` — non-breaking changes → file accepted
- `defaultLayoutHash` — detects when default layout definitions changed → `DEFAULT_LAYOUT_HASH_MISMATCH` result → triggers reset

---

## Hidden/Non-Obvious Features

1. **Per-widget CPU override**: Individual dock widgets can target a different CPU than the layout's default (stored as `"target"` per widget in JSON)
2. **Atomic file saves**: Temp file → rename prevents corruption on crash
3. **Layout auto-ordering**: `index` field in JSON preserves tab order across sessions
4. **Floating windows bypass lock**: `dragAboutToStart` allows moving floating windows even when layout is locked
5. **Toolbar toolbar state per layout**: Each layout independently saves/restores toolbar positions
6. **Default layout hash change detection**: Upgrading PCSX2 auto-resets default layouts when their definition changes
7. **Widget recreation on CPU change**: `setCpu()` calls `recreateDebuggerView()` if widget can't switch CPU
8. **First widget auto-primary**: If no primary found during validation, first widget of type becomes primary
9. **Tab close buttons hidden when locked**: `setTabsClosable(!m_layout_locked)`
10. **HACK: Tab size refresh**: Setting tab text to itself forces size recalculation after closable change
11. **Dock widget factory as static callback**: KDDockWidgets calls back into DockManager during layout restore
12. **Drop indicator inhibited on locked drag**: Prevents visual noise when drag is rejected
13. **Start drag distance override**: Minimum 32px to prevent accidental drags
14. **NoLayoutsWidget as emergency recovery**: Prevents completely empty debugger window
