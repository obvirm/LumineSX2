# GameList Deep Analysis — PCSX2 Qt

## 1. Table Columns (11 total)

| # | Column Enum | Display Name | Default Width | Sort Behavior | Visibility |
|---|-------------|-------------|---------------|---------------|------------|
| 0 | `Column_Type` | "Type" | 55px | By enum value, then title | Shown |
| 1 | `Column_Serial` | "Code" | 85px | Case-insensitive string, then title | Shown |
| 2 | `Column_Title` | "Title" | -1 (stretch) | Locale-sensitive compare via `QtHost::LocaleSensitiveCompare` | Shown (default sort) |
| 3 | `Column_FileTitle` | "File Title" | -1 (stretch) | Case-insensitive string, then title | **Hidden by default** |
| 4 | `Column_CRC` | "CRC" | 75px | Integer compare, then title | **Hidden by default** |
| 5 | `Column_TimePlayed` | "Time Played" | 95px | Integer (seconds), then title | Shown |
| 6 | `Column_LastPlayed` | "Last Played" | 90px | Integer (timestamp), then title | Shown |
| 7 | `Column_Size` | "Size" | 80px | Integer (bytes), then title | Shown |
| 8 | `Column_Region` | "Region" | 60px | By enum value, then title | Shown |
| 9 | `Column_Compatibility` | "Compatibility" | 120px | By enum value, then title | Shown |
| 10 | `Column_Cover` | *(no header)* | N/A | N/A | **Always hidden** in table view |

- **Column lookup by name**: `getColumnIdForName(string_view)` maps "Type", "Code", "Title", etc. to enum.
- **Default sort**: Column_Title, Ascending.
- **Sort persistence**: Saves `SortColumn` (by name string) and `SortDescending` (bool) to `[GameListTableView]`.
- **Header state persistence**: Saves/restores full QHeaderView state as Base64 string in `[GameListTableView]/HeaderState`.
- **Column toggling**: Right-click header → context menu with checkboxes for each column (except Cover).
- **Reset button**: "Reset All Columns" in header context menu restores defaults.
- **Safety**: `ensureMinimumOneColumnVisible()` forces Title column visible if user hides everything.
- **Column reordering**: `setSectionsMovable(true)` — drag-and-drop column reordering in header.

## 2. Supported File Formats

```
.bin/.iso (ISO Disc Images)
.mdf (Media Descriptor File)
.chd (Compressed Hunks of Data)
.cso (Compressed ISO)
.zso (Compressed ISO)
.gz (Gzip Compressed ISO)
```

## 3. View Modes

### Table View (index 0)
- `QTableView` with alternating row colors, no grid, row selection, per-pixel scrolling
- Custom `GameListIconStyleDelegate` on columns 0 (Type), 8 (Region), 9 (Compatibility) to center-align icons
- Icon tinting: selected rows get a 30% alpha highlight overlay on icons, cached by `QPixmapCache`

### Grid/Cover View (index 1)
- `GameListGridListView` (custom `QListView` subclass) in `QListView::IconMode`
- Shows `Column_Cover` as model column
- `Adjust` resize mode, uniform item sizes, horizontal center alignment
- **Zoom**: Ctrl+Wheel → `zoomIn`/`zoomOut` signals → `gridZoomIn()`/`gridZoomOut()` (±0.05 step)
- **Scale range**: 0.1 to 2.0
- **Int slider**: `m_ui.gridScale` QSlider, maps 10-200 int → 0.1-2.0 float
- **Cover titles toggle**: `m_ui.viewGridTitles` button, persisted as `GameListShowCoverTitles`
- **Cover art dimensions**: 350×512px base, spacing 32px, scaled by `m_cover_scale`
- **Size hint**: Width = `350 + 16` × scale, Height = `512 + 16` × scale (+32 if titles shown)

### Empty Widget (index 2)
- Shown when `rowCount() == 0` after refresh completes
- Displays supported formats string
- Buttons: "Add Game Directory" (→ `addGameDirectoryRequested`), "Scan for New Games" (→ `refresh(false, true)`)

## 4. Filtering

### Filter by Type
- `m_ui.filterType` QComboBox, populated from `GameList::EntryType` enum (PS2Disc, PS1Disc, ELF)
- Index 0 = "All" → `EntryType::Count` (no filter)

### Filter by Region
- `m_ui.filterRegion` QComboBox, populated from `GameList::Region` enum with flag icons
- Index 0 = "All" → `Region::Count` (no filter)

### Filter by Text (Search)
- `m_ui.searchText` QLineEdit
- Matches against: `entry->path`, `entry->serial`, `entry->title`, `entry->title_en` (case-insensitive)
- **Keyboard shortcut**: Ctrl+F / QKeySequence::Find → focuses search field

### Filter Implementation
- `GameListSortModel` (QSortFilterProxyModel subclass) with `setFilterType()`, `setFilterRegion()`, `setFilterName()`
- All three filters can be combined (AND logic)

## 5. Cover Art System

### Loading Pipeline
1. `loadOrGenerateCover(entry)` called on-demand from `data()` for `DecorationRole` on `Column_Cover`
2. Checks LRU cache first → returns cached pixmap if found
3. If not cached, inserts placeholder into cache and queues async load via `QtConcurrent::run`
4. Async: looks up `GameList::GetCoverImagePathForEntry(&entry)` → loads from disk
5. If no cover found → generates placeholder with game title text centered
6. Final scale validation via atomic counter (`m_cover_scale_counter`) before inserting into cache
7. After cache insert → `invalidateCoverForPath()` emits `dataChanged` for that row's Cover column

### Cover Cache
- `LRUCache<std::string, QPixmap>` keyed by game file path
- Capacity dynamically calculated: `max(num_columns × num_rows, 256)`
- Cleared entirely on scale change or `refreshCovers()`

### Placeholder Generation
- Loads `cover-placeholder.png` from resources
- Resizes to current cover dimensions
- Draws game title text (point size = `32 × scale`, min 1) centered with word wrap

### Cover Dimensions
- Base: 350×512px, spacing 32px
- Scaled: `max(static_cast<int>(350 × scale), 1)` × `max(static_cast<int>(512 × scale), 1)`

## 6. Custom Background System

### Settings
- `UI/GameListBackgroundPath` — absolute or relative path to image/animated PNG
- `UI/GameListBackgroundMode` — scaling mode: Fill, Fit, Stretch, Center (via `QtUtils::ScalingMode`)
- `UI/GameListBackgroundOpacity` — float 0-100

### Supported Formats
- Static images (any Qt-supported format)
- Animated PNG (APNG) — detected by `.png` extension → `QMovie` with format "apng"

### Rendering
- Painted via `eventFilter` on `m_ui.stack` paint event
- `QPainter::drawTiledPixmap()` with scaled pixmap
- Animation paused when window not visible or not active
- `updateCustomBackgroundState()` controls play/pause based on visibility + `Qt::ApplicationActive`
- Frame processing: `processBackgroundFrames()` → resize/scale pixmap per widget dimensions

### Alternating Row Colors
- Disabled when custom background is active

## 7. Toolbar Elements

| Element | Type | Function |
|---------|------|----------|
| `viewGameList` | QPushButton | Switch to table view |
| `viewGameGrid` | QPushButton | Switch to grid view |
| `viewGridTitles` | QPushButton (toggle) | Show/hide cover titles in grid |
| `gridScale` | QSlider | Grid zoom (10-200) |
| `filterType` | QComboBox | Filter by entry type |
| `filterRegion` | QComboBox | Filter by region |
| `searchText` | QLineEdit | Text search |

- `updateToolbar()` syncs UI state from model/Host settings on every view change

## 8. Refresh System

### Thread
- `GameListRefreshThread` (separate class, not in these files)
- Started by `refresh(invalidate_cache, popup_on_error)`
- Emits `refreshProgress(status, current, total)` and `refreshComplete()`

### Cancellation
- `cancelRefresh()` → calls `cancel()` + `wait()` + spin-loop processing events until `m_refresh_thread == nullptr`

### Progress
- On first progress → switches away from empty widget (index 2) to table/grid
- On complete → calls `m_model->refresh()` (beginResetModel/endResetModel)
- If still 0 rows → switches to empty widget (index 2)

### Rescan
- `rescanFile(path)` — UI thread, calls `GameList::RescanPath(path)` then `m_model->refresh()`
- Blocked when VM is running

## 9. Selection & Activation

- **Selection**: `SingleSelection` + `SelectRows` behavior
- **Activation** (double-click/enter): Emits `entryActivated()` signal
- **Context menu**: Emits `entryContextMenuRequested(QPoint)` with global coordinates
- **Selection change**: Emits `selectionChanged()` signal

### getSelectedEntry()
- Returns `std::optional<GameList::Entry>` (copy, not pointer)
- Handles both table and grid view selection
- Uses `GameList::GetLock()` mutex

## 10. Sort Behavior per Column

| Column | Primary Sort | Secondary Sort (tie-breaker) |
|--------|-------------|------------------------------|
| Type | Enum value | Title |
| Serial | Case-insensitive string | Title |
| Title | `QtHost::LocaleSensitiveCompare` | N/A |
| File Title | Case-insensitive string | Title |
| CRC | Integer | Title |
| Time Played | Integer (seconds) | Title |
| Last Played | Integer (timestamp) | Title |
| Size | Integer (bytes) | Title |
| Region | Enum value | Title |
| Compatibility | Enum value | Title |

## 11. UI Settings Persistence

| Setting Key | Type | Default | Purpose |
|-------------|------|---------|---------|
| `UI/GameListGridView` | bool | false | Table vs grid view |
| `UI/GameListShowCoverTitles` | bool | true | Show titles in grid |
| `UI/GameListCoverArtScale` | float | 0.45 | Grid zoom level |
| `UI/GameListBackgroundPath` | string | "" | Custom background path |
| `UI/GameListBackgroundMode` | string | "Fit" | Background scaling mode |
| `UI/GameListBackgroundOpacity` | float | 100.0 | Background opacity |
| `UI/PreferEnglishGameList` | bool | false | Prefer English titles |
| `GameListTableView/HeaderState` | string (Base64) | auto | Column visibility/order/width/sort |
| `GameListTableView/SortColumn` | string | "Title" | Last sort column name |
| `GameListTableView/SortDescending` | bool | false | Sort direction |

## 12. Signals Emitted by GameListWidget

| Signal | Parameters | When |
|--------|-----------|------|
| `refreshProgress` | `QString status, int current, int total` | During scan |
| `refreshComplete` | *(none)* | Scan finished |
| `selectionChanged` | *(none)* | Selection changed in table or grid |
| `entryActivated` | *(none)* | Double-click/enter on game |
| `entryContextMenuRequested` | `QPoint point` | Right-click on game (global coords) |
| `addGameDirectoryRequested` | *(none)* | Empty widget "Add Game Directory" button |
| `layoutChange` | *(none)* | View mode changed or cover titles toggled |

## 13. Hidden/Edge Features

1. **APNG support**: Custom backgrounds support animated PNG via `QMovie` with "apng" format
2. **DPI awareness**: `setDevicePixelRatio()` propagates to model, affects cover rendering and icon sizes
3. **Column reordering**: Drag-and-drop in header, persisted in header state
4. **Ctrl+F shortcut**: Global shortcut to focus search field
5. **Viewport background**: Custom background paints on stack widget via event filter, not on individual views
6. **Safety fallback**: Header visibility guaranteed even with corrupt config
7. **VM-aware rescanning**: `rescanFile()` refuses to run while VM is active
8. **Atomic cover scale counter**: Prevents stale async cover loads from corrupting cache after zoom changes
9. **Highlighted icon cache**: Selected-row icon tinting cached in `QPixmapCache` by key combining cacheKey + enabled state + color
10. **Application state awareness**: Background animation pauses when app loses focus
11. **Title sorting preference**: `m_prefer_english_titles` affects sorting AND display via `GetTitleSort()` / `GetTitle()`
12. **Per-column icon delegates**: Type (col 0), Region (col 8), Compatibility (col 9) use custom `GameListIconStyleDelegate` for centered icon rendering
13. **Cover title font scaling**: Grid font size = `20.0f × cover_scale`

## 14. Source Files Referenced

| File | Path | Lines |
|------|------|-------|
| GameListWidget.h | `pcsx2-qt/GameList/GameListWidget.h` | ~120 |
| GameListWidget.cpp | `pcsx2-qt/GameList/GameListWidget.cpp` | ~650 |
| GameListModel.h | `pcsx2-qt/GameList/GameListModel.h` | ~100 |
| GameListModel.cpp | `pcsx2-qt/GameList/GameListModel.cpp` | ~400 |

### External Dependencies (not analyzed)
- `GameListRefreshThread` — separate file, handles actual disk scanning
- `GameList::Entry`, `GameList::GetLock()`, `GameList::GetEntryByIndex()` — core data layer
- `QtUtils::ResizeColumnsForTableView`, `QtUtils::resizeAndScalePixmap` — utility functions
- `InterfaceSettingsWidget::BACKGROUND_SCALE_NAMES` — scaling mode name strings
