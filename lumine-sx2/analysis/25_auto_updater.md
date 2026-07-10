# AutoUpdaterDialog — Complete Feature Analysis

## Overview
Modal dialog for downloading and installing PCSX2 updates. Supports Windows (exe updater + elevation), Linux (AppImage replace), macOS (tar + bundle swap). Uses `HTTPDownloader` with polling timer pattern.

---

## UI Elements

| Widget | Type | Purpose |
|--------|------|---------|
| `label_2` | QLabel (24x24) | Update icon pixmap `:/icons/update.png` |
| `label` | QLabel (16pt bold) | Title: "Update Available" |
| `currentVersion` | QLabel | "Current Version: {tag} ({date})" |
| `newVersion` | QLabel | "New Version: {version} ({timestamp})" |
| `downloadSize` | QLabel | "Download Size: {size} MB" |
| `updateNotes` | QTextBrowser | Changelog HTML with commits list, save state / settings warnings |
| `downloadAndInstall` | QPushButton | **Disabled** until changelog loads. Triggers download+install flow |
| `skipThisUpdate` | QPushButton | Saves version to settings so it won't prompt again |
| `remindMeLater` | QPushButton (default) | Closes dialog, no persistence |

Layout: `QVBoxLayout` → icon+title row, 3 info labels, `QTextBrowser`, button row (`QHBoxLayout` with spacer).

---

## Static API

| Method | Returns | Purpose |
|--------|---------|---------|
| `isSupported()` | bool | True only on tagged commits. Windows/macOS always. Linux only if `$APPIMAGE` set |
| `getTagList()` | QStringList | `["stable", "nightly"]` — available release channels |
| `getDefaultTag()` | std::string | `DEFAULT_UPDATER_CHANNEL` compile-time (defaults to `"nightly"`) |
| `getCurrentVersion()` | QString | `BuildVersion::GitTag` |
| `getCurrentVersionDate()` | QString | `BuildVersion::GitDate` |
| `cleanupAfterUpdate()` | void | Post-update cleanup — removes updater exe (Win) or backup AppImage (Linux). macOS=noop |

## Signals

| Signal | Purpose |
|--------|---------|
| `updateCheckCompleted()` | Emitted when check finishes (success or fail). Used by MainWindow to know when to proceed |

## Slots

| Slot | Purpose |
|------|---------|
| `queueUpdateCheck(bool display_message)` | Entry point. `display_message=false` = background silent check. `display_message=true` = user-initiated, shows errors in QMessageBox |

---

## Update Check Flow

1. `queueUpdateCheck(display_message)` called
2. `ensureHttpReady()` — creates `HTTPDownloader`, starts 10ms polling `QTimer`
3. HTTP GET → `https://api.pcsx2.net/v1/{tag}Releases?pageSize=1`
4. `getLatestReleaseComplete()` parses JSON response:
   - `data[0].version` → `m_latest_version`
   - `data[0].publishedAt` → `m_latest_version_timestamp`
   - `data[0].assets.{Platform}[]` → asset selection with scoring:
     - Score 4: Perfect ISA match (AVX2/SSE4 matching current build)
     - Score 3: AVX2 preferred over SSE4
     - Score 2: SSE4 preferred over untagged
     - Score 1: Multi-ISA (no tags, fallback)
     - Skipped: `symbols`, `installer` tagged assets
     - Skipped: AVX2 if current CPU lacks AVX2 (`cpuinfo_has_x86_avx2()`)
   - Best asset → `m_download_url`, `m_download_size`
5. `checkIfUpdateNeeded()`:
   - Compare `m_latest_version` vs `BuildVersion::GitTag` AND `LastVersion` setting
   - If same → "No update available" (shows message only if `display_message=true`)
   - If VM running AND auto-check → skip dialog silently
   - Otherwise → populate labels, call `queueGetChanges()`, then `exec()` the modal dialog

## Changelog Fetch

1. `queueGetChanges()` → HTTP GET `https://api.github.com/repos/PCSX2/pcsx2/compare/{currentHash}...{latestVersion}`
2. `getChangesComplete()` parses commit list:
   - Each commit: first-line message + author → `<li>message <i>(author)</i></li>`
   - **Special tags detected**:
     - `[SAVEVERSION+]` in commit message → **Save state incompatibility warning** prepended
     - `[SETTINGSVERSION+]` in commit message → **Settings reset warning** prepended
   - HTML assembled with `<h2>Changes:</h2><ul>...</ul>` + warnings
3. Enables `downloadAndInstall` button

---

## Download & Install Flow

### `downloadUpdateClicked()`
1. If `m_update_will_break_save_states` → show critical QMessageBox warning, require confirmation
2. Show `QtModalProgressCallback` with "Downloading {version}..."
3. HTTP GET `m_download_url` → binary data
4. Calls platform-specific `processUpdate()`
5. On success → `QMetaObject::invokeMethod(g_main_window, "requestExit", ...)` + `done(0)`

### Platform-Specific Install

#### Windows (`_WIN32`)
1. Write downloaded data to `{DataRoot}/update.zip`
2. Extract updater exe from zip via `ExtractUpdater()` → `{DataRoot}/updater.exe`
3. `doesUpdaterNeedElevation()` — tries to create dummy file in app dir; if fails → needs UAC elevation
4. `ShellExecuteExW()` with:
   - `lpVerb = "runas"` if elevation needed (triggers UAC prompt)
   - Arguments: `{PID} "{appDir}" "{zipPath}" "{programPath}"`
   - Updater waits for PCSX2 to exit, then replaces files
5. PCSX2 exits → updater completes installation
6. `cleanupAfterUpdate()` — removes updater exe from DataRoot if portable mode

#### Linux (`__linux__`)
1. Get `$APPIMAGE` path
2. Write downloaded data to `{appimage}.new`
3. Backup: rename current `{appimage}` → `{appimage}.backup`
4. Rename `{appimage}.new` → `{appimage}` (preserve permissions)
5. Execute new AppImage with `-updatecleanup` flag (detached process)
6. Current process exits
7. `cleanupAfterUpdate()` — removes `{appimage}.backup`

#### macOS (`__APPLE__`)
1. Get non-translocated bundle path via `CocoaTools::GetNonTranslocatedBundlePath()`
2. Create temp staging directory
3. Pipe downloaded tar data to `/usr/bin/tar xC {staging}` with progress bar (65KB chunks)
4. Find `.app` bundle in extracted contents
5. Trash old app via `CocoaTools::MoveToTrash()`
6. Rename new app → update version number in name
7. Launch new app via `CocoaTools::DelayedLaunch()`
8. Current process exits
9. `cleanupAfterUpdate()` — noop on macOS

---

## Settings Keys

| Section | Key | Purpose |
|---------|-----|---------|
| `AutoUpdater` | `UpdateTag` | Release channel: "stable" or "nightly" |
| `AutoUpdater` | `LastVersion` | Skipped version (from "Skip This Update") |

## HTTP Infrastructure

- `HTTPDownloader` created with `Host::GetHTTPUserAgent()`
- Polling timer: 10ms interval, single-fire=false
- Stops when all requests complete
- `downloadUpdateClicked()` blocks with manual event loop: `QApplication::processEvents()` + `PollRequests()`
- Cancellation supported via `QProgressDialog`

## ISA Detection

- Compile-time: `UPDATE_ADDITIONAL_TAGS` = "AVX2" or "SSE4" based on `_M_SSE`
- Runtime: `cpuinfo_has_x86_avx2()` for CPU capability check
- Multi-ISA shared compilation (`MULTI_ISA_SHARED_COMPILATION`) disables compile-time tag but runtime check still applies

## Error Handling

- Silent errors (background check): log to console only via `Console.Error()`
- User-visible errors (manual check): `QMessageBox::critical()` with error message
- Download cancellation: silently returns (HTTP_STATUS_CANCELLED)
- All platform install failures: `reportError()` with descriptive message

## Hidden/Advanced Features

1. **Save state compatibility warning**: Detects `[SAVEVERSION+]` in commit messages, shows prominent warning before download
2. **Settings version warning**: Detects `[SETTINGSVERSION+]`, warns about settings reset
3. **UAC elevation detection**: Windows updater tests write permissions before deciding whether to request elevation
4. **ISA-aware asset selection**: Scoring system picks optimal binary for current CPU capabilities
5. **Installer/symbols filtering**: Automatically skips installer and debug symbol assets
6. **VM-aware dialog suppression**: Won't show update dialog during gameplay (background check only)
7. **Full-screen UI awareness**: Also suppresses dialog if running fullscreen Big Picture mode
8. **Version number in app name**: macOS updates rename the .app bundle with new version number
9. **Translocation bypass**: macOS uses `GetNonTranslocatedBundlePath()` to handle Gatekeeper translocation
10. **Portable mode cleanup**: Windows only removes updater exe when AppRoot ≠ DataRoot (portable mode)
