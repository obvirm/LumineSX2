# PCSX2 Qt: DebuggerWindow Deep Analysis

## Class Structure
- Inherits from `KDDockWidgets::QtWidgets::MainWindow` (not plain QMainWindow!)
- Singleton pattern via `g_debugger_window` global pointer
- Uses KDDockWidgets docking system for flexible window layout

## Singleton Lifecycle
```
getInstance()     → creates if not exists, returns pointer
createInstance()  → configures DockManager, creates new window
destroyInstance() → closes and schedules deletion
shouldShowOnStartup() → reads "Debugger/UserInterface" → "ShowOnStartup" setting (default: false)
```

## Constructor Initialization Order
1. KDDockWidgets::MainWindow base init (named "DebuggerWindow")
2. Create DockManager
3. setupUi (from .ui file)
4. setupDefaultToolBarState()
5. setupFonts()
6. restoreWindowGeometry()
7. m_dock_manager->loadLayouts()
8. Connect ALL signals
9. Check current VM state (Starting/Paused/Resumed/Stopped)
10. m_dock_manager->switchToLayout(0) → load first layout
11. Setup menu bar: menuBar() → setMenuWidget(dockManager->createMenuBar())
12. updateTheme()
13. RunOnCPUThread → R5900SymbolImporter.OnDebuggerOpened()
14. updateFromSettings()

## Toolbar Actions (from .ui file)

### File Menu
| Action | Signal | Handler |
|--------|--------|---------|
| actionAnalyse | triggered | onAnalyse() → opens AnalysisOptionsDialog |
| actionSettings | triggered | onSettings() → g_main_window->doSettings("Debug") |
| actionGameSettings | triggered | onGameSettings() → g_main_window->doGameSettings("Debug") |
| actionClose | triggered | close() |

### View Menu
| Action | Signal | Handler |
|--------|--------|---------|
| actionOnTop | triggered(bool) | toggle Qt::WindowStaysOnTopHint |

### Debug Menu
| Action | Signal | Handler |
|--------|--------|---------|
| actionRun | triggered | onRunPause() |
| actionStepInto | triggered | onStepInto() |
| actionStepOver | triggered | onStepOver() |
| actionStepOut | triggered | onStepOut() |
| actionShutDown | triggered | g_emu_thread->shutdownVM(false) |
| actionReset | triggered | g_emu_thread->resetVM() |

### Layout Menu
| Action | Signal | Handler |
|--------|--------|---------|
| actionResetAllLayouts | triggered | confirmation → dockManager->resetAllLayouts() |
| actionResetDefaultLayouts | triggered | confirmation → dockManager->resetDefaultLayouts() |

### Font Actions
| Action | Signal | Handler |
|--------|--------|---------|
| actionIncreaseFontSize | triggered | font_size++, max 30 |
| actionDecreaseFontSize | triggered | font_size--, min 5 |
| actionResetFontSize | triggered | reset to QApplication default |

## Dynamic Menus (populated on show)
- **menuTools** → `aboutToShow` → `m_dock_manager->createToolsMenu(menu)`
  - Dynamically populated by DockManager with available debug tool windows
- **menuWindows** → `aboutToShow` → `m_dock_manager->createWindowsMenu(menu)`
  - Dynamically populated with open dock widget windows

## VM State Management

### onVMStarting()
Enables: Run, StepInto, StepOver, StepOut, Analyse, GameSettings, ShutDown, Reset

### onVMPaused()
- Run button → "Run" icon (play-line)
- Enables: StepInto, StepOver, StepOut
- **Breakpoint handling:**
  - If breakpoint triggered: switch to layout with that CPU, blink tab
  - Get CPU type (EE or IOP) from breakpoint
  - Call `m_dock_manager->switchToLayoutWithCPU(cpu_type, blink_tab)`
  - Clear temporary breakpoints
  - Set skip-first for both EE and IOP to avoid re-triggering
- If not core-paused: emit `onVMActuallyPaused()` signal

### onVMResumed()
- Run button → "Pause" icon
- Disables: StepInto, StepOver, StepOut

### onVMStopped()
Disables: Run, StepInto, StepOver, StepOut, Analyse, GameSettings, ShutDown, Reset

## Stepping Logic (Hidden Advanced Features!)

### Step Into
1. Skip current breakpoint at PC
2. Get opcode info via `MIPSAnalyst::GetOpcodeInfo(cpu, pc)`
3. Calculate next instruction:
   - Default: `pc + 4`
   - **Non-conditional branch**: jump to `branchTarget`
   - **Conditional branch, met**: jump to `branchTarget`
   - **Conditional branch, not met**: `pc + 8` (skip delay slot)
   - **Syscall**: jump to `branchTarget` (always taken)
4. Set temporary breakpoint at calculated address
5. Resume CPU

### Step Over
1. Get opcode info
2. Calculate next instruction:
   - Default: `pc + 4`
   - **Non-conditional linked branch** (jal/jalr): `pc + 8` (skip call + delay slot)
   - **Non-conditional non-linked branch** (j): jump to `branchTarget`
   - **Conditional, met**: jump to `branchTarget`
   - **Conditional, not met**: `pc + 8`
3. Set temporary breakpoint
4. Resume CPU

### Step Out
1. Skip current breakpoint
2. Walk the call stack using `MipsStackWalk::Walk()`
3. Find current running thread (Status == THS_RUN)
4. Get stack frames from PC, return address (reg 31), stack pointer (reg 29), entry point
5. Need at least 2 frames
6. Set breakpoint at frame[1].pc (caller)
7. Resume CPU

## Docking System (KDDockWidgets)

### Architecture
- Uses **KDDockWidgets** library (not Qt's built-in QDockWidget)
- `DockManager` manages all docking operations
- Supports **multiple layout presets** (switched by index)
- Can **switch to layout by CPU type** (EE/IOP)
- Supports **tab blinking** when breakpoint hits

### Layout Management
- `loadLayouts()` → load saved layouts
- `switchToLayout(index)` → switch to preset layout
- `switchToLayoutWithCPU(cpu_type, blink_tab)` → auto-switch + visual feedback
- `resetAllLayouts()` → confirmation → reset
- `resetDefaultLayouts()` → confirmation → reset
- `saveCurrentLayout()` → save on close

### Toolbars
- Initially ALL hidden (to save default state)
- `setupDefaultToolBarState()` → save state, then connect `topLevelChanged` to `DockManager::updateToolBarLockState`
- `clearToolBarState()` → restore default state

### Dynamic Menu Generation
- `createToolsMenu(QMenu*)` → creates tool window toggle actions
- `createWindowsMenu(QMenu*)` → creates window visibility actions
- `createMenuBar(QMenuBar*)` → creates full menu bar with layout tabs

## Font System
- Stored in settings: `"Debugger/UserInterface"` → `"FontSize"`
- Range: 5pt to 30pt
- Default: QApplication::font().pointSize()
- Applied via stylesheet: `font-size: {size}pt;`
- **HACK**: Calls setStyleSheet twice for default font size (performance workaround)

## Theme Management
- Detects recursive StyleChange events (m_is_updating_theme flag)
- Updates stylesheet with font size
- Propagates to dockManager->updateTheme()
- Listens for PaletteChange and StyleChange events

## Window Geometry
- Saved/Restored via `"Debugger/UserInterface"` → `"WindowGeometry"`
- Base64 encoded QByteArray
- Controlled by `"SaveWindowGeometry"` setting (default: true)

## Refresh Timer
- Setting: `"Debugger/UserInterface"` → `"RefreshInterval"` (default: 1000ms)
- Clamped to 10ms - 100000ms
- Broadcasts `DebuggerEvents::Refresh()` to all DebuggerViews

## Signals
| Signal | When |
|--------|------|
| onVMActuallyPaused() | Only when pause wasn't breakpoint-triggered |

## Settings Paths
| Setting Key | Default | Purpose |
|-------------|---------|---------|
| Debugger/UserInterface → ShowOnStartup | false | Auto-open debugger |
| Debugger/UserInterface → FontSize | system | Font size in pt |
| Debugger/UserInterface → WindowGeometry | - | Saved geometry |
| Debugger/UserInterface → SaveWindowGeometry | true | Whether to save |
| Debugger/UserInterface → RefreshInterval | 1000 | Refresh timer ms |

## Hidden/Advanced Features
1. **Tab blinking on breakpoint**: DockManager can make layout tabs blink when breakpoint hits
2. **CPU-aware layout switching**: Auto-switches to correct CPU layout on breakpoint
3. **Breakpoint skip-first**: Automatically skips breakpoints when stepping
4. **Stack walking**: Step Out uses MIPS stack walker to find caller
5. **Opcode analysis**: Step Into/Over uses MIPSAnalyst for branch detection
6. **Conditional branch handling**: Steps correctly handle branch delay slots
7. **Syscall detection**: Steps through syscalls correctly
8. **Tool menu dynamic population**: Tools menu populated by DockManager based on available widgets
9. **Window persistence**: Full geometry + layout save/restore
10. **Font size persistence**: Saved per-session with min/max bounds
11. **Stay on top toggle**: Window can be pinned above other windows
12. **Dual CPU support**: EE and IOP with separate layouts
13. **Symbol importer lifecycle**: Notifies when debugger opens/closes

## Missing Features (need DockManager/DebuggerView analysis)
- [ ] Full list of available debug tool windows
- [ ] Layout preset management
- [ ] DebuggerView event broadcasting system
- [ ] Breakpoint model/view architecture
