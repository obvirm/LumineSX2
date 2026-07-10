# Input Recording System - Deep Analysis

## Files Analyzed
- `pcsx2-qt/Tools/InputRecording/NewInputRecordingDlg.h/.cpp`
- `pcsx2-qt/Tools/InputRecording/InputRecordingViewer.h/.cpp`
- `pcsx2/Recording/InputRecording.h`
- `pcsx2/Recording/InputRecordingControls.h`
- `pcsx2/Recording/InputRecordingFile.h`
- `pcsx2/Recording/PadData.h`

---

## 1. New Input Recording Dialog (`NewInputRecordingDlg`)

### UI Elements
| Element | Type | ID | Description |
|---------|------|-----|-------------|
| Radio: Power On | `QRadioButton` | `m_recTypePowerOn` | Start recording from fresh boot |
| Radio: From Save State | `QRadioButton` | `m_recTypeSaveState` | Start recording from savestate |
| Warning label | `QLabel` | `m_recTypeWarning` | Shown only when savestate mode selected |
| File path input | `QLineEdit` | `m_filePathInput` | Disabled (read-only), set via browse |
| Browse button | `QPushButton` | `m_filePathBrowseBtn` | Opens save file dialog (*.p2m2) |
| Author input | `QLineEdit` | `m_authorInput` | Author name for the recording |
| OK/Cancel buttons | `QDialogButtonBox` | `m_dlgBtns` | OK disabled until form valid |

### Recording Types (enum `InputRecording::Type`)
- **POWER_ON** — Recording starts from fresh game boot. Default selection.
- **FROM_SAVESTATE** — Recording starts from current savestate. Shows warning label.

### Form Validation
- OK button is **disabled by default**
- OK enabled only when: `!filePath.isEmpty() && !authorName.isEmpty()`
- File filter: `Input Recording Files (*.p2m2)`
- Uses `QFileDialog::getSaveFileName` (native dialog)

### Signals/Callbacks
| Signal | Slot | Action |
|--------|------|--------|
| `m_recTypePowerOn.clicked(bool)` | `onRecordingTypePowerOnChecked` | Set type=POWER_ON, hide warning |
| `m_recTypeSaveState.checked(bool)` | `onRecordingTypeSaveStateChecked` | Set type=FROM_SAVESTATE, show warning |
| `m_filePathBrowseBtn.clicked()` | `onBrowseForPathClicked` | Open save dialog, update path |
| `m_authorInput.textEdited(QString)` | `onAuthorNameChanged` | Update author, revalidate |

### Return Values (from dialog exec)
- `getInputRecType()` → `InputRecording::Type`
- `getFilePath()` → `std::string`
- `getAuthorName()` → `std::string`

---

## 2. Input Recording Viewer (`InputRecordingViewer`)

### Window Type
- `QMainWindow` (not dialog — has menu bar)

### UI Elements
| Element | Type | ID | Description |
|---------|------|-----|-------------|
| Menu: Open | `QAction` | `actionOpen` | Open .p2m2 file for viewing |
| Menu: Close | `QAction` | `actionClose` | Close current file, disabled until file open |
| Table widget | `QTableWidget` | `tableWidget` | Frame-by-frame input data display |

### Table Columns (18 columns)
| Index | Column | Data Type |
|-------|--------|-----------|
| 0 | Left Analog | `<u8, u8>` (x, y) |
| 1 | Right Analog | `<u8, u8>` (x, y) |
| 2 | Cross | `bool` + `u8` pressure |
| 3 | Square | `bool` + `u8` pressure |
| 4 | Triangle | `bool` + `u8` pressure |
| 5 | Circle | `bool` + `u8` pressure |
| 6 | L1 | `bool` + `u8` pressure |
| 7 | L2 | `bool` + `u8` pressure (NOTE: code has index 7=L2, 8=R1 — bug in original?) |
| 8 | R1 | `bool` + `u8` pressure |
| 9 | R2 | `bool` + `u8` pressure |
| 10 | D-Pad Down | `bool` + `u8` pressure |
| 11 | D-Pad Right | `bool` + `u8` pressure |
| 12 | D-Pad Up | `bool` + `u8` pressure |
| 13 | D-Pad Left | `bool` + `u8` pressure |
| 14 | L3 | `bool` (no pressure) |
| 15 | R3 | `bool` (no pressure) |
| 16 | Select | `bool` (no pressure) |
| 17 | Start | `bool` (no pressure) — **BUG: code reads `m_select` not `m_start`** |

### Display Format
- Analog: `"127 127"` (space-separated x y)
- Button with pressure: `"true [128]"` or `"false [0]"`
- Simple button: `"true"` or `"false"`

### Selection Mode
- `QAbstractItemView::NoSelection` — read-only display

### Data Loading
- `loadTable()` calls `m_file.bulkReadPadData(0, totalFrames, 0)` — **only reads port 0**
- Naive implementation: loads ALL frames at once (TODO in code: replace with lazy-loading QTableView)

---

## 3. InputRecording Core (`InputRecording`)

### Singleton
- `extern InputRecording g_InputRecording;`

### Recording Modes (InputRecordingControls::Mode)
- **Recording** — capturing new input
- **Replaying** — playing back existing input

### Key Methods
| Method | Description |
|--------|-------------|
| `create(filename, fromSaveState, authorName)` | Start new recording |
| `play(path)` | Load and play existing recording |
| `stop()` | Stop current recording |
| `toggleRecordMode()` | Switch between Record/Replay |
| `isRecording()` / `isReplaying()` | Query current mode |
| `isActive()` | Whether recording is active |
| `getFrameCounter()` | Current frame number |
| `incFrameCounter()` | Increment frame counter |
| `handleExceededFrameCounter()` | Called when playback reaches end |
| `handleReset()` | Handle system reset |
| `handleLoadingSavestate()` | Handle savestate load during recording |
| `isTypeSavestate()` | Whether started from savestate |
| `adjustFrameCounterOnReRecord()` | Adjust counter on re-record |
| `processRecordQueue()` | Process queued recording actions |

### State
- `m_type` — POWER_ON or FROM_SAVESTATE
- `m_initial_load_complete` — first frame loaded
- `m_is_active` — recording active
- `m_watching_for_rerecords` — monitoring for re-records
- `m_frame_counter` — current frame
- `m_frame_counter_stateless` — frame counter without savestate offset
- `m_starting_frame` — 0 for power-on, g_FrameCount for savestate

### Queue System
- `m_recordingQueue` — `std::queue<std::function<void()>>` for deferred actions
- `processControlQueue()` — processes control commands at frame boundary

---

## 4. InputRecordingFile — File Format

### Header Structure (`InputRecordingFileHeader`)
| Field | Type | Size | Description |
|-------|------|------|-------------|
| `m_fileVersion` | `u8` | 1 byte | Always 1 (v1 format) |
| `m_emulatorVersion` | `char[50]` | 50 bytes | PCSX2 version string |
| `m_author` | `char[255]` | 255 bytes | Author name |
| `m_gameName` | `char[255]` | 255 bytes | Game name from CDROM |

### File Layout
```
[Header (561 bytes)] [TotalFrames (4 bytes)] [UndoCount (4 bytes)] [SavestateHeader (1 byte)] [FrameData...]
```

### Constants
- `s_controllerPortsSupported = 2`
- `s_controllerInputBytes = 18` (per port)
- `s_inputBytesPerFrame = 36` (18 × 2 ports)
- `s_headerSize = 569` (header + totalFrames + undoCount)

### Seek Points
| Seek Point | Offset |
|------------|--------|
| Total Frames | `sizeof(Header)` = 561 |
| Undo Count | `sizeof(Header) + 4` = 565 |
| Savestate Header | `sizeof(Header) + 8` = 569 |

### File Extension
- `.p2m2` (version 2 of the format)

---

## 5. PadData — Controller Data Structure

### Per-Frame Data (18 bytes per port)
| Field | Type | Range | Description |
|-------|------|-------|-------------|
| `m_leftAnalog` | `<u8,u8>` | 0-255 (127=center) | Left stick X,Y |
| `m_rightAnalog` | `<u8,u8>` | 0-255 (127=center) | Right stick X,Y |
| `m_circle` | `<bool,u8>` | pressed, 0-255 | Circle button |
| `m_cross` | `<bool,u8>` | pressed, 0-255 | Cross button |
| `m_square` | `<bool,u8>` | pressed, 0-255 | Square button |
| `m_triangle` | `<bool,u8>` | pressed, 0-255 | Triangle button |
| `m_up/down/left/right` | `<bool,u8>` | pressed, 0-255 | D-Pad |
| `m_l1/l2/r1/r2` | `<bool,u8>` | pressed, 0-255 | Shoulder buttons |
| `m_start` | `bool` | pressed | Start |
| `m_select` | `bool` | pressed | Select |
| `m_l3` | `bool` | pressed | L3 (stick press) |
| `m_r3` | `bool` | pressed | R3 (stick press) |
| `m_compactPressFlagsGroupOne/Two` | `u8` | 0-255 | Compact flags for quick read |

### Constants
- `ANALOG_VECTOR_NEUTRAL = 127` — center position

### Methods
- `OverrideActualController()` — Overwrite live controller state with recorded data
- `LogPadData()` — Debug output to controller log filter

---

## 6. UI Design for Slint Implementation

### New Recording Dialog Layout
```
┌─────────────────────────────────┐
│ New Input Recording             │
├─────────────────────────────────┤
│ ○ Power On                      │
│ ● From Save State               │
│ ⚠ Warning: savestate mode...   │
│                                 │
│ File Path: [________] [Browse]  │
│ Author:    [________]           │
│                                 │
│         [Cancel] [OK]           │
└─────────────────────────────────┘
```

### Viewer Layout
```
┌─────────────────────────────────┐
│ Input Recording Viewer    [─][□][×]│
├─────────────────────────────────┤
│ File │ Edit │ View              │
├─────────────────────────────────┤
│ Frame │ L.Analog │ R.Analog │ ... │
│   0   │  127 127 │  127 127 │ ... │
│   1   │  130 125 │  127 127 │ ... │
│  ...  │   ...    │   ...    │ ... │
└─────────────────────────────────┘
```

---

## 7. Hidden Features / Edge Cases

1. **Re-record counting** — `incrementUndoCount()` tracks how many times savestate was loaded during recording (TAS feature)
2. **Frame limit** — 32-bit signed = ~1.13 years at 60fps continuous recording
3. **Bulk read** — `bulkReadPadData()` can read arbitrary frame ranges for viewer
4. **Two ports** — Recording supports 2 controller ports simultaneously
5. **Compact press flags** — `m_compactPressFlagsGroupOne/Two` for efficient button state packing
6. **Savestate header** — 1-byte flag indicating if recording uses savestate
7. **TODO in code** — Version 2 planned to move everything into header for simpler access
8. **Bug in viewer** — Column 17 (Start) reads `m_select` instead of `m_start`
9. **Port 0 only** — Viewer currently only displays port 0 data
10. **Naive table fill** — All frames loaded at once, no lazy loading (noted TODO)
