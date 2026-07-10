# 46 — DEV9 Dialogs (DNS Host Dialog + HDD Creation Dialog)

## Source Files
- `pcsx2-qt/Settings/DEV9DnsHostDialog.h/.cpp`
- `pcsx2-qt/Settings/HddCreateQt.h/.cpp`
- `pcsx2-qt/Settings/DEV9UiCommon.h/.cpp` (supporting types)

---

## 1. DEV9DnsHostDialog

**Purpose:** Modal dialog for selecting/filtering DNS host entries for DEV9 network adapter emulation. Used to import/export DNS host override lists.

### UI Elements
| Element | Type | Description |
|---------|------|-------------|
| `hostList` | QTableView | Displays DNS host entries with 5 columns |
| `btnOK` | QPushButton | Accept dialog |
| `btnCancel` | QPushButton | Reject dialog |

### Table Model (QStandardItemModel — 5 columns)
| Column | Header | Editable | Checkable | Notes |
|--------|--------|----------|-----------|-------|
| 0 | "Selected" | No | Yes (checkbox) | Pre-checked for all entries |
| 1 | "Name" | Disabled | No | Entry description (`HostEntryUi.Desc`) |
| 2 | "Hostname" | Disabled | No | URL (`HostEntryUi.Url`) |
| 3 | "Address" | Disabled | No | IP address, uses `IPItemDelegate` for validation |
| 4 | "Enabled" | No | Yes (checkbox) | Reflects `HostEntryUi.Enabled`, disabled (read-only) |

### Column Resizing
- Event filter on `hostList` handles `QEvent::Resize` and `QEvent::Show`
- Calls `QtUtils::ResizeColumnsForTableView` with widths: `{80, -1, 170, 90, 80}`
- `-1` = stretch to fill remaining space (Hostname column)

### Sorting
- Default sort: column 1 (Name), ascending
- Uses `QSortFilterProxyModel` between model and view

### IPItemDelegate (from DEV9UiCommon)
- Custom `QStyledItemDelegate` for column 3 (Address)
- Provides a `QLineEdit` editor
- Validates/restricts IP address input

### Flow
1. Constructor receives `std::vector<HostEntryUi> hosts`
2. Populates table with all entries (all pre-selected)
3. `PromptList()` calls `exec()` (modal)
4. On accept: returns only entries where column 0 is checked
5. On reject: returns `std::nullopt`

### Signals/Callbacks
| Signal/Slot | Trigger | Action |
|-------------|---------|--------|
| `btnOK.clicked` → `onOK()` | Click OK | `accept()` |
| `btnCancel.clicked` → `onCancel()` | Click Cancel | `reject()` |

### HostEntryUi Struct
```cpp
struct HostEntryUi {
    std::string Desc;     // Human-readable name
    std::string Url;      // Hostname/URL
    std::string Address;  // IP address
    bool Enabled;         // Whether entry is active
};
```

---

## 2. HddCreateQt

**Purpose:** Progress dialog for creating HDD image files for DEV9 internal HDD emulation. Wraps the base `HddCreate` class with Qt UI.

### UI Elements
| Element | Type | Description |
|---------|------|-------------|
| `progressDialog` | QProgressDialog | Modal progress bar with cancel button |

### Progress Dialog Config
- **Title:** "HDD Creator"
- **Label:** "Creating HDD file \n {written} / {total} MiB"
- **Cancel button:** "Cancel"
- **Range:** 0 to `reqMiB` (size in MiB)
- **Modality:** `Qt::WindowModal`

### Size Calculation
- `reqMiB = (neededSize + 1024*1024 - 1) / (1024*1024)` — rounds up to nearest MiB
- `neededSize` comes from base `HddCreate` class

### Virtual Method Overrides (from HddCreate base)
| Method | Purpose |
|--------|---------|
| `Init()` | Creates QProgressDialog, calculates MiB size |
| `SetFileProgress(u64 currentSize)` | Updates progress bar + label text; checks for cancellation |
| `SetError()` | Shows QMessageBox warning: "Failed to create HDD image" |
| `Cleanup()` | Deletes progressDialog |

### Error Handling
- On error: `QMessageBox::warning` with title "HDD Creator", message "Failed to create HDD image"
- On cancel: calls `SetCanceled()` (from base class) which stops the creation process

### Flow
1. `HddCreateQt(parent)` constructed with parent widget
2. Base class calls `Init()` → creates progress dialog
3. Base class calls `SetFileProgress()` periodically during file creation
4. If user clicks Cancel → `wasCanceled()` returns true → `SetCanceled()` stops creation
5. On error → `SetError()` shows warning dialog
6. Completion → `Cleanup()` deletes progress dialog

---

## Features Summary for LumineSX2

### DNS Host Dialog Features
- [ ] Modal dialog for DNS host selection
- [ ] 5-column table: Selected (checkbox), Name, Hostname, Address, Enabled
- [ ] Checkboxes for selection (column 0) and enabled state (column 4)
- [ ] IP address input validation via custom delegate
- [ ] Sort by column (default: Name ascending)
- [ ] Auto-resize columns on window resize/show
- [ ] Filter proxy model for sorted display
- [ ] Returns selected subset on accept, nullopt on cancel

### HDD Creation Dialog Features
- [ ] Progress dialog with MiB counter
- [ ] Cancel button (stops creation process)
- [ ] Error dialog on failure
- [ ] Window-modal blocking
- [ ] Size display: "X / Y MiB"

### Hidden/Advanced Features
- **IP validation delegate** — not just display; restricts what can be typed in Address column
- **Selection vs Enabled separation** — user can select entries AND see their enabled state independently
- **Column resize event filter** — responsive layout that adapts to window size
- **Cancel-safe HDD creation** — progress dialog cancellation propagates to base class to stop file I/O
