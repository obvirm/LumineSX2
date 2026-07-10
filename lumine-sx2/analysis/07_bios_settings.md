# BIOS Settings Widget — Deep Analysis

## Source Files
- `pcsx2-qt/Settings/BIOSSettingsWidget.h`
- `pcsx2-qt/Settings/BIOSSettingsWidget.cpp`
- `pcsx2-qt/Settings/BIOSSettingsWidget.ui`

## UI Layout (3 GroupBoxes)

### 1. BIOS Directory
| Element | Type | Purpose |
|---------|------|---------|
| `searchDirectory` | QLineEdit | Path to BIOS folder |
| `browseSearchDirectory` | QPushButton ("Browse...") | Folder picker dialog |
| `resetSearchDirectory` | QPushButton ("Reset") | Reset to default path |
| label | QLabel | "PCSX2 will search for BIOS images in this directory." |

- **Setting key**: `Folders/Bios`
- **Default**: `Path::Combine(EmuFolders::DataRoot, "bios")`
- **Binding**: `SettingWidgetBinder::BindWidgetToFolderSetting` (auto binds browse/reset/open buttons)

### 2. BIOS Selection
| Element | Type | Purpose |
|---------|------|---------|
| `fileList` | QTreeWidget | Two columns: **Filename** (250px min), **Version** |
| `openSearchDirectory` | QPushButton ("Open BIOS Folder...") | Opens folder in file explorer |
| `refresh` | QPushButton ("Refresh List") | Re-scans directory |

### 3. Fast Boot Options
| Element | Type | Purpose |
|---------|------|---------|
| `fastBoot` | QCheckBox ("Fast Boot") | Skip BIOS boot animation |
| `fastBootFastForward` | QCheckBox ("Fast Forward Boot") | Remove speed throttle until game starts |

- **Fast Boot setting**: `EmuCore/EnableFastBoot` (default: true)
- **Fast Forward Boot**: `EmuCore/EnableFastBootFastForward` (default: false)
- **Dependency**: `fastBootFastForward` is **disabled** when `fastBoot` is unchecked

## BIOS Detection Logic

### `populateList(QTreeWidget*, directory)`
1. Gets currently selected BIOS from `Filenames/BIOS`
2. Scans directory using `FileSystem::FindFiles` (includes hidden files)
3. For each file, calls `IsBIOS(filename, &version, &description, &region, &zone)`
4. Populates tree with **Filename** + **Description**
5. Adds **region flag icon** based on region code:

| Region Code | Region | Flag Icon |
|-------------|--------|-----------|
| 0 | Japan | `icons/flags/jp.svg` |
| 1 | USA | `icons/flags/us.svg` |
| 2 | Europe | `icons/flags/eu.svg` |
| 3 | Oceania | `icons/flags/au.svg` |
| 4 | Asia | `icons/flags/hk.svg` |
| 5 | Russia | `icons/flags/ru.svg` |
| 6 | China | `icons/flags/cn.svg` |
| 7 | Mexico | `icons/flags/mx.svg` |
| 8-10 | T10K/Test/Free/Default | `icons/flags/jp.svg` |

6. Auto-selects the currently saved BIOS

### `IsBIOS` signature (from BiosTools.h)
```cpp
bool IsBIOS(const char* filename, u32& version, std::string& description, u32& region, std::string& zone);
```

## Signals/Callbacks
| Signal | Handler | Action |
|--------|---------|--------|
| `searchDirectory::textChanged` | `refreshList()` | Re-scan directory on path change |
| `refresh::clicked` | `refreshList()` | Manual rescan |
| `fileList::currentItemChanged` | `listItemChanged()` | Save selected BIOS to `Filenames/BIOS`, commit, apply settings |
| `fastBoot::checkStateChanged` | `fastBootChanged()` | Enable/disable fast-forward checkbox |

## Help Tooltips
- **Fast Boot**: "Patches the BIOS to skip the console's boot animation." (default: Checked)
- **Fast Forward Boot**: "Removes emulation speed throttle until the game starts to reduce startup time." (default: Unchecked)

## Hidden/Notable Features
1. **Hidden file scanning**: `FILESYSTEM_FIND_HIDDEN_FILES` flag is used — scans for hidden BIOS files
2. **Fast Forward Boot**: Distinct from Fast Boot — removes speed throttle entirely until game loads
3. **`populateList` is static**: Can be called from other widgets (e.g., setup wizard)
4. **Real-time apply**: Changing BIOS selection immediately applies via `g_emu_thread->applySettings()`
5. **Region-based flag icons**: Visual region indicator via SVG flag icons
6. **QSignalBlocker**: Prevents signal loops during list repopulation
7. **`qApp->processEvents`**: Prevents UI freeze during directory scan

## Settings Keys Summary
| Key | Section | Type | Default |
|-----|---------|------|---------|
| `Bios` | `Folders` | string (path) | `<DataRoot>/bios` |
| `BIOS` | `Filenames` | string (filename) | (none — user selects) |
| `EnableFastBoot` | `EmuCore` | bool | `true` |
| `EnableFastBootFastForward` | `EmuCore` | bool | `false` |
