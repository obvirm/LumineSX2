# Analysis #16 — Folder Settings

## Files Analyzed
- `FolderSettingsWidget.h` — trivial header, inherits SettingsWidget
- `FolderSettingsWidget.cpp` — 7 folder bindings + 2 bool checkboxes
- `FolderSettingsWidget.ui` — Qt Designer form, 700x700px

## Architecture Pattern: `BindWidgetToFolderSetting`
Each folder uses a **4-widget group**: `QLineEdit` (path text) + 3 `QPushButton`s:
| Button | Purpose |
|--------|---------|
| **Browse…** | Opens native folder picker dialog |
| **Open…** | Opens folder in OS file explorer |
| **Reset** | Reverts to default path |

Signature: `BindWidgetToFolderSetting(sif, lineEdit, browseBtn, openBtn, resetBtn, section, key, defaultPath, use_relative=true)`

## All 6 Custom Folder Paths

| # | GroupBox Title | Setting Section | Key | Default Path | Description |
|---|---------------|----------------|-----|-------------|-------------|
| 1 | **Cache Directory** | `Folders` | `Cache` | `{DataRoot}/cache` | Shaders, game list, achievement data |
| 2 | **Cheats Directory** | `Folders` | `Cheats` | `{DataRoot}/cheats` | `.pnach` files containing game cheats |
| 3 | **Snapshots Directory** | `Folders` | `Snapshots` | `{DataRoot}/snaps` | Screenshots and GS dumps |
| 4 | **Save States Directory** | `Folders` | `SaveStates` | `{DataRoot}/sstates` | Save state files |
| 5 | **Covers Directory** | `Folders` | `Covers` | `{DataRoot}/covers` | Cover art for game grid/Big Picture |
| 6 | **Video Recording Directory** | `Folders` | `Videos` | `{DataRoot}/videos` | Video recording files |

`{DataRoot}` = `EmuFolders::DataRoot`

## Bool Checkboxes (Hidden Features!)

| # | Checkbox Label | Setting Section | Key | Default | Description |
|---|---------------|----------------|-----|---------|-------------|
| 1 | **Save Snapshots in Game-Specific Folders** | `EmuCore/GS` | `OrganizeScreenshotsByGame` | `false` | Saves screenshots to per-game subfolders |
| 2 | **Save Video Recordings in Game-Specific Folders** | `EmuCore/GS` | `OrganizeVideoCaptureByGame` | `false` | Saves video recordings to per-game subfolders |

## UI Layout (from .ui)
```
QVBoxLayout (vertical)
├── QGroupBox "Cache Directory"
│   └── QGridLayout (2 rows)
│       Row 0: QLabel "Used for storing shaders, game list, and achievement data."
│       Row 1: QLineEdit | Browse | Open | Reset
├── QGroupBox "Cheats Directory"
│   └── QGridLayout
│       Row 0: QLabel "Used for storing .pnach files containing game cheats."
│       Row 1: QLineEdit | Browse | Open | Reset
├── QGroupBox "Snapshots Directory"
│   └── QGridLayout (3 rows)
│       Row 0: QLabel "Used for saving screenshots and GS dumps."
│       Row 1: QLineEdit | Browse | Open | Reset
│       Row 2: QCheckBox "Save Snapshots in Game-Specific Folders"
├── QGroupBox "Save States Directory"
│   └── QGridLayout
│       Row 0: QLabel "Used for storing save states."
│       Row 1: QLineEdit | Browse | Open | Reset
├── QGroupBox "Covers Directory"
│   └── QGridLayout
│       Row 0: QLabel "Used for storing covers in the game grid/Big Picture UIs."
│       Row 1: QLineEdit | Browse | Open | Reset
├── QGroupBox "Video Recording Directory"
│   └── QGridLayout (4 rows)
│       Row 0: QLabel "Used for storing video recordings."
│       Row 3: QLineEdit | Browse | Open | Reset
│       Row 4: QCheckBox "Save Video Recordings in Game-Specific Folders"
└── QSpacerItem (vertical stretch)
```

## UI Elements Summary
- 6 × QLineEdit (path text fields)
- 6 × QPushButton "Browse…" (folder picker)
- 6 × QPushButton "Open…" (open in file explorer)
- 6 × QPushButton "Reset" (restore default)
- 6 × QLabel (descriptions)
- 2 × QCheckBox (organize by game)
- 6 × QGroupBox (section containers)
- 1 × QSpacerItem

Total: 33 widgets

## Tab Order
cache → cacheBrowse → cacheOpen → cacheReset → cheats → cheatsBrowse → cheatsOpen → cheatsReset → snapshots → snapshotsBrowse → snapshotsOpen → snapshotsReset → organizeSnapshotsByGame → saveStates → saveStatesBrowse → saveStatesOpen → saveStatesReset → covers → coversBrowse → coversOpen → coversReset → videoDumpingDirectory → videoDumpingDirectoryBrowse → videoDumpingDirectoryOpen → videoDumpingDirectoryReset

## Settings Keys for LumineSX2 Implementation
```
[Folders]
Cache = ""       (default: {DataRoot}/cache)
Cheats = ""      (default: {DataRoot}/cheats)
Snapshots = ""   (default: {DataRoot}/snaps)
SaveStates = ""  (default: {DataRoot}/sstates)
Covers = ""      (default: {DataRoot}/covers)
Videos = ""      (default: {DataRoot}/videos)

[EmuCore/GS]
OrganizeScreenshotsByGame = false
OrganizeVideoCaptureByGame = false
```

## Gaps in LumineSX2 UI
Current `FoldersSettingsView` has 7 text fields but is **missing**:
1. ❌ **Cache** folder
2. ❌ **Covers** folder
3. ❌ **Videos** folder
4. ❌ "Open in File Explorer" buttons (Browse/Open/Reset per field)
5. ❌ **Organize Snapshots by Game** checkbox
6. ❌ **Organize Videos by Game** checkbox
7. ❌ Description labels explaining each folder's purpose
