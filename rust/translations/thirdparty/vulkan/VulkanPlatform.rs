//! Idiomatic Rust 2021 translation of the Vulkan platform headers.
//!
//! This module covers `vk_platform.h`, `vk_layer.h`, and the per-platform
//! surface headers (`vulkan_android.h`, `vulkan_ios.h`, `vulkan_macos.h`,
//! `vulkan_metal.h`, `vulkan_wayland.h`, `vulkan_win32.h`, `vulkan_xcb.h`,
//! `vulkan_xlib.h`, `vulkan_fuchsia.h`, `vulkan_screen.h`,
//! `vk_layer_dispatch_table.h`).
//!
//! In Rust the integer typedefs and calling-attribute macros collapse into
//! idiomatic primitives.  Foreign types (HWND, ANativeWindow, wl_display, ...)
//! are exposed as `pub type` aliases to platform-specific opaque pointers.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

// ---------------------------------------------------------------------------
// Calling convention / linkage macros (no-ops on stable Rust).
// ---------------------------------------------------------------------------

/// Vulkan API attribute (`VKAPI_ATTR`).  No-op in Rust.
#[macro_export]
macro_rules! VKAPI_ATTR { () => {}; }

/// Vulkan API call (`VKAPI_CALL`).  No-op in Rust.
#[macro_export]
macro_rules! VKAPI_CALL { () => {}; }

/// Vulkan API pointer (`VKAPI_PTR`).  No-op in Rust.
#[macro_export]
macro_rules! VKAPI_PTR { () => {}; }

// ---------------------------------------------------------------------------
// Standard integer typedefs mirrored from <stdint.h>.
// ---------------------------------------------------------------------------

pub type int8_t = i8;
pub type int16_t = i16;
pub type int32_t = i32;
pub type int64_t = i64;
pub type uint8_t = u8;
pub type uint16_t = u16;
pub type uint32_t = u32;
pub type uint64_t = u64;

pub type int_least8_t = i8;
pub type int_least16_t = i16;
pub type int_least32_t = i32;
pub type int_least64_t = i64;
pub type uint_least8_t = u8;
pub type uint_least16_t = u16;
pub type uint_least32_t = u32;
pub type uint_least64_t = u64;

pub type int_fast8_t = i8;
pub type int_fast16_t = i32;
pub type int_fast32_t = i32;
pub type int_fast64_t = i64;
pub type uint_fast8_t = u8;
pub type uint_fast16_t = u32;
pub type uint_fast32_t = u32;
pub type uint_fast64_t = u64;

pub type intptr_t = isize;
pub type uintptr_t = usize;
pub type intmax_t = i64;
pub type uintmax_t = u64;

pub type size_t = usize;
pub type ptrdiff_t = isize;
pub type wchar_t = u16;

// ---------------------------------------------------------------------------
// VK_NULL_HANDLE - opaque null handle.
// ---------------------------------------------------------------------------

/// Vulkan null handle constant.  On 64-bit targets this matches `0` as `u64`.
pub const VK_NULL_HANDLE: u64 = 0;

// ---------------------------------------------------------------------------
// Platform-specific surface foreign types.
// ---------------------------------------------------------------------------

// ---------- Windows (vulkan_win32.h) ----------

#[cfg(target_os = "windows")]
pub type HWND = *mut core::ffi::c_void;
#[cfg(target_os = "windows")]
pub type HINSTANCE = *mut core::ffi::c_void;
#[cfg(target_os = "windows")]
pub type HANDLE = *mut core::ffi::c_void;
#[cfg(target_os = "windows")]
pub type HMONITOR = *mut core::ffi::c_void;
#[cfg(target_os = "windows")]
pub type LPCWSTR = *const u16;

#[cfg(target_os = "windows")]
#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: u32,
    pub lpSecurityDescriptor: *mut core::ffi::c_void,
    pub bInheritHandle: i32,
}

// ---------- X11 (vulkan_xlib.h / vulkan_xcb.h) ----------

#[cfg(target_family = "unix")]
pub type Display = core::ffi::c_void;
#[cfg(target_family = "unix")]
pub type Window = u64;
#[cfg(target_family = "unix")]
pub type VisualID = u64;
#[cfg(target_family = "unix")]
pub type RROutput = u64;

#[cfg(target_family = "unix")]
pub type xcb_connection_t = core::ffi::c_void;
#[cfg(target_family = "unix")]
pub type xcb_window_t = u32;
#[cfg(target_family = "unix")]
pub type xcb_visualid_t = u32;

// ---------- Wayland (vulkan_wayland.h) ----------

#[cfg(target_os = "linux")]
pub type wl_display = core::ffi::c_void;
#[cfg(target_os = "linux")]
pub type wl_surface = core::ffi::c_void;

// ---------- Android (vulkan_android.h) ----------

#[cfg(target_os = "android")]
pub type ANativeWindow = core::ffi::c_void;
#[cfg(target_os = "android")]
pub type AHardwareBuffer = core::ffi::c_void;

// ---------- iOS / macOS (vulkan_ios.h / vulkan_macos.h) ----------

#[cfg(target_os = "macos")]
pub type CAMetalLayer = core::ffi::c_void;

// ---------- Fuchsia (vulkan_fuchsia.h) ----------

#[cfg(target_os = "fuchsia")]
pub type zx_handle_t = u32;
#[cfg(target_os = "fuchsia")]
pub type image_handle_t = u32;
#[cfg(target_os = "fuchsia")]
pub type buffer_handle_t = u32;

// ---------- QNX Screen (vulkan_screen.h) ----------

#[cfg(target_os = "nto")]
pub type _screen_context = core::ffi::c_void;
#[cfg(target_os = "nto")]
pub type _screen_window = core::ffi::c_void;
#[cfg(target_os = "nto")]
pub type _screen_buffer = core::ffi::c_void;
#[cfg(target_os = "nto")]
pub type screen_context_t = *mut _screen_context;
#[cfg(target_os = "nto")]
pub type screen_window_t = *mut _screen_window;
#[cfg(target_os = "nto")]
pub type screen_buffer_t = *mut _screen_buffer;

// ---------- Generic placeholders (compile on any platform) ----------

#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type HWND = *mut core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type HINSTANCE = *mut core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type HANDLE = *mut core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type HMONITOR = *mut core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type Display = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type Window = u64;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type VisualID = u64;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type RROutput = u64;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type xcb_connection_t = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type xcb_window_t = u32;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type xcb_visualid_t = u32;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type wl_display = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type wl_surface = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type ANativeWindow = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type AHardwareBuffer = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type CAMetalLayer = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type zx_handle_t = u32;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type image_handle_t = u32;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type buffer_handle_t = u32;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type _screen_context = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type _screen_window = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type _screen_buffer = core::ffi::c_void;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type screen_context_t = *mut _screen_context;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type screen_window_t = *mut _screen_window;
#[cfg(not(any(target_os = "windows", target_family = "unix", target_os = "android",
              target_os = "macos", target_os = "fuchsia", target_os = "nto")))]
pub type screen_buffer_t = *mut _screen_buffer;

// ---------------------------------------------------------------------------
// vk_layer.h
// ---------------------------------------------------------------------------

/// `VkLayerFunction` enum from `vk_layer.h`.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VkLayerFunction {
    VK_LAYER_FUNCTION_LINK = 0,
    VK_LAYER_FUNCTION_CREATE_INSTANCE = 1,
    VK_LAYER_FUNCTION_GET_INSTANCE_PROC_ADDR = 2,
    VK_LAYER_FUNCTION_GET_PHYSICAL_DEVICE_PROC_ADDR = 3,
    VK_LAYER_FUNCTION_CREATE_DEVICE = 4,
    VK_LAYER_FUNCTION_GET_DEVICE_PROC_ADDR = 5,
    VK_LAYER_FUNCTION_DESTROY_DEVICE = 6,
    VK_LAYER_FUNCTION_VALIDATION_CACHE_EXT = 7,
    VK_LAYER_FUNCTION_PHYSICAL_DEVICE_MERGE_PROPERTIES_EXT = 8,
    VK_LAYER_FUNCTION_PHYSICAL_DEVICE_COMPUTE_BOUND_PROPERTIES_EXT = 9,
    VK_LAYER_FUNCTION_GET_PHYSICAL_DEVICE_PROPERTIES = 10,
    VK_LAYER_FUNCTION_QUEUE_FAMILY_OWNERSHIP_TRANSFER = 11,
    VK_LAYER_FUNCTION_SET_DEVICE_LOADER_DATA = 12,
    VK_LAYER_FUNCTION_GET_DEVICE_QUEUE = 13,
    VK_LAYER_FUNCTION_INVALID = 1000,
    VK_LAYER_FUNCTION_MAX_ENUM = 0x7FFFFFFF,
}

/// Range metadata for `VkLayerFunction` (cannot live inside the enum in Rust
/// because duplicate discriminant values are forbidden; in C these are enum
/// entries that shadow their `LINK` / `SET_DEVICE_LOADER_DATA` counterparts).
impl VkLayerFunction {
    /// First valid discriminant (= `VK_LAYER_FUNCTION_LINK`).
    pub const VK_LAYER_FUNCTION_BEGIN_RANGE: VkLayerFunction = VkLayerFunction::VK_LAYER_FUNCTION_LINK;
    /// Last valid discriminant (= `VK_LAYER_FUNCTION_SET_DEVICE_LOADER_DATA`).
    pub const VK_LAYER_FUNCTION_END_RANGE: VkLayerFunction = VkLayerFunction::VK_LAYER_FUNCTION_SET_DEVICE_LOADER_DATA;
    /// Number of valid discriminants (`END_RANGE - BEGIN_RANGE + 1`).
    pub const VK_LAYER_FUNCTION_RANGE_SIZE: i32 = (VkLayerFunction::VK_LAYER_FUNCTION_SET_DEVICE_LOADER_DATA as i32)
        - (VkLayerFunction::VK_LAYER_FUNCTION_LINK as i32)
        + 1;
}

impl Default for VkLayerFunction {
    fn default() -> Self {
        VkLayerFunction::VK_LAYER_FUNCTION_LINK
    }
}

// ---------------------------------------------------------------------------
// vk_layer_dispatch_table.h
// ---------------------------------------------------------------------------

/// Opaque key for looking up per-device / per-instance dispatch tables.
pub type PFN_vkGetInstanceProcAddr = extern "system" fn(instance: u64, p_name: *const u8) -> extern "system" fn() -> u64;
pub type PFN_vkGetDeviceProcAddr = extern "system" fn(device: u64, p_name: *const u8) -> extern "system" fn() -> u64;

/// Per-instance dispatch table placeholder.  PCSX2 does not consume this
/// directly; we expose it for symbol-compatibility.
#[repr(C)]
pub struct VkLayerInstanceDispatchTable_ {
    _private: [u8; 0],
}
pub type VkLayerInstanceDispatchTable = VkLayerInstanceDispatchTable_;

/// Per-device dispatch table placeholder.
#[repr(C)]
pub struct VkLayerDispatchTable_ {
    _private: [u8; 0],
}
pub type VkLayerDispatchTable = VkLayerDispatchTable_;
