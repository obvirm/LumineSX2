# Cover Download Feature — Deep Analysis

## Files Analyzed
- `pcsx2-qt/CoverDownloadDialog.h` (48 lines)
- `pcsx2-qt/CoverDownloadDialog.cpp` (182 lines)
- `pcsx2/GameList.cpp` — `DownloadCovers()`, `GetCoverImagePathForEntry()`, `GetNewCoverImagePathForEntry()`
- `pcsx2/GameList.h` — function signatures
- `pcsx2/ImGui/FullscreenUI.cpp` — `DrawCoverDownloaderWindow()`, `OpenCoverDownloaderWindow()`, `CloseCoverDownloaderWindow()`

---

## 1. UI Elements (Qt Dialog)

| Element | Type | Purpose |
|---------|------|---------|
| `urls` | QTextEdit (multiline) | URL template input, one per line |
| `useTitleFileNames` | QCheckBox | Toggle title-based vs serial-based filenames |
| `start` | QPushButton | Start/Stop toggle button |
| `close` | QPushButton | Close dialog |
| `status` | QLabel | Current download status text |
| `progress` | QProgressBar | Download progress bar |
| `coverIcon` | QLabel | Icon display (artboard-2-line theme) |

## 2. URL Template System

### Placeholders
| Placeholder | Description | Example |
|-------------|-------------|---------|
| `${title}` | URL-encoded game title | `Final+Fantasy+X` |
| `${filetitle}` | URL-encoded file title (from ISO path) | `SLUS_203.12` |
| `${serial}` | URL-encoded game serial | `SLUS-20312` |

### Validation Rules
- URL template MUST contain at least one of: `${title}`, `${filetitle}`, or `${serial}`
- If none found → error: "URL template must contain at least one of..."
- Multiple URLs supported (one per line)
- Each URL template is applied to ALL game entries

### URL Examples (typical)
```
https://example.com/covers/${serial}.jpg
https://cdn.example.com/${title}/cover.png
```

## 3. Cover Filename Resolution (`GetCoverImagePathForEntry`)

Search priority (per extension `.jpg`, `.jpeg`, `.png`, `.webp`):

1. **File title** (from ISO path) — prioritized because users can change titles for modded games
2. **Serial** — most specific identifier
3. **Game title** — sanitized filename
4. **English title** (`title_en`) — fallback for localized titles

Returns empty string if no cover found (download will proceed).

## 4. Download Flow

### `GameList::DownloadCovers()` (core logic)

```
1. Validate URL templates (must have ${title}, ${filetitle}, or ${serial})
2. Create covers directory (EmuFolders::Covers)
3. Build download queue:
   - For each game entry in GameList
   - Skip if cover already exists (GetCoverImagePathForEntry returns non-empty)
   - For each URL template:
     - Replace placeholders with URL-encoded values
     - Add (entry_path, url) to queue
4. Create HTTPDownloader instance
5. For each download URL:
   - Check cancellation
   - Verify entry still doesn't have cover
   - Show status: "Downloading cover for {title} [{serial}]..."
   - HTTP GET request
   - On response (200 OK, non-empty data):
     - Determine extension from Content-Type header
     - Fallback: parse extension from URL, or default "cover.jpg"
     - Get write path via GetNewCoverImagePathForEntry()
     - Write binary file to disk
     - Call save_callback if provided
   - Wait for each request sequentially (serial downloads)
   - Increment progress
```

### Hidden Features
- **Skips existing covers**: Won't re-download if cover already exists
- **Content-Type detection**: Uses HTTP Content-Type header for file extension, not just URL
- **Extension fallback chain**: Content-Type → URL extension → `cover.jpg`
- **File title vs serial naming**: `GetNewCoverImagePathForEntry()` decides filename based on `use_serial` flag
- **Sequential downloads** (with comment: "we could actually do a few in parallel here...")
- **Sanitized filenames**: `Path::SanitizeFileName()` applied to all generated filenames

## 5. Thread Architecture

### Qt Version
```
Main Thread:
  CoverDownloadDialog
    ├── QTextEdit (URL input)
    ├── QCheckBox (title/serial toggle)
    ├── QProgressBar
    └── Start/Stop/Close buttons

Worker Thread:
  CoverDownloadThread (extends QtAsyncProgressThread)
    ├── Inherits QThread + BaseProgressCallback
    ├── Signals: statusUpdated, progressUpdated, threadFinished
    └── runAsync() → GameList::DownloadCovers()
```

### FullscreenUI (ImGui) Version
```
Main Thread:
  DrawCoverDownloaderWindow() — ImGui rendering
    ├── InputTextMultiline (URL buffer)
    ├── ToggleButton (use title filenames)
    ├── ProgressBar
    └── Start/Stop buttons

Worker Thread:
  std::thread → CoverDownloaderThreadFunc()
    └── Uses custom ProgressCallback with mutex-protected state
```

## 6. Auto-Save Preferences

### URL Persistence
- **Storage**: `Host::Internal::GetSecretsSettingsLayer()` (encrypted/secure settings)
- **Section**: `[UI/CoverURLs]`
- **Format**:
  ```ini
  [UI/CoverURLs]
  Count=2
  0=https://example.com/covers/${serial}.jpg
  1=https://cdn.example.com/${title}/cover.png
  ```
- **Load**: On dialog construction (`loadCoverURLs()`)
- **Save**: On dialog destruction (`~CoverDownloadDialog()`) and `closeEvent()`
- **Clear + rewrite**: Entire section removed and rewritten each save

### FullscreenUI Version
- **Storage**: `FullscreenUI::GetUSBDeviceConfig()` path (different from Qt)
- **Buffer**: `s_cover_downloader_urls_buffer` (char array, 4096 bytes)

## 7. Progress & Rate Limiting

- **Progress bar**: Tracks per-game completion (value/range)
- **Refresh throttling**: `coverRefreshRequested()` signal limited to once per 5 seconds (`Common::Timer`)
- **Rationale**: "Otherwise it's way too flickery. Ideally in the future we'd have some way to invalidate only a single cover."
- **On complete**: Final `coverRefreshRequested()` emitted (no throttle)

## 8. Signal/Callback Flow

```
CoverDownloadThread::statusUpdated(str)    → onDownloadStatus(str)
CoverDownloadThread::progressUpdated(v, r) → onDownloadProgress(v, r)
CoverDownloadThread::threadFinished()      → onDownloadComplete()

onDownloadProgress:
  - Update progress bar max/value
  - Throttled coverRefreshRequested() signal (5s interval)

onDownloadComplete:
  - Emit coverRefreshRequested() (unthrottled)
  - Join thread, reset pointer
  - Update status: "Download complete."

onStartClicked:
  - If thread running → cancelThread()
  - Else → startThread()

cancelThread:
  - requestInterruption()
  - join()
  - reset thread
```

## 9. UI State Management

```
State Transitions:
  IDLE → DOWNLOADING → COMPLETE
       ↓
    CANCELLED (via Stop button or close)

updateEnabled() states:
  - Running: Start→Stop text, Close disabled, URLs disabled
  - Idle: Start enabled only if URLs non-empty, Close enabled, URLs enabled
```

## 10. FullscreenUI Differences

| Feature | Qt Dialog | FullscreenUI |
|---------|-----------|--------------|
| Thread type | QtAsyncProgressThread | std::thread |
| Progress callback | Signal-based | Mutex-protected globals |
| URL storage | Secrets settings layer | Config buffer |
| Error display | Status label | Toast notification |
| Cover map | N/A | Clears `s_cover_image_map` on completion |
| Filename option | "Use Title File Names" checkbox | "Use Title File Names" toggle button |
| Serialization | `useTitleFileNames` checked → `!use_serial` | `use_title_filenames` → `!use_serial` (inverted!) |

**Hidden detail**: Both UIs invert the user's checkbox/toggle to get `use_serial` — the UI shows "use title filenames" but the backend uses "use serial".

## 11. Edge Cases Handled

1. **Empty URL list**: Start button disabled
2. **Thread already running**: Old thread cancelled before starting new
3. **Close while downloading**: Thread cancelled via `closeEvent()`
4. **Cover exists during download**: Skipped (checked before each request)
5. **Content-Type missing**: Falls back to URL extension or `cover.jpg`
6. **Download failed (non-200)**: Silently skipped (no error per-file)
7. **Directory doesn't exist**: Created automatically
8. **Interrupted thread**: `requestInterruption()` + `join()`

## 12. Missing from Slint UI (Gaps)

| Feature | Status |
|---------|--------|
| Cover download dialog | ❌ Not implemented |
| URL template input | ❌ Missing |
| Title/serial filename toggle | ❌ Missing |
| Download progress | ❌ Missing |
| Cover refresh signal | ❌ Missing |
| Cover path resolution (4-level lookup) | ❌ Missing |
| Auto-skip existing covers | ❌ Missing |
| Content-Type extension detection | ❌ Missing |
| Secrets settings persistence | ❌ Missing |
| Rate-limited refresh (5s) | ❌ Missing |

## Implementation Notes for Slint/Rust

1. **HTTP client**: Use `reqwest` (async) or `ureq` (sync) in Rust
2. **Thread**: Use `tokio` async task or `std::thread` with channel for progress
3. **Settings storage**: Use `serde` + `toml` or `rsecret` crate for encrypted storage
4. **Cover path resolution**: Implement 4-level lookup (file_title → serial → title → title_en) for `.jpg`, `.jpeg`, `.png`, `.webp`
5. **Progress callback**: Use `mpsc::channel` to send status/progress from download thread to UI
6. **URL encoding**: Use `urlencoding` crate or `url::form_urlencoded`
7. **Content-Type parsing**: Map MIME types to extensions (`image/jpeg` → `.jpg`, `image/png` → `.png`, `image/webp` → `.webp`)
