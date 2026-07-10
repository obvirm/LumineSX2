# GameSummaryWidget — Deep Analysis

## Overview
Per-game settings tab displaying game metadata, disc verification, input profile selection, and custom overrides (title/region). Part of the per-game settings dialog.

## UI Elements

| Element | Type | Description |
|---------|------|-------------|
| `title` | QLineEdit | Editable game title |
| `titleSort` | QLineEdit | Sort title (hidden if empty) |
| `titleEN` | QLineEdit | English title (hidden if empty) |
| `path` | QLineEdit | Game file path (read-only) |
| `serial` | QLineEdit | Game serial (read-only) |
| `crc` | QLineEdit | CRC32 hash (read-only) |
| `type` | QComboBox | Entry type (PS1/PS2 Disc, ELF) |
| `region` | QComboBox | Region selector with flag icons |
| `compatibility` | QLabel | Compatibility rating + star display |
| `inputProfile` | QComboBox | Input profile selector |
| `discPath` | QLineEdit | Disc path override (ELF only) |
| `discPathBrowse` | QPushButton | Browse disc path (ELF only) |
| `discPathClear` | QPushButton | Clear disc path (ELF only) |
| `restoreTitle` | QPushButton | Restore original title |
| `restoreRegion` | QPushButton | Restore original region |
| `verify` | QPushButton | Verify disc hashes against Redump DB |
| `verifyResult` | QPlainTextEdit | Verification output |
| `verifyResult` | QPlainTextEdit | Verification result (hidden until verified) |
| `searchHash` | QPushButton | Search hash on redump.org (hidden until verified) |
| `checkWiki` | QPushButton | Open PCSX2 wiki page for serial |
| `tracks` | QTableWidget | Track list with hash/status columns |

## Signals/Callbacks

| Signal | Slot | Description |
|--------|------|-------------|
| `inputProfile::currentIndexChanged` | `onInputProfileChanged(int)` | Save input profile to per-game settings |
| `verify::clicked` | `onVerifyClicked()` | Compute MD5/SHA1 hashes and verify against GameDatabase |
| `searchHash::clicked` | `onSearchHashClicked()` | Open `http://redump.org/discs/quicksearch/{hash}` |
| `checkWiki::clicked` | `onCheckWikiClicked(serial)` | Open `https://wiki.pcsx2.net/{serial}` |
| `discPath::textChanged` | `onDiscPathChanged(QString)` | Save disc path override for ELF |
| `discPathBrowse::clicked` | `onDiscPathBrowseClicked()` | File dialog for disc image |
| `discPathClear::clicked` | `discPath::clear` | Clear disc path override |
| `title::editingFinished` | lambda | Save custom title via `GameList::SaveCustomTitleForPath` |
| `restoreTitle::clicked` | lambda | Clear custom title (set empty) |
| `region::currentIndexChanged` | lambda | Save custom region via `GameList::SaveCustomRegionForPath` |
| `restoreRegion::clicked` | lambda | Clear custom region (set -1) |

## Features

### 1. Game Metadata Display
- Title (editable, saveable as custom)
- Sort title (hidden if empty)
- English title (hidden if empty)
- Path (read-only)
- Serial (read-only)
- CRC32 (read-only, uppercase hex)
- Type dropdown
- Region dropdown with flag icons (`icons/flags/{flag}.svg`)
- Compatibility rating with ★/☆ star display

### 2. Custom Title/Region Override
- Edit title → saves via `GameList::SaveCustomTitleForPath`
- Restore button clears custom title
- Change region → saves via `GameList::SaveCustomRegionForPath`
- Restore button clears custom region (sets -1)
- Both trigger `repopulateCurrentDetails()` to refresh UI

### 3. Input Profile Selection
- Populated from `Pad::GetInputProfileNames()`
- Index 0 = default (clears profile), others = saves `InputProfileName` to `EmuCore`

### 4. Disc Path Override (ELF Only)
- Only shown for ELF entry type
- Browse button → file dialog with disc image filter
- Clear button → empties field
- Change triggers `g_main_window->rescanFile()` to re-extract serial

### 5. Disc Verification
- Only available for PS1/PS2 disc types
- Disabled while VM is running
- Uses `IsoHasher` to read tracks
- CD: columns = #, Mode, Start, Sectors, Size, MD5, Status
- DVD: columns = #, Start, Sectors, Size, MD5, Status
- MD5/SHA1 computed on verify click
- Verified against `GameDatabase::lookupHash()`
- Green ✓ / Red ✗ per track
- Result: "Verified as {name} [{serial}] (Version {version})"
- On verify, button replaced with result text + "Search Hash" button

### 6. Redump Search
- Only visible after verification
- Opens `http://redump.org/discs/quicksearch/{hash}`

### 7. Wiki Link
- Opens `https://wiki.pcsx2.net/{serial}`
- Disabled if serial is empty

### 8. Track List Table
- Populated from `IsoHasher::GetTracks()`
- Shows track number, mode (CD only), start LSN, sectors, size
- MD5 column shows "not computed" until verify is clicked

### 9. Region Flags
- Loaded from `{resources}/icons/flags/{RegionToFlagFilename}.svg`
- Set as icons on region combobox items

## Hidden/Non-obvious Features
1. **Title sort / English title** — hidden rows in form layout, only shown when non-empty
2. **Disc path forces rescan** — changing disc path triggers full file rescan to re-extract serial
3. **Verify button self-destructs** — after first verify, the button is deleted and replaced with result text
4. **Redump search keyword** — stored from first track hash during verify, used for "Search Hash" button
5. **ELF disc path** — allows setting a virtual disc for ELF executables (used for homebrew/ELF loading)
6. **Per-game settings** — uses `dialog()->getStringValue/setStringSettingValue` which is per-game INI
7. **Compatibility stars** — calculated as `rating_value - 1` filled stars out of 5 (6 minus filled)
