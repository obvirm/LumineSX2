# Game List Settings Widget — Deep Analysis

## Source Files
- `pcsx2-qt/Settings/GameListSettingsWidget.h`
- `pcsx2-qt/Settings/GameListSettingsWidget.cpp`
- `pcsx2-qt/Settings/GameListSettingsWidget.ui`

---

## UI Layout Structure

```
QWidget (GameListSettingsWidget) — 700×800
└── QVBoxLayout
    └── QGroupBox "Game Scanning"
        └── QVBoxLayout
            ├── QHBoxLayout
            │   ├── QLabel "Search Directories (will be scanned for games)"
            │   ├── QToolButton "Add..." (theme: folder-add-line)
            │   └── QToolButton "Remove" (theme: folder-reduce-line)
            ├── QTableWidget [searchDirectoryList]
            │   ├── Column 0: "Search Directory"
            │   └── Column 1: "Scan Recursively"
            ├── QHBoxLayout
            │   ├── QLabel "Excluded Paths (will not be scanned)"
            │   ├── QToolButton "Directory..." (theme: folder-add-line)
            │   ├── QToolButton "File..." (theme: file-add-line)
            │   └── QToolButton "Remove" (theme: file-reduce-line)
            ├── QListWidget [excludedPaths]
            └── QHBoxLayout
                ├── QPushButton "Scan For New Games" (theme: file-search-line)
                └── QPushButton "Rescan All Games" (theme: refresh-line)
```

---

## UI Elements Inventory

### 1. Search Directory List (QTableWidget)
- **Widget name:** `searchDirectoryList`
- **Columns:** 2 — "Search Directory" (text), "Scan Recursively" (checkbox)
- **Selection mode:** `SingleSelection`
- **Selection behavior:** `SelectRows`
- **Alternating row colors:** enabled
- **Grid:** hidden
- **Header highlight:** disabled on click
- **Vertical header:** hidden
- **Context menu:** custom (right-click)
- **Initial selection:** cleared (`setCurrentIndex({})`)
- **Column resize:** auto-resize on layout/resize events — column 0 = fill remaining, column 1 = 100px fixed
- **Sorting:** sorted ascending by column 0 after refresh

### 2. Excluded Paths List (QListWidget)
- **Widget name:** `excludedPaths`
- **Initial state:** remove button disabled

### 3. Buttons
| Button | Widget Name | Theme Icon | Function |
|--------|-------------|------------|----------|
| Add directory | `addSearchDirectoryButton` | `folder-add-line` | Opens folder picker, asks recursive? |
| Remove directory | `removeSearchDirectoryButton` | `folder-reduce-line` | Removes selected directory |
| Add excluded directory | `addExcludedPath` | `folder-add-line` | Opens folder picker |
| Add excluded file | `addExcludedFile` | `file-add-line` | Opens file picker |
| Remove excluded path | `removeExcludedPath` | `file-reduce-line` | Removes selected exclusion |
| Scan for new games | `scanForNewGames` | `file-search-line` | Incremental scan |
| Rescan all games | `rescanAllGames` | `refresh-line` | Full rescan |

### 4. Tab Order
`addSearchDirectoryButton` → `removeSearchDirectoryButton` → `searchDirectoryList` → `addExcludedPath` → `addExcludedFile` → `removeExcludedPath` → `excludedPaths` → `scanForNewGames` → `rescanAllGames`

---

## Settings Keys (INI)

### GameList/Paths (string list)
- Non-recursive search directories

### GameList/RecursivePaths (string list)
- Recursive search directories

### GameList/ExcludedPaths (string list)
- Excluded file paths and directory paths

---

## Signals & Slots

| Signal | Slot | Behavior |
|--------|------|----------|
| `searchDirectoryList.customContextMenuRequested` | `onDirectoryListContextMenuRequested` | Shows context menu with "Remove" and "Open Directory..." |
| `searchDirectoryList.itemSelectionChanged` | `onDirectoryListSelectionChanged` | Enables/disables remove button |
| `addSearchDirectoryButton.clicked` | `onAddSearchDirectoryButtonClicked` | Calls `addSearchDirectory(QWidget*)` |
| `removeSearchDirectoryButton.clicked` | `onRemoveSearchDirectoryButtonClicked` | Removes selected row from table and settings |
| `addExcludedFile.clicked` | `onAddExcludedFileButtonClicked` | File picker → add to excluded paths |
| `addExcludedPath.clicked` | `onAddExcludedPathButtonClicked` | Folder picker → add to excluded paths |
| `removeExcludedPath.clicked` | `onRemoveExcludedPathButtonClicked` | Removes selected exclusion |
| `excludedPaths.itemSelectionChanged` | `onExcludedPathsSelectionChanged` | Enables/disables remove button |
| `scanForNewGames.clicked` | `onScanForNewGamesClicked` | Calls `g_main_window->refreshGameList(false, true)` |
| `rescanAllGames.clicked` | `onRescanAllGamesClicked` | Calls `g_main_window->refreshGameList(true, true)` |

---

## Detailed Callback Flows

### addSearchDirectory(QWidget* parent_widget)
1. Opens `QFileDialog::getExistingDirectory` → native separators
2. If empty, return
3. Shows `QMessageBox::question` — "Scan Recursively?" with Yes/No/Cancel
4. If Cancel, return
5. If Yes → `addSearchDirectory(dir, true)` (recursive)
6. If No → `addSearchDirectory(dir, false)` (non-recursive)

### addSearchDirectory(path, recursive)
1. Removes path from opposite list (if recursive, remove from "Paths"; if not, remove from "RecursivePaths")
2. Adds path to correct list
3. Commits settings
4. Refreshes directory list table
5. Triggers `g_main_window->refreshGameList(false, true)`

### removeSearchDirectory(path)
1. Tries to remove from both "Paths" and "RecursivePaths"
2. If found and removed → commits settings, refreshes table, triggers game list refresh

### addPathToTable(path, recursive)
1. Inserts new row at end of table
2. Column 0: QTableWidgetItem with path (non-editable)
3. Column 1: QCheckBox (checked = recursive)
4. Checkbox state change signal → moves path between "Paths" and "RecursivePaths" lists

### refreshDirectoryList()
1. Blocks signals on table
2. Clears all rows
3. Loads "Paths" → adds with recursive=false
4. Loads "RecursivePaths" → adds with recursive=true
5. Sorts by column 0 ascending
6. Disables remove button

### addExcludedPath(path)
1. `Host::AddBaseValueToStringList("GameList", "ExcludedPaths", path)`
2. Commits settings
3. Adds to QListWidget
4. Triggers `g_main_window->refreshGameList(false, true)`

### refreshExclusionList()
1. Clears QListWidget
2. Loads all "ExcludedPaths" into list
3. Disables remove button

### onDirectoryListContextMenuRequested(point)
- Context menu items:
  - "Remove" → calls `onRemoveSearchDirectoryButtonClicked()`
  - Separator
  - "Open Directory..." → opens directory in file manager via `QtUtils::OpenURL`

### onScanForNewGamesClicked()
- Calls `g_main_window->refreshGameList(false, true)` — incremental scan (new games only)

### onRescanAllGamesClicked()
- Calls `g_main_window->refreshGameList(true, true)` — full rescan (all games)

---

## Hidden Features & Edge Cases

1. **Recursive toggle is inline** — each directory row has a checkbox for recursive scanning; toggling it immediately moves the path between "Paths" and "RecursivePaths" settings keys
2. **Context menu on directory list** — right-click shows "Remove" and "Open Directory..." (opens in OS file manager)
3. **Excluded paths can be files OR directories** — two separate buttons for file and directory exclusion
4. **addExcludedPath is public** — callable from outside the widget (e.g., from game list context menu to quickly exclude a game)
5. **refreshExclusionList is public** — callable from outside to reload exclusions
6. **Auto column resize** — table columns resize on every layout/resize event; column 0 fills remaining space, column 1 is 100px
7. **Single selection only** — directory list restricted to single row selection
8. **Immediate persistence** — every add/remove/recursive-toggle immediately commits to INI settings
9. **Game list refresh triggers** — all add/remove operations trigger `g_main_window->refreshGameList(false, true)` for incremental scan
10. **Native separators** — all file paths converted to native OS separators via `QDir::toNativeSeparators`
11. **Recursive scan dialog** — adding a directory prompts Yes/No/Cancel for recursive scanning
12. **addSearchDirectory is a public slot** — can be called from MainWindow or other components
13. **Duplicate handling** — adding a directory that already exists in one list removes it from the other (Paths ↔ RecursivePaths)
