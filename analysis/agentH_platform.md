# Agent H: HostSys + Platform-specific

### common/HostSys.cpp/.h
- C++: 181 + 209 = 390 lines
- Rust: 888 lines ✅
- Rust pub fn:
  - 63:    pub struct SYSTEM_INFO {
  - 79:    pub struct MEMORYSTATUSEX {
  - 94:    pub struct GROUP_AFFINITY {
  - 105:    pub struct CACHE_RELATIONSHIP {
  - 118:    pub struct PROCESSOR_RELATIONSHIP {
  - 128:    pub struct NUMA_NODE_RELATIONSHIP {
  - 140:    pub struct SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX {
  - 159:        pub fn GetSystemInfo(lpSystemInfo: *mut SYSTEM_INFO);
  - 160:        pub fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> BOOL;
  - 161:        pub fn QueryPerformanceCounter(lpPerformanceCount: *mut i64) -> BOOL;

### common/HostSys.cpp/.h
- C++: 181 + 209 = 390 lines
- Rust: 55 lines ✅
- Rust pub fn:

### common/Darwin/DarwinMisc.cpp/.h
- C++: 642 + 24 = 666 lines
- Rust: 492 lines ✅
- Rust pub fn:
  - 90:        pub fn IOPMAssertionCreateWithName(
  - 97:        pub fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> i32;
  - 104:        pub fn CFStringCreateWithCString(
  - 119:        pub fn CFRelease(cf: *const c_void);
  - 165:pub fn inhibit_screensaver(inhibit: bool) -> bool {
  - 229:pub fn play_sound_async(path: &Path) -> bool {
  - 287:pub fn set_mouse_position(x: i32, y: i32) {
  - 314:pub fn get_program_path() -> PathBuf {
  - 361:pub fn set_path_compression(_path: &mut PathBuf) -> bool {
- 🍎 macOS-specific

### common/Darwin/DarwinThreads.cpp/.h
- C++: 279 + 0 = 279 lines
- Rust: 231 lines ✅
- Rust pub fn:
  - 84:    pub fn timeslice() {
  - 94:    pub fn spin_wait() {
  - 114:    pub fn sleep(ms: u32) {
  - 123:    pub fn sleep_until(deadline: Instant) {
  - 136:    pub fn get_thread_cpu_time() -> u64 {
  - 171:    pub fn set_name_of_current_thread(name: &str) {
- 🍎 macOS-specific

### common/Linux/LnxHostSys.cpp/.h
- C++: 352 + 0 = 352 lines
- Rust: 973 lines ✅
- Rust pub fn:
  - 140:    pub fn mem_protect(
  - 168:    pub fn create_shared_memory(name: &str, size: usize) -> *mut u8 {
  - 212:    pub fn destroy_shared_memory(ptr: *mut u8, _size: usize) {
  - 227:    pub fn map_shared_memory(
  - 267:    pub fn unmap_shared_memory(map_base: *mut u8, map_size: usize) -> bool {
  - 293:    pub struct SharedMemoryMappingArea {
  - 310:        pub fn create(size: usize) -> Option<Self> {
  - 334:        pub fn base_ptr(&self) -> *mut u8 {
  - 339:        pub fn size(&self) -> usize {
  - 345:        pub fn map(
- 🐧 Linux-specific

### common/Linux/LnxMisc.cpp/.h
- C++: 380 + 0 = 380 lines
- Rust: 421 lines ✅
- Rust pub fn:
  - 81:pub fn inhibit_screensaver(inhibit: bool) -> bool {
  - 160:pub fn play_sound_async(path: &Path) -> bool {
  - 226:pub fn set_mouse_position(x: i32, y: i32) {
  - 260:pub fn attach_mouse_position_cb(cb: Box<dyn Fn(i32, i32) + Send + 'static>) {
  - 277:pub fn detach_mouse_position_cb() {
- 🐧 Linux-specific

### common/Linux/LnxThreads.cpp/.h
- C++: 348 + 0 = 348 lines
- Rust: 361 lines ✅
- Rust pub fn:
  - 60:pub fn get_thread_cpu_time() -> u64 {
  - 83:pub fn get_thread_ticks_per_second() -> u64 {
  - 95:pub fn set_name_of_current_thread(name: &str) {
  - 117:pub fn timeslice() {
  - 127:pub fn spin_wait() {
  - 138:pub fn enable_hires_scheduler() {
  - 150:pub fn disable_hires_scheduler() {
  - 155:pub fn sleep(ms: u32) {
  - 168:pub fn sleep_until(ticks: u64) {
- 🐧 Linux-specific

### common/CocoaTools.cpp/.h
- C++: 0 + 45 = 45 lines
- Rust: 277 lines ✅
- Rust pub fn:
  - 86:    pub fn create_window(title: &str, width: u32, height: u32) -> WindowHandle {
  - 137:    pub fn destroy_window(window: WindowHandle) {
  - 152:    pub fn run_event_loop(forever: bool) {
  - 190:    pub fn stop_event_loop() {
  - 218:    pub fn create_window(_title: &str, _width: u32, _height: u32) -> WindowHandle {
  - 221:    pub fn destroy_window(_window: WindowHandle) {}
  - 222:    pub fn run_event_loop(_forever: bool) {}
  - 223:    pub fn stop_event_loop() {}
- 🍎 macOS-specific (ObjC)


### Platform-Specific: PrecompiledHeader

- MSVC PCH optimization. Tidak perlu Rust.
✅ Agent H selesai
