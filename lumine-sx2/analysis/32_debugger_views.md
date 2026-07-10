# 32 - Debugger View Base Class & Event System

## Source Files
- `pcsx2-qt/Debugger/DebuggerView.h`
- `pcsx2-qt/Debugger/DebuggerView.cpp`
- `pcsx2-qt/Debugger/DebuggerEvents.h`

---

## 1. DebuggerViewParameters (Constructor Config)

| Field | Type | Default | Purpose |
|-------|------|---------|---------|
| `unique_name` | `QString` | — | KDDockWidgets identifier, unique per layout |
| `id` | `u64` | `0` | Sorting ID for views with same display name |
| `cpu` | `DebugInterface*` | `nullptr` | Default CPU context for the view |
| `cpu_override` | `optional<BreakPointCpu>` | empty | Per-view CPU override (EE, IOP, VU0, VU1) |
| `parent` | `QWidget*` | `nullptr` | Qt parent widget |

---

## 2. DebuggerView Class (Base for All Debugger Dock Widgets)

### 2.1 Flags Enum
| Flag | Bit | Effect |
|------|-----|--------|
| `NO_DEBUGGER_FLAGS` | `0` | No special behavior |
| `DISALLOW_MULTIPLE_INSTANCES` | `1<<0` | User can only open one dock of this type |
| `MONOSPACE_FONT` | `1<<1` | Applies platform-specific monospace font stylesheet |

### 2.2 Properties

| Property | Type | Access | Purpose |
|----------|------|--------|---------|
| `uniqueName` | `QString` | read | KDDockWidgets ID |
| `id` | `u64` | read | Sort order |
| `displayName` | `QString` | read | Full name with suffix + CPU tag |
| `displayNameWithoutSuffix` | `QString` | read | Translated base name only |
| `customDisplayName` | `QString` | read/write | User-set name (truncated to `MAX_DOCK_WIDGET_NAME_SIZE`) |
| `isPrimary` | `bool` | read/write | Primary views handle events first |
| `cpu` | `DebugInterface&` | read | Effective CPU (override > layout default) |
| `cpuOverride` | `optional<BreakPointCpu>` | read/write | Per-view CPU binding |
| `displayNameSuffixNumber` | `optional<int>` | read/write | Disambiguator for duplicate names |

### 2.3 Display Name Logic
```
displayName = displayNameWithoutSuffix
if suffix_number exists:
    displayName = "{name} #{number}"
if cpu_override exists:
    displayName = "{name} ({cpu_name})"
```

### 2.4 Custom Display Name
- Max length: `DockUtils::MAX_DOCK_WIDGET_NAME_SIZE`
- `setCustomDisplayName()` returns `false` if exceeded
- Custom name overrides translated name in `retranslateDisplayName()`

### 2.5 CPU Binding
- **`cpu()`**: Returns `DebugInterface::get(*m_cpu_override)` if override set, else `*m_cpu`
- **`setCpu(new_cpu)`**: Changes layout-level CPU. Returns `false` if CPU type changed (requires recreation)
- **`setCpuOverride(new_cpu)`**: Changes per-view CPU. Returns `false` if CPU type changed

### 2.6 Stylesheet
- `MONOSPACE_FONT` flag → platform-specific monospace font:
  - Windows: `Lucida Console`
  - macOS: `Monaco`
  - Linux: `Monospace`

### 2.7 Instance Control
- `supportsMultipleInstances()` = `!(flags & DISALLOW_MULTIPLE_INSTANCES)`
- Controlled by `DockTables::DEBUGGER_VIEWS` registry

---

## 3. Event System (DebuggerEvents namespace)

### 3.1 Event Types

| Event | Fields | Action String | Purpose |
|-------|--------|---------------|---------|
| `Refresh` | (none) | — | Sent on creation + periodic broadcast |
| `GoToAddress` | `address: u32`, `filter: Filter`, `switch_to_tab: bool` | "Go to in %1" / "Go to in..." | Navigate to address in disasm or memory |
| `VMUpdate` | (none) | — | VM state changed (pause/resume) |
| `AddToSavedAddresses` | `address: u32`, `switch_to_tab: bool` | "Add to %1" / "Add to..." | Add address to bookmarks list |

### 3.2 GoToAddress Filter Enum
| Value | Meaning |
|-------|---------|
| `NONE` | Any view can handle |
| `DISASSEMBLER` | Only disassembly view handles |
| `MEMORY_VIEW` | Only memory view handles |

### 3.3 Event Dispatch Model
- **`sendEvent(event)`**: Sends to first handler that returns `true`. **Primary views first**, then non-primary.
- **`broadcastEvent(event)`**: Sends to ALL views, no short-circuit.
- **Thread safety**: If called off UI thread, marshals to UI thread via `QtHost::RunOnUIThread`.

### 3.4 Event Registration
```cpp
// Lambda-based
receiveEvent<GoToAddress>([](const GoToAddress& e) -> bool { ... });

// Member function-based
receiveEvent<GoToAddress>(&MyView::handleGoTo);
```
- Uses `typeid(Event).name()` as key in `std::multimap`
- Multiple handlers per event type supported (multimap)

### 3.5 Event Context Menu Generation
- `createEventActions<Event>(menu, event_func, skip_self, max_top_level_actions)`
- Finds all views that accept the event type
- Sorts by display name, then suffix number
- If receivers > `max_top_level_actions` (default 5): creates overflow submenu
- `skip_self = true` by default: excludes sender from receiver list
- Each action triggers `receiver->handleEvent(event)` on click

---

## 4. JSON Persistence

### 4.1 toJson()
```json
{
  "customDisplayName": "My Custom Name",
  "isPrimary": true
}
```

### 4.2 fromJson()
- Reads `customDisplayName` (string, truncated to max size)
- Reads `isPrimary` (bool)
- Returns `true` on success

---

## 5. Static Helper Methods

| Method | Purpose |
|--------|---------|
| `goToInDisassembler(address, switch_to_tab)` | Sends `GoToAddress` with `DISASSEMBLER` filter |
| `goToInMemoryView(address, switch_to_tab)` | Sends `GoToAddress` with `MEMORY_VIEW` filter |
| `switchToThisTab()` | Activates this dock widget in the DockManager |

---

## 6. Event Dispatch Flow

```
sendEvent(event)
  ├─ if off UI thread → marshal to UI thread
  ├─ iterate all views (primary first)
  │   ├─ primary view handles? → return
  │   └─ non-primary handles? → return
  └─ no handler found → silently dropped

broadcastEvent(event)
  ├─ if off UI thread → marshal to UI thread
  └─ iterate ALL views → call handleEvent on each
```

---

## 7. Features for Slint UI Implementation

| Feature | Priority | Notes |
|---------|----------|-------|
| View base class with CPU binding | HIGH | All debugger views need CPU context |
| Event system (send/broadcast/receive) | HIGH | Core inter-view communication |
| Primary view preference | HIGH | Events go to primary view first |
| JSON save/restore of view config | MEDIUM | Layout persistence |
| Custom display name | LOW | Nice-to-have for multi-instance views |
| Context menu event actions | MEDIUM | Right-click → "Go to in Disassembler" |
| Monospace font stylesheet | HIGH | Essential for code display |
| Multiple instance control | MEDIUM | Some views can only have one instance |
| Display name suffix disambiguation | LOW | For multi-instance views |

---

## 8. Hidden/Advanced Features

1. **Thread-safe event dispatch**: Events from emulator thread auto-marshaled to UI thread
2. **Primary view priority**: Primary views get first chance to handle events
3. **Event overflow menus**: Context menus auto-create submenus when >5 receivers
4. **CPU type change detection**: `setCpu()`/`setCpuOverride()` return `false` if CPU type changed, requiring widget recreation
5. **Cross-platform monospace**: Auto-selects `Lucida Console` (Win), `Monaco` (Mac), `Monospace` (Linux)
6. **Display name localization**: Uses `QCoreApplication::translate()` with `DebuggerView` context
7. **Dock widget name size limit**: `MAX_DOCK_WIDGET_NAME_SIZE` enforced on custom names
