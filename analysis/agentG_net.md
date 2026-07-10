# Agent G: HTTPDownloader + MD5Digest + ProgressCallback + Perf + ZipHelpers + LRUCache

### common/HTTPDownloader.cpp/.h
- C++: 321 + 98 = 419 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 8:#include <atomic>
  - 9:#include <functional>
  - 10:#include <memory>

### common/HTTPDownloaderCurl.cpp/.h
- C++: 206 + 37 = 243 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 12:#include <algorithm>
  - 13:#include <functional>
  - 14:#include <pthread.h>
  - 15:#include <signal.h>
  - 8:#include <atomic>
  - 9:#include <memory>
  - 10:#include <mutex>

### common/HTTPDownloaderWinHTTP.cpp/.h
- C++: 324 + 40 = 364 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 10:#include <VersionHelpers.h>
  - 11:#include <algorithm>
  - 9:#include <winhttp.h>

### common/MD5Digest.cpp/.h
- C++: 210 + 20 = 230 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 5:#include <cstring>

### common/ProgressCallback.cpp/.h
- C++: 245 + 108 = 353 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 10:#include <cmath>
  - 11:#include <cstdio>
  - 12:#include <limits>
  - 7:#include <memory>
  - 8:#include <string>

### common/Perf.cpp/.h
- C++: 215 + 31 = 246 lines
- Rust: 82 lines ✅
- Rust pub types:
  - 10:pub enum PerfGroup {
  - 20:pub struct PerfScope {
  - 26:    pub fn new(group: PerfGroup) -> Self {
  - 41:pub fn perf_init() -> bool {
  - 46:pub fn perf_shutdown() {}
  - 49:pub fn perf_register(_group: PerfGroup, _addr: *const std::ffi::c_void, _size: u64, _name: *const std::ffi::c_char) {}
  - 52:pub fn perf_register_pc(_group: PerfGroup, _addr: *const std::ffi::c_void, _size: u64, _pc: u32) {}
  - 55:pub fn perf_register_key(_group: PerfGroup, _addr: *const std::ffi::c_void, _size: u64, _key: u64, _name: *const std::ffi::c_char) {}
- C++ external:
  - 13:#include <array>
  - 14:#include <cstring>
  - 17:#include <atomic>
  - 18:#include <ctime>
  - 19:#include <mutex>
  - 6:#include <vector>
  - 7:#include <cstdio>
  → Rust: perf.rs is stub

### common/LRUCache.cpp/.h
- Header-only: 122 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 6:#include <algorithm>
  - 7:#include <cstdint>
  - 8:#include <map>

### common/ZipHelpers.cpp/.h
- Header-only: 141 lines
- ❌ Rust NOT FOUND
- C++ external:
  - 5:#include <memory>
  - 6:#include <optional>
  - 7:#include <string>

✅ Agent G selesai
