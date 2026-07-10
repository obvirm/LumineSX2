# Agent 49: GameListRefreshThread — Deep Analysis

## Files Analyzed
- `GameList/GameListRefreshThread.h`
- `GameList/GameListRefreshThread.cpp`

---

## Architecture Overview

Two classes: `AsyncRefreshProgressCallback` (progress bridge) and `GameListRefreshThread` (background thread).

The thread delegates all work to `GameList::Refresh(invalidate_cache, false, &m_progress)` and emits `refreshComplete()` when done.

---

## Class: `GameListRefreshThread`

### Constructor Parameters
| Parameter | Type | Purpose |
|-----------|------|---------|
| `invalidate_cache` | `bool` | When true, forces full rescan ignoring cached data |
| `popup_on_error` | `bool` | When true, shows error dialogs; otherwise logs to console |

### Signals
| Signal | Parameters | When Emitted |
|--------|-----------|-------------|
| `refreshProgress` | `QString status, int current, int total` | On every meaningful progress update (status text, range, or value change) |
| `refreshComplete` | *(none)* | When `GameList::Refresh()` returns |

### Methods
| Method | Visibility | Purpose |
|--------|-----------|---------|
| `cancel()` | public | Calls `m_progress.Cancel()` to set cancelled flag |
| `run()` | protected (QThread) | The thread entry point — calls `GameList::Refresh()` |

### Cache Invalidation
- `m_invalidate_cache` bool passed directly to `GameList::Refresh(invalidate_cache, false, &m_progress)`
- The second `false` parameter = don't add new entries only (full refresh)

---

## Class: `AsyncRefreshProgressCallback` (extends `BaseProgressCallback`)

### Purpose
Thread-safe bridge between the scanning engine (`GameList::Refresh`) and the Qt UI thread (via signals through parent `GameListRefreshThread`).

### Constructor Parameters
| Parameter | Purpose |
|-----------|---------|
| `popup_on_error` | Controls whether errors show as dialogs or just log |
| `parent` | Back-pointer to fire signals |

### Progress Callback Methods Implemented

| Method | Behavior |
|--------|----------|
| `SetStatusText(const char* text)` | Converts to `QString`, skips if unchanged, fires update |
| `SetProgressRange(u32 range)` | Stores range via base class, fires update if changed |
| `SetProgressValue(u32 value)` | Stores value via base class, fires update if changed |
| `SetTitle(const char* title)` | **NO-OP** — ignored |
| `DisplayError(const char* message)` | If `m_popup_on_error`: `Host::ReportErrorAsync()`; else `ERROR_LOG` |
| `DisplayWarning(const char* message)` | **Not implemented** — `pxFailRel` assertion |
| `DisplayInformation(const char* message)` | **Not implemented** — `pxFailRel` assertion |
| `DisplayDebugMessage(const char* message)` | Logs via `qDebug()` |
| `ModalError(const char* message)` | **Not implemented** — `pxFailRel` assertion |
| `ModalConfirmation(const char* message)` | **Not implemented** — `pxFailRel` assertion |
| `ModalInformation(const char* message)` | **Not implemented** — `pxFailRel` assertion |
| `Cancel()` | Sets `m_cancelled = true` (non-atomic, lazy cancellation) |

### Fire Update Mechanism
```cpp
void AsyncRefreshProgressCallback::fireUpdate()
{
    m_parent->refreshProgress(m_status_text, m_last_value, m_last_range);
}
```
- Called only when a value actually changes (deduplication via `m_last_*` comparison)
- Uses `Common::Timer m_last_update_time` — declared but **not actually used for throttling** in the current code (potential future throttle)

---

## Background Scanning Flow

1. **Caller creates** `GameListRefreshThread(invalidate_cache, popup_on_error)`
2. **Caller connects** to `refreshProgress` and `refreshComplete` signals
3. **Caller starts thread** via `QThread::start()`
4. **Thread runs** `GameList::Refresh(invalidate_cache, false, &m_progress)`
5. During scanning, engine calls progress callback methods:
   - `SetStatusText("Scanning /path/to/iso...")`
   - `SetProgressRange(total_files)`
   - `SetProgressValue(current_file_index)`
6. Each meaningful change fires `refreshProgress` signal across thread boundary
7. **On completion** (success or error): `refreshComplete` signal emitted
8. **Cancellation**: `cancel()` sets `m_cancelled` flag; scanning engine checks this periodically and exits early

---

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Scan error (bad file, corrupt ISO) | `DisplayError()` called — shows popup or logs |
| Warning during scan | **Assertion failure** — not used by engine in practice |
| Modal dialog needed | **Assertion failure** — not used by engine in practice |
| Cancelled by user | Lazy cancel via bool flag, engine checks periodically |

---

## Hidden Features & Notes

1. **`m_last_update_time` (Common::Timer)** — declared but never read. This is scaffolding for potential rate-limiting of UI updates (e.g., max 30fps progress updates). Currently every change fires immediately.

2. **Lazy cancellation** — comment says "Not atomic, but we don't need to cancel immediately." The cancel flag is a plain `bool`, not `std::atomic<bool>`. Safe because it's only written from UI thread and read from worker thread with happens-before from the thread lifecycle.

3. **`pxFailRel` on unused methods** — Warning/Information/Modal methods are assertion-failures. This means the scanning engine (`GameList::Refresh`) never calls these. If it did, it would crash in debug builds and silently misbehave in release.

4. **Deduplication** — `SetStatusText`, `SetProgressRange`, `SetProgressValue` all skip the signal fire if the value hasn't changed. This prevents UI thrashing.

5. **Thread owns progress callback** — `m_progress` is a member, not heap-allocated. Destroyed with the thread. The back-pointer to parent is valid for the thread's lifetime.

6. **`refreshComplete` emitted unconditionally** — whether scan succeeded, was cancelled, or errored. Caller must check state separately.

---

## Features Extracted

| # | Feature | Status |
|---|---------|--------|
| 1 | Background game list scanning | ✅ Fully implemented |
| 2 | Progress reporting (status + current + total) | ✅ Fully implemented |
| 3 | Cache invalidation toggle | ✅ Via constructor param |
| 4 | Error popup toggle | ✅ Via `popup_on_error` param |
| 5 | Cancellation support | ✅ Lazy bool flag |
| 6 | Progress deduplication | ✅ Skip unchanged values |
| 7 | Rate limiting on progress updates | ❌ Timer declared but unused |
| 8 | Warning/Info/Modal callbacks | ❌ Not implemented (assert-fail) |
| 9 | Debug message logging | ✅ Via `qDebug()` |
| 10 | Thread-safe signal emission | ✅ Qt cross-thread signal |

---

## Slint UI Implications

For the LumineSX2 Slint UI, this analysis suggests:

1. **Need a background scan thread** — Rust equivalent using `std::thread` + channel
2. **Progress model** — `{ status_text: string, current: int, total: int }` property in Slint
3. **Signals/Callbacks** — `refresh-progress(status, current, total)` and `refresh-complete` callbacks
4. **Cache invalidation** — `scan_games(invalidate_cache: bool)` function
5. **Cancellation** — `cancel_scan()` sets atomic flag
6. **Error handling** — Log-only model (no modal dialogs in async scan)
7. **Rate limiting** — Consider throttling to ~30fps for UI updates during scan
