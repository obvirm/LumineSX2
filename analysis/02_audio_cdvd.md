# Analisis Eksternal Dependencies — Audio & CDVD

## 1. Host/AudioStream.cpp

### SoundTouch (pitch/tempo audio stretching)
**Header:** `#include "SoundTouch.h"`
**Namespace:** `soundtouch::SoundTouch`

**API Calls:**
```
m_soundtouch = std::make_unique<soundtouch::SoundTouch>();
m_soundtouch->setSampleRate(m_sample_rate);
m_soundtouch->setChannels(m_internal_channels);
m_soundtouch->setSetting(SETTING_USE_QUICKSEEK, ...);
m_soundtouch->setSetting(SETTING_USE_AA_FILTER, ...);
m_soundtouch->setSetting(SETTING_SEQUENCE_MS, ...);
m_soundtouch->setSetting(SETTING_SEEKWINDOW_MS, ...);
m_soundtouch->setSetting(SETTING_OVERLAP_MS, ...);
m_soundtouch->setTempo(m_nominal_rate);
m_soundtouch->putSamples(block, CHUNK_SIZE);
m_soundtouch->receiveSamples(m_staging_buffer, CHUNK_SIZE);
m_soundtouch->clear();
```

**Fungsi:** Audio timestretching — menyesuaikan pitch/tempo tanpa mengubah sample rate. Dipakai untuk seamless audio saat emulasi speed != 100%.

**Rust replacement:** `rubato` crate (WindowedInterpolator, SincInterpolator) atau `petgraph`. Rubato punya:
- `Rubato::new()` — setup
- `process()` — putSamples / receiveSamples equivalent
- Channel count, sample rate configuration

**Kompleksitas port:** Medium (rubato API berbeda tapi konsep sama)

### FreeSurroundDecoder
**Header:** `#include "FreeSurroundDecoder.h"`
- Class `FreeSurroundDecoder` — surround upmixing (stereo → 5.1/7.1)

**Rust replacement:** Custom DSP atau `dasp` crate untuk filter audio

### AudioStream base class (self-contained)
- Pure virtual class, ~80 method signatures
- Semua internal logic pake std::vector, std::atomic, template
- Method: `ReadFrames`, `WriteFrames`, `SetPaused`, `BaseInitialize`

---

## 2. Host/CubebAudioStream.cpp

### cubeb (audio output — by Mozilla)
**Header:** `#include "cubeb/cubeb.h"`

**API Calls:**
```
cubeb_set_log_callback(CUBEB_LOG_NORMAL, LogCallback);
cubeb_init(&m_context, "PCSX2", driver_name);
cubeb_destroy(m_context);

cubeb_get_min_latency(m_context, &params, &min_latency_frames);

cubeb_enumerate_devices(m_context, CUBEB_DEVICE_TYPE_OUTPUT, &devices);
cubeb_device_collection_destroy(m_context, &devices);

cubeb_stream_init(m_context, &stream, name, NULL, NULL, device,
    &params, latency_frames, DataCallback, StateCallback, this);
cubeb_stream_start(stream);
cubeb_stream_stop(stream);
cubeb_stream_destroy(stream);

cubeb_get_backend_names();
```

**Struct/Types:**
- `cubeb*` — context handle
- `cubeb_stream*` — stream handle
- `cubeb_stream_params { format, rate, channels, layout, prefs }`
- `cubeb_devid` — device ID
- `cubeb_device_info`, `cubeb_device_collection`
- `cubeb_state` (CUBEB_STATE_STARTED/STOPPED/DRAINED/ERROR)
- Callbacks: `long (*DataCallback)(...)`, `void (*StateCallback)(...)`

**Windows-specific:**
- `CoInitializeEx(nullptr, COINIT_MULTITHREADED)` — COM init
- `wil::unique_couninitialize_call` — COM cleanup guard

**Rust replacement:** `cpal` crate
| cubeb | cpal |
|-------|------|
| `cubeb_init()` | `Host::create()` |
| `cubeb_stream_init()` | `OutputStream::new()` / `InputStream::new()` |
| `cubeb_stream_start()` | `.play()` |
| `cubeb_stream_stop()` | `.pause()` |
| `DataCallback` | `move |data: &mut [f32], _: &cpal::OutputCallbackInfo|` |
| `cubeb_enumerate_devices()` | `cpal::devices()` / `default_output_device()` |
| `cubeb_get_min_latency()` | `config.buffer_size` |

**Kompleksitas port:** Rendah (cpal API lebih simpel, callback-based)

---

## 3. Host/SDLAudioStream.cpp

### SDL3 Audio
**Header:** `#include <SDL3/SDL.h>`

**API Calls:**
```
SDL_SetHint("SDL_AUDIO_DEVICE_APP_NAME", "PCSX2");
SDL_InitSubSystem(SDL_INIT_AUDIO);
SDL_QuitSubSystem(SDL_INIT_AUDIO);

SDL_AudioSpec spec = {SDL_AUDIO_F32, m_output_channels, sample_rate};
SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &spec, AudioCallback, this);

SDL_GetAudioDeviceFormat(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &obtained_spec, &obtained_samples);
SDL_ResumeAudioDevice(SDL_GetAudioStreamDevice(m_stream));
SDL_PauseAudioDevice(SDL_GetAudioStreamDevice(m_stream));
SDL_DestroyAudioStream(m_stream);

SDL_AudioStream* m_stream;
// Callback: static void AudioCallback(void* userdata, SDL_AudioStream* stream, int additional_amount, int total_amount)
SDL_PutAudioStreamData(stream, buffer, additional_amount);
SDL_stack_alloc / SDL_stack_free
```

**Fungsi:** Alternatif backend audio selain cubeb. Hampir semua fungsi spesifik SDL3 Audio.

**Rust replacement:** `cpal` crate (sama seperti cubeb — cukup 1 backend)

**Kompleksitas port:** Rendah (SDL3 audio bisa langsung diganti cpal)

---

## 4. CDVD/ChdFileReader.cpp

### libchdr (CHD compressed disc image)
**Header:** `#include "libchdr/chd.h"`

**API Calls:**
```
chd_open_core_file(core_file, CHD_OPEN_READ, parent_chd, &chd);
chd_close(chd);
chd_get_header(chd) -> const chd_header*
chd_read(chd, chunk_id, dst);
chd_read_header_file(fp, &header);
chd_core_file(chd);
chd_error_string(err);
chd_get_metadata(chd, tag, index, buf, size, &len, NULL, NULL);
```

**Struct/Types:**
- `chd_file*`
- `chd_header { hunkbytes, unitbytes, unitcount, md5, sha1, parentmd5, parentsha1 }`
- `chd_error` enum: `CHDERR_NONE`, `CHDERR_REQUIRES_PARENT`, etc.
- `core_file { argp, fsize, fread, fclose, fseek }` — VTable untuk I/O

**Pattern:** Custom `core_file` wrapper class (`ChdCoreFileWrapper`) yang menyediakan callbacks `fsize/fread/fclose/fseek`. Ini adalah pattern adapter.

**Rust replacement:** Crate `chd` (https://crates.io/crates/chd) — TAPI perlu dicek apakah support CHD v5 (yang dipake PCSX2). Alternatif: bungkus libchdr C via FFI.

**Kompleksitas port:** Medium (butuh CHD parser atau FFI ke libchdr)

### xxhash
**Header:** `#include "xxhash.h"`
- Dipakai untuk hash cache lookup (query hash dari CHD header)
- Rust: `xxhash-rust` crate

### fmt (format string)
**Header:** `#include "fmt/format.h"`
- `fmt::format(...)` — string formatting
- Rust: `format!()` built-in, std::fmt

---

## 5. CDVD/CsoFileReader.cpp

### zlib (compression)
**Header:** `#include <zlib.h>`

**API Calls:**
```
inflateInit2(&m_z_stream, -15);   // raw inflate (no zlib/gzip header)
inflate(&m_z_stream, Z_FINISH);
inflateReset(&m_z_stream);
inflateEnd(&m_z_stream);
```

**Struct:** `z_stream { next_in, avail_in, next_out, avail_out, total_out, zalloc, zfree, opaque }`

**Fungsi:** Decompress CSO frames. CSO = Compressed ISO format pake deflate/zlib.

**Rust replacement:** `flate2` crate
```rust
use flate2::Decompress;
let mut d = Decompress::new(false); // false = raw deflate
d.decompress(input, output, FlushDecompress::Finish)?;
```

### lz4
**Header:** `#include "lz4.h"`

**API Calls:**
```
LZ4_decompress_safe_partial(src, dst, src_size, dst_size, dst_size);
```

**Fungsi:** Decompress ZSO frames (varian CSO pake LZ4 instead of zlib).

**Rust replacement:** `lz4` crate (lz4_flex lebih ringan)
```rust
use lz4_flex::decompress;
let result = decompress(src, dst_size)?;
```

### fmt
Sama seperti ChdFileReader — dipakai untuk logging.

---

## 6. CDVD/zlib_indexed.h

### zlib random access index (self-contained)
**Header:** `#include <zlib.h>`

**API Calls:**
```
inflateInit2(&strm, 47);      // auto zlib/gzip detection
inflate(&strm, Z_BLOCK);      // block-level inflate
inflateInit2(&strm, -15);     // raw inflate
inflatePrime(&strm, bits, value);
inflateSetDictionary(&strm, window, WINSIZE);
inflateEnd(&strm);
```

**Structures:**
- `point { out, in, bits, window[32K] }` — access point entry
- `access { have, size, list, span, uncompressed_size }` — index
- `zstate { out_offset, in_offset, strm, isValid }` — streaming state

**Fungsi:** Indexed random access ke compressed zlib/gzip stream. Build index (access points ≈ every SPAN bytes), lalu bisa seek ke offset tertentu tanpa decompress seluruh file. Penting untuk fast loading compressed disc images.

**Pattern unik:** `Z_BLOCK` buat deteksi block boundary, `inflatePrime` buat bit-level positioning, `inflateSetDictionary` buat restore sliding window.

**Rust replacement:** Kombinasi `flate2` + manual indexing. Atau cari crate `zran` (tidak ada di Rust ecosystem). Perlu implement ulang custom.

**Kompleksitas port:** Tinggi (algoritma indexing kompleks)

---

## Ringkasan Audio/CDVD External Libraries

| Library | Files | Rust Crate | Kompleksitas |
|---------|-------|-----------|-------------|
| **SoundTouch** | AudioStream.cpp | `rubato` | Medium |
| **FreeSurround** | AudioStream.cpp | `dasp` atau custom | Medium |
| **cubeb** | CubebAudioStream.cpp | `cpal` | Rendah |
| **SDL3 Audio** | SDLAudioStream.cpp | `cpal` | Rendah |
| **libchdr** | ChdFileReader.cpp | `chd` (crate) | Medium |
| **xxhash** | ChdFileReader.cpp | `xxhash-rust` | Rendah |
| **zlib** | CsoFileReader.cpp, zlib_indexed.h | `flate2` | Rendah-Medium |
| **lz4** | CsoFileReader.cpp | `lz4_flex` | Rendah |
| **fmtlib** | ChdFileReader.cpp, CsoFileReader.cpp | std::fmt built-in | none |

---

## Key Insight untuk Rust Port

1. **Audio backend cukup 1:** PCSX2 punya 3 backend (Null, Cubeb, SDL3). Di Rust cukup `cpal` — modern, cross-platform, callback-based. Tidak perlu multiple backends.

2. **CHD reader tersedia di Rust ecosystem:** Crate `chd` by TeamPks (v5 support). Cek apakah cocok.

3. **zlib_indexed.h adalah blocker:** Random access ke compressed stream butuh implementasi Rust baru. Ini critical path untuk CDVD loading speed.

4. **SoundTouch → rubato:** API berbeda. Rubato pake `process()` dengan audio buffer chunks, bukan `putSamples/receiveSamples`. Perlu adapter.

5. **Platform-specific COM init:** CubebAudioStream pake `CoInitializeEx` + `wil::unique_couninitialize_call` — di Rust handle via `CoInitializeEx` FFI + Drop guard.
