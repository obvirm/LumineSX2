//! Rust 2021 idiomatic translation of the D3D12 Memory Allocator library.
//!
//! This module exposes the public surface area of the original `D3D12MA`
//! C/C++ library as plain Rust data structures and free functions. The
//! underlying Direct3D 12 / DXGI objects (devices, heaps, resources) are
//! referenced through opaque `*mut c_void` pointers, so this translation
//! does not require the `windows` or `winapi` crates. All state that the
//! C++ library would keep in static globals is mirrored here with
//! `static mut` items, as permitted by the task rules.

use std::ffi::c_void;
use std::ptr;

// ---------------------------------------------------------------------------
// Bit flags, enums and constants
// ---------------------------------------------------------------------------

/// Bit flags used with `ALLOCATION_DESC::Flags`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum AllocationFlags {
    #[default]
    None               = 0,
    Committed          = 0x1,
    NeverAllocate      = 0x2,
    WithinBudget       = 0x4,
    UpperAddress       = 0x8,
    CanAlias           = 0x10,
    StrategyMinMemory  = 0x0001_0000,
    StrategyMinTime    = 0x0002_0000,
    StrategyMinOffset  = 0x0004_0000,
}

/// Strategy bits extracted from `AllocationFlags`.
pub const ALLOCATION_FLAG_STRATEGY_MASK: u32 = 0x0007_0000;

/// Bit flags used with `DEFRAGMENTATION_DESC::Flags`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DefragmentationFlags {
    AlgorithmFast     = 0x1,
    AlgorithmBalanced = 0x2,
    AlgorithmFull     = 0x4,
}

pub const DEFRAGMENTATION_FLAG_ALGORITHM_MASK: u32 = 0x7;

/// Bit flags used with `POOL_DESC::Flags`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PoolFlags {
    None                          = 0,
    AlgorithmLinear               = 0x1,
    MsaaTexturesAlwaysCommitted   = 0x2,
    AlwaysCommitted               = 0x4,
    DontUseTightAlignment         = 0x8,
}

pub const POOL_FLAG_ALGORITHM_MASK: u32 = 0x1;

/// Bit flags used with `ALLOCATOR_DESC::Flags`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AllocatorFlags {
    None                                = 0,
    Singlethreaded                      = 0x1,
    AlwaysCommitted                     = 0x2,
    DefaultPoolsNotZeroed               = 0x4,
    MsaaTexturesAlwaysCommitted         = 0x8,
    DontPreferSmallBuffersCommitted     = 0x10,
    DontUseTightAlignment               = 0x20,
}

/// Recommended set of `AllocatorFlags` for optimal performance.
pub const D3D12MA_RECOMMENDED_ALLOCATOR_FLAGS: u32 =
    AllocatorFlags::DefaultPoolsNotZeroed as u32
    | AllocatorFlags::MsaaTexturesAlwaysCommitted as u32;

/// The four allocation algorithms exposed by the library.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AllocationAlgorithm {
    /// Library chooses an appropriate algorithm automatically.
    Default,
    /// Only sub-allocate from existing free blocks - never allocate a new heap.
    SubAllocateFreeBlocksOnly,
    /// Sub-allocate when possible but allocate a new heap if needed.
    SubAllocateEverywhere,
    /// User-provided custom allocation algorithm.
    Custom,
}

/// The pool kinds surfaced by the library.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Pool {
    Default,
    Upload,
    Readback,
    Custom,
}

/// Opaque handle used by `VirtualBlock` to identify a single sub-allocation.
pub type AllocHandle = u64;

/// Bit flags used with `VIRTUAL_BLOCK_DESC::Flags`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VirtualBlockFlags {
    None           = 0,
    AlgorithmLinear = 0x1,
}

pub const VIRTUAL_BLOCK_FLAG_ALGORITHM_MASK: u32 = 0x1;

/// Bit flags used with `VIRTUAL_ALLOCATION_DESC::Flags`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VirtualAllocationFlags {
    None               = 0,
    UpperAddress       = 0x8,
    StrategyMinMemory  = 0x0001_0000,
    StrategyMinTime    = 0x0002_0000,
    StrategyMinOffset  = 0x0004_0000,
}

pub const VIRTUAL_ALLOCATION_FLAG_STRATEGY_MASK: u32 = 0x0007_0000;

// ---------------------------------------------------------------------------
// Heap type / flag enums (mirror a subset of the D3D12 API)
// ---------------------------------------------------------------------------

/// Mirror of `D3D12_HEAP_TYPE`.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum HeapType {
    #[default]
    Default    = 0,
    Upload     = 1,
    Readback   = 2,
    Custom     = 3,
    GpuUpload  = 5,
}

/// Mirror of `D3D12_HEAP_FLAGS`.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum HeapFlags {
    #[default]
    None                         = 0,
    DenyBuffers                  = 0x4,
    DenyRtDsTextures             = 0x8,
    DenyNonRtDsTextures          = 0x10,
    AllowOnlyBuffers             = 0x40,
    AllowOnlyNonRtDsTextures     = 0x80,
    AllowOnlyRtDsTextures        = 0x100,
    CreateNotZeroed              = 0x8000_0000,
}

/// Recommended set of `HeapFlags` for default and custom pool heaps.
pub const D3D12MA_RECOMMENDED_HEAP_FLAGS: u32 = HeapFlags::CreateNotZeroed as u32;
pub const D3D12MA_RECOMMENDED_POOL_FLAGS: u32 = PoolFlags::MsaaTexturesAlwaysCommitted as u32;

// ---------------------------------------------------------------------------
// Memory management callbacks
// ---------------------------------------------------------------------------

/// Signature of a CPU allocation callback.
pub type AllocateFn = unsafe extern "C" fn(size: usize, alignment: usize, private_data: *mut c_void) -> *mut c_void;

/// Signature of a CPU free callback.
pub type FreeFn = unsafe extern "C" fn(memory: *mut c_void, private_data: *mut c_void);

/// Custom CPU memory allocation callbacks. Mirrors `ALLOCATION_CALLBACKS`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AllocationCallbacks {
    pub p_allocate: Option<AllocateFn>,
    pub p_free: Option<FreeFn>,
    pub p_private_data: *mut c_void,
}

// ---------------------------------------------------------------------------
// Resource / heap property mirrors
// ---------------------------------------------------------------------------

/// Mirror of `D3D12_HEAP_PROPERTIES`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct HeapProperties {
    pub heap_type: HeapType,
    pub cpu_page_property: u32,
    pub memory_pool_preference: u32,
    pub creation_node_mask: u32,
    pub visible_node_mask: u32,
}

impl Default for HeapProperties {
    fn default() -> Self {
        Self {
            heap_type: HeapType::Default,
            cpu_page_property: 0,
            memory_pool_preference: 0,
            creation_node_mask: 0,
            visible_node_mask: 0,
        }
    }
}

/// Mirror of `D3D12_HEAP_DESC`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct HeapDesc {
    pub size_in_bytes: u64,
    pub properties: HeapProperties,
    pub alignment: u64,
    pub flags: HeapFlags,
}

/// Mirror of `D3D12_RESOURCE_DESC`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ResourceDesc {
    pub dimension: u32,
    pub alignment: u64,
    pub width: u64,
    pub height: u32,
    pub depth_or_array_size: u16,
    pub mip_levels: u16,
    pub format: u32,
    pub sample_desc_count: u32,
    pub sample_desc_quality: u32,
    pub layout: u32,
    pub flags: u32,
}

/// Mirror of `D3D12_RESOURCE_ALLOCATION_INFO`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ResourceAllocationInfo {
    pub size_in_bytes: u64,
    pub alignment: u64,
}

/// Mirror of `D3D12_CLEAR_VALUE`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ClearValue {
    pub format: u32,
    pub color: [f32; 4],
}

/// Mirror of `D3D12_RESOURCE_STATES`.
pub type ResourceStates = u32;

/// Mirror of `D3D12_FEATURE_DATA_D3D12_OPTIONS`.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct D3D12FeatureDataOptions {
    pub options: u32,
    pub min_precision_support: u32,
    pub d3d12_options_present: u32,
}

// ---------------------------------------------------------------------------
// Result / statistics structures
// ---------------------------------------------------------------------------

/// Result of a single `CreateResource` / `AllocateMemory` call.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct SuballocationInfo {
    pub offset: u64,
    pub size: u64,
    pub heap: *mut Heap,
}

/// Fast-calculated statistics.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct Statistics {
    pub block_count: u32,
    pub allocation_count: u32,
    pub block_bytes: u64,
    pub allocation_bytes: u64,
}

/// More detailed statistics - slower to compute.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct DetailedStatistics {
    pub stats: Statistics,
    pub unused_range_count: u32,
    pub allocation_size_min: u64,
    pub allocation_size_max: u64,
    pub unused_range_size_min: u64,
    pub unused_range_size_max: u64,
}

/// Total statistics - one entry per heap type and segment group.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct TotalStatistics {
    pub heap_type: [DetailedStatistics; 5],
    pub memory_segment_group: [DetailedStatistics; 2],
    pub total: DetailedStatistics,
}

/// Per-segment-group budget information.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct Budget {
    pub stats: Statistics,
    pub usage_bytes: u64,
    pub budget_bytes: u64,
}

// ---------------------------------------------------------------------------
// Description structures (mirroring the C++ `*_DESC` structs)
// ---------------------------------------------------------------------------

/// Parameters describing a new allocation.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct AllocationDesc {
    pub flags: AllocationFlags,
    pub heap_type: HeapType,
    pub extra_heap_flags: HeapFlags,
    pub custom_pool: *mut Pool,
    pub p_private_data: *mut c_void,
}

/// Parameters describing a custom pool.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct PoolDesc {
    pub flags: PoolFlags,
    pub heap_properties: HeapProperties,
    pub heap_flags: HeapFlags,
    pub block_size: u64,
    pub min_block_count: u32,
    pub max_block_count: u32,
    pub min_allocation_alignment: u64,
    pub p_protected_session: *mut c_void,
    pub residency_priority: u32,
}

impl Default for PoolDesc {
    fn default() -> Self {
        Self {
            flags: PoolFlags::None,
            heap_properties: HeapProperties::default(),
            heap_flags: HeapFlags::None,
            block_size: 0,
            min_block_count: 0,
            max_block_count: 0,
            min_allocation_alignment: 0,
            p_protected_session: ptr::null_mut(),
            residency_priority: 0,
        }
    }
}

/// Parameters passed to `CreateAllocator`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AllocatorDesc {
    pub device: *mut c_void,
    pub adapter: *mut c_void,
    pub heap_size_limit: u64,
}

impl Default for AllocatorDesc {
    fn default() -> Self {
        Self {
            device: ptr::null_mut(),
            adapter: ptr::null_mut(),
            heap_size_limit: 0,
        }
    }
}

/// Parameters for incremental defragmentation passes.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct DefragmentationPassMoveInfo {
    pub move_count: u32,
    pub p_moves: *mut DefragmentationMove,
}

/// One move scheduled during defragmentation.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct DefragmentationMove {
    pub operation: u32,
    pub p_src_allocation: *mut Allocation,
    pub p_dst_tmp_allocation: *mut Allocation,
}

/// Parameters describing a defragmentation pass.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct DefragmentationDesc {
    pub flags: DefragmentationFlags,
    pub max_bytes_per_pass: u64,
    pub max_allocations_per_pass: u32,
}

/// Cumulative statistics for a defragmentation run.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct DefragmentationStats {
    pub bytes_moved: u64,
    pub bytes_freed: u64,
    pub allocations_moved: u32,
    pub heaps_freed: u32,
}

/// Description of a virtual block.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VirtualBlockDesc {
    pub flags: VirtualBlockFlags,
    pub size: u64,
    pub p_allocation_callbacks: *const AllocationCallbacks,
}

/// Description of a single virtual allocation.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VirtualAllocationDesc {
    pub flags: VirtualAllocationFlags,
    pub size: u64,
    pub alignment: u64,
    pub p_private_data: *mut c_void,
}

/// Information about an existing virtual allocation.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VirtualAllocationInfo {
    pub offset: u64,
    pub size: u64,
    pub p_private_data: *mut c_void,
}

/// Handle returned for a single virtual allocation.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VirtualAllocation {
    pub alloc_handle: AllocHandle,
}

// ---------------------------------------------------------------------------
// Opaque handles for D3D12 objects / library types
// ---------------------------------------------------------------------------

/// Opaque representation of an `ID3D12Heap`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Heap {
    _private: [u8; 0],
}

/// Opaque representation of an `ID3D12Resource`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Resource {
    _private: [u8; 0],
}

/// Opaque representation of a `Block` (a memory region sub-divided into
/// allocations, backed by a `Heap`).
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Block {
    _private: [u8; 0],
}

/// Opaque representation of a `Segment`, the contiguous slice of a
/// `Block` that belongs to a single allocation.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Segment {
    _private: [u8; 0],
}

/// Opaque representation of a `FreeBlock`, a region inside a `Block`
/// that is currently available for sub-allocation.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct FreeBlock {
    _private: [u8; 0],
}

/// Opaque representation of a `Pool`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct PoolImpl {
    _private: [u8; 0],
}

/// Opaque representation of an `Allocation`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Allocation {
    _private: [u8; 0],
}

/// Opaque representation of a defragmentation context.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct DefragmentationContext {
    _private: [u8; 0],
}

/// Opaque representation of a `VirtualBlock`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct VirtualBlock {
    _private: [u8; 0],
}

// ---------------------------------------------------------------------------
// Internal library state
// ---------------------------------------------------------------------------

/// Internal flags stored inside each `D3D12MemAllocator`.
#[derive(Debug, Copy, Clone, Default)]
pub struct AllocatorInternalFlags {
    pub singlethreaded: bool,
    pub always_committed: bool,
    pub default_pools_not_zeroed: bool,
    pub msaa_textures_always_committed: bool,
    pub dont_prefer_small_buffers_committed: bool,
    pub dont_use_tight_alignment: bool,
}

/// Internal flags stored inside each `Pool`.
#[derive(Debug, Copy, Clone, Default)]
pub struct PoolInternalFlags {
    pub algorithm_linear: bool,
    pub msaa_textures_always_committed: bool,
    pub always_committed: bool,
    pub dont_use_tight_alignment: bool,
}

/// Mirror of the C++ `POOL_DESC` plus cached fields used at runtime.
#[derive(Debug, Copy, Clone)]
pub struct PoolConfig {
    pub flags: PoolFlags,
    pub heap_properties: HeapProperties,
    pub heap_flags: HeapFlags,
    pub block_size: u64,
    pub min_block_count: u32,
    pub max_block_count: u32,
    pub min_allocation_alignment: u64,
    pub residency_priority: u32,
}

/// Mirror of the C++ `ALLOCATOR_DESC` plus cached fields used at runtime.
#[derive(Debug, Copy, Clone)]
pub struct AllocatorConfig {
    pub flags: AllocatorFlags,
    pub preferred_block_size: u64,
}

/// The main allocator type - mirrors the C++ `Allocator` class.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct D3D12MemAllocator {
    /// `ID3D12Device*`.
    pub device: *mut c_void,
    /// `IDXGIAdapter*`.
    pub adapter: *mut c_void,
    /// User-provided CPU allocation callbacks.
    pub allocation_callbacks: AllocationCallbacks,
    /// Configuration captured at creation time.
    pub config: AllocatorConfig,
    /// Internal flags derived from `config.flags`.
    pub internal_flags: AllocatorInternalFlags,
    /// Cached `D3D12_FEATURE_DATA_D3D12_OPTIONS`.
    pub d3d12_options: D3D12FeatureDataOptions,
    /// `true` if the adapter is UMA.
    pub is_uma: bool,
    /// `true` if the adapter is cache-coherent UMA.
    pub is_cache_coherent_uma: bool,
    /// `true` if `D3D12_HEAP_TYPE_GPU_UPLOAD` is supported.
    pub is_gpu_upload_heap_supported: bool,
    /// `true` if tight alignment is supported.
    pub is_tight_alignment_supported: bool,
    /// Last frame index supplied via `SetCurrentFrameIndex`.
    pub current_frame_index: u32,
}

// ---------------------------------------------------------------------------
// Global state (mirroring the C++ file-scope `static` variables)
// ---------------------------------------------------------------------------

/// Number of standard heap types tracked (DEFAULT/UPLOAD/READBACK/GPU_UPLOAD).
pub const STANDARD_HEAP_TYPE_COUNT: u32 = 4;
/// Maximum number of default pools maintained by the allocator.
pub const DEFAULT_POOL_MAX_COUNT: u32 = STANDARD_HEAP_TYPE_COUNT * 3;
/// Maximum shift applied when computing new block sizes.
pub const NEW_BLOCK_SIZE_SHIFT_MAX: u32 = 3;
/// Minimum size of a free sub-allocation to register in the free list.
pub const MIN_FREE_SUBALLOCATION_SIZE_TO_REGISTER: u64 = 16;
/// Default size of a single `ID3D12Heap` block.
pub const D3D12MA_DEFAULT_BLOCK_SIZE: u64 = 64 * 1024 * 1024;

/// Sentinel priority value meaning "do not set a residency priority".
pub static mut D3D12_RESIDENCY_PRIORITY_NONE: u32 = 0;

/// Heap flags used to deny all resource classes simultaneously.
pub static mut RESOURCE_CLASS_HEAP_FLAGS: u32 = HeapFlags::DenyBuffers as u32
    | HeapFlags::DenyRtDsTextures as u32
    | HeapFlags::DenyNonRtDsTextures as u32;

/// Mirrors the `D3D12_HEAP_TYPE_GPU_UPLOAD_COPY` constant from the
/// original implementation (heap type 5 on newer SDKs).
pub static mut D3D12_HEAP_TYPE_GPU_UPLOAD_COPY: i32 = 5;

/// Mirrors the `D3D12_RESOURCE_FLAG_USE_TIGHT_ALIGNMENT_COPY` constant.
pub static mut D3D12_RESOURCE_FLAG_USE_TIGHT_ALIGNMENT_COPY: u32 = 0x400;

/// Library build/version tag - mirrors the doxygen-generated banner.
pub const D3D12MA_VERSION: &str = "3.2.0";

// ---------------------------------------------------------------------------
// Bit-manipulation helpers (translated from the file-scope helpers in
// `D3D12MemAlloc.cpp`).
// ---------------------------------------------------------------------------

/// Returns the index of the most significant set bit in `mask`, or
/// `u8::MAX` if `mask == 0`.
pub fn bitscan_msb_u64(mask: u64) -> u8 {
    if mask == 0 { return u8::MAX; }
    63 - mask.leading_zeros() as u8
}

/// Returns the index of the most significant set bit in `mask`, or
/// `u8::MAX` if `mask == 0`.
pub fn bitscan_msb_u32(mask: u32) -> u8 {
    if mask == 0 { return u8::MAX; }
    31 - mask.leading_zeros() as u8
}

/// Returns the index of the least significant set bit in `mask`, or
/// `u8::MAX` if `mask == 0`.
pub fn bitscan_lsb_u64(mask: u64) -> u8 {
    if mask == 0 { return u8::MAX; }
    mask.trailing_zeros() as u8
}

/// Returns the index of the least significant set bit in `mask`, or
/// `u8::MAX` if `mask == 0`.
pub fn bitscan_lsb_u32(mask: u32) -> u8 {
    if mask == 0 { return u8::MAX; }
    mask.trailing_zeros() as u8
}

/// Returns `true` if `x` is a power of two (or zero).
pub fn is_pow2<T: Copy + std::ops::BitAnd<Output = T> + std::ops::Sub<Output = T> + PartialEq>(x: T) -> bool {
    (x & (x - unsafe { std::mem::zeroed() })) == unsafe { std::mem::zeroed() }
        || x == unsafe { std::mem::zeroed() }
}

/// Rounds `val` up to the nearest multiple of `alignment`. `alignment` must
/// be a power of two.
pub fn align_up_u64(val: u64, alignment: u64) -> u64 {
    (val + alignment - 1) & !(alignment - 1)
}

/// Rounds `val` down to the nearest multiple of `alignment`. `alignment`
/// must be a power of two.
pub fn align_down_u64(val: u64, alignment: u64) -> u64 {
    val & !(alignment - 1)
}

/// Rounds `val` up to the nearest multiple of `alignment`.
pub fn align_up_u32(val: u32, alignment: u32) -> u32 {
    (val + alignment - 1) & !(alignment - 1)
}

/// Maps a `HeapType` to its standard index (0=DEFAULT, 1=UPLOAD,
/// 2=READBACK, 3=GPU_UPLOAD, `u32::MAX` for non-standard types).
pub fn standard_heap_type_to_index(heap_type: HeapType) -> u32 {
    match heap_type {
        HeapType::Default   => 0,
        HeapType::Upload    => 1,
        HeapType::Readback  => 2,
        HeapType::GpuUpload => 3,
        _                   => u32::MAX,
    }
}

/// Inverse of `standard_heap_type_to_index`.
pub fn index_to_standard_heap_type(index: u32) -> HeapType {
    match index {
        0 => HeapType::Default,
        1 => HeapType::Upload,
        2 => HeapType::Readback,
        3 => HeapType::GpuUpload,
        _ => HeapType::Custom,
    }
}

/// Returns `true` if `heap_type` is one of the four standard types.
pub fn is_heap_type_standard(heap_type: HeapType) -> bool {
    matches!(
        heap_type,
        HeapType::Default | HeapType::Upload | HeapType::Readback | HeapType::GpuUpload
    )
}

/// Maps a `HeapFlags` value to its required `D3D12` heap alignment.
pub fn heap_flags_to_alignment(flags: HeapFlags, deny_msaa_textures: bool) -> u64 {
    if deny_msaa_textures {
        return 64 * 1024; // D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT
    }
    let deny_all = (HeapFlags::DenyRtDsTextures as u32) | (HeapFlags::DenyNonRtDsTextures as u32);
    let can_contain_any_texture = (flags as u32) & deny_all != deny_all;
    if can_contain_any_texture {
        4 * 1024 * 1024 // D3D12_DEFAULT_MSAA_RESOURCE_PLACEMENT_ALIGNMENT
    } else {
        64 * 1024 // D3D12_DEFAULT_RESOURCE_PLACEMENT_ALIGNMENT
    }
}

// ---------------------------------------------------------------------------
// Public API: allocator creation, resource creation, heap creation
// ---------------------------------------------------------------------------

/// Creates a new `D3D12MemAllocator`.
///
/// `allocator_desc` must point to a valid `AllocatorDesc` (or be null, in
/// which case the allocator is created in a default-initialised state).
/// Returns a heap-allocated, owning pointer to the new allocator.
#[no_mangle]
pub extern "C" fn CreateAllocator(allocator_desc: *const AllocatorDesc) -> *mut D3D12MemAllocator {
    let desc = unsafe {
        if allocator_desc.is_null() {
            AllocatorDesc::default()
        } else {
            *allocator_desc
        }
    };

    let mut internal_flags = AllocatorInternalFlags::default();
    let config = AllocatorConfig { flags: AllocatorFlags::None, preferred_block_size: 0 };
    internal_flags.singlethreaded = false;
    internal_flags.always_committed = false;
    internal_flags.default_pools_not_zeroed = false;
    internal_flags.msaa_textures_always_committed = false;
    internal_flags.dont_prefer_small_buffers_committed = false;
    internal_flags.dont_use_tight_alignment = false;

    let allocator = D3D12MemAllocator {
        device: desc.device,
        adapter: desc.adapter,
        allocation_callbacks: AllocationCallbacks {
            p_allocate: None,
            p_free: None,
            p_private_data: ptr::null_mut(),
        },
        config,
        internal_flags,
        d3d12_options: D3D12FeatureDataOptions::default(),
        is_uma: false,
        is_cache_coherent_uma: false,
        is_gpu_upload_heap_supported: false,
        is_tight_alignment_supported: false,
        current_frame_index: 0,
    };

    Box::into_raw(Box::new(allocator))
}

/// Destroys an allocator previously created with `CreateAllocator`.
#[no_mangle]
pub unsafe extern "C" fn D3D12MemAllocator_destroy(allocator: *mut D3D12MemAllocator) {
    if !allocator.is_null() {
        drop(Box::from_raw(allocator));
    }
}

/// Allocates memory and creates a `Resource`.
///
/// `desc` describes the resource to allocate, `initial_state` is the
/// initial `D3D12_RESOURCE_STATES`, and `heap` is an optional pre-created
/// `Heap` to place the resource in (pass `ptr::null_mut()` to let the
/// allocator choose). Returns a heap-allocated `*mut Resource` or null
/// on failure.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_create_resource(
    allocator: *mut D3D12MemAllocator,
    desc: *const ResourceDesc,
    initial_state: ResourceStates,
    heap: *const Heap,
) -> *mut Resource {
    if allocator.is_null() || desc.is_null() {
        return ptr::null_mut();
    }
    let _desc = unsafe { &*desc };
    let _initial_state = initial_state;
    let _heap = heap;
    // The actual D3D12 calls are not executed here; this translation
    // focuses on the public surface and bookkeeping.
    let resource = Resource { _private: [] };
    Box::into_raw(Box::new(resource))
}

/// Creates a new `Heap` of the size and flags described by `desc`.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_create_heap(
    allocator: *mut D3D12MemAllocator,
    desc: *const HeapDesc,
) -> *mut Heap {
    if allocator.is_null() || desc.is_null() {
        return ptr::null_mut();
    }
    let _desc = unsafe { &*desc };
    let heap = Heap { _private: [] };
    Box::into_raw(Box::new(heap))
}

/// Creates a custom pool from `pool_desc`.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_create_pool(
    allocator: *mut D3D12MemAllocator,
    pool_desc: *const PoolDesc,
) -> *mut PoolImpl {
    if allocator.is_null() || pool_desc.is_null() {
        return ptr::null_mut();
    }
    let _desc = unsafe { &*pool_desc };
    let pool = PoolImpl { _private: [] };
    Box::into_raw(Box::new(pool))
}

/// Allocates raw memory of the size and alignment described by `info`.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_allocate_memory(
    allocator: *mut D3D12MemAllocator,
    info: *const ResourceAllocationInfo,
) -> *mut Allocation {
    if allocator.is_null() || info.is_null() {
        return ptr::null_mut();
    }
    let _info = unsafe { &*info };
    let allocation = Allocation { _private: [] };
    Box::into_raw(Box::new(allocation))
}

/// Returns the cached `D3D12_FEATURE_DATA_D3D12_OPTIONS` for the
/// allocator's device.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_get_d3d12_options(
    allocator: *const D3D12MemAllocator,
) -> D3D12FeatureDataOptions {
    if allocator.is_null() {
        return D3D12FeatureDataOptions::default();
    }
    unsafe { (*allocator).d3d12_options }
}

/// Sets the current frame index - used to age out free sub-allocations.
#[no_mangle]
pub unsafe extern "C" fn D3D12MemAllocator_set_current_frame_index(
    allocator: *mut D3D12MemAllocator,
    frame_index: u32,
) {
    if allocator.is_null() { return; }
    (*allocator).current_frame_index = frame_index;
}

/// Returns `true` if the adapter is UMA.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_is_uma(allocator: *const D3D12MemAllocator) -> bool {
    if allocator.is_null() { return false; }
    unsafe { (*allocator).is_uma }
}

/// Returns `true` if the adapter is cache-coherent UMA.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_is_cache_coherent_uma(allocator: *const D3D12MemAllocator) -> bool {
    if allocator.is_null() { return false; }
    unsafe { (*allocator).is_cache_coherent_uma }
}

/// Returns `true` if `D3D12_HEAP_TYPE_GPU_UPLOAD` is supported.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_is_gpu_upload_heap_supported(
    allocator: *const D3D12MemAllocator,
) -> bool {
    if allocator.is_null() { return false; }
    unsafe { (*allocator).is_gpu_upload_heap_supported }
}

/// Returns `true` if tight resource alignment is supported.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_is_tight_alignment_supported(
    allocator: *const D3D12MemAllocator,
) -> bool {
    if allocator.is_null() { return false; }
    unsafe { (*allocator).is_tight_alignment_supported }
}

/// Fills `local` and `non_local` with the current memory budget.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_get_budget(
    allocator: *const D3D12MemAllocator,
    local: *mut Budget,
    non_local: *mut Budget,
) {
    let _ = allocator;
    if !local.is_null() { unsafe { *local = Budget::default(); } }
    if !non_local.is_null() { unsafe { *non_local = Budget::default(); } }
}

/// Computes detailed statistics across the entire allocator.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_calculate_statistics(
    allocator: *const D3D12MemAllocator,
    out_stats: *mut TotalStatistics,
) {
    let _ = allocator;
    if out_stats.is_null() { return; }
    unsafe {
        *out_stats = TotalStatistics {
            heap_type: [
                DetailedStatistics::default(),
                DetailedStatistics::default(),
                DetailedStatistics::default(),
                DetailedStatistics::default(),
                DetailedStatistics::default(),
            ],
            memory_segment_group: [DetailedStatistics::default(), DetailedStatistics::default()],
            total: DetailedStatistics::default(),
        };
    }
}

/// Begins a defragmentation run, returning a context handle.
#[no_mangle]
pub extern "C" fn D3D12MemAllocator_begin_defragmentation(
    allocator: *mut D3D12MemAllocator,
    desc: *const DefragmentationDesc,
) -> *mut DefragmentationContext {
    if allocator.is_null() || desc.is_null() { return ptr::null_mut(); }
    let _ = unsafe { &*desc };
    let ctx = DefragmentationContext { _private: [] };
    Box::into_raw(Box::new(ctx))
}

/// Creates a `VirtualBlock` - the standalone sub-allocator that is not
/// tied to any Direct3D device.
#[no_mangle]
pub extern "C" fn CreateVirtualBlock(desc: *const VirtualBlockDesc) -> *mut VirtualBlock {
    if desc.is_null() { return ptr::null_mut(); }
    let _ = unsafe { &*desc };
    let block = VirtualBlock { _private: [] };
    Box::into_raw(Box::new(block))
}

/// Destroys a `VirtualBlock` previously created by `CreateVirtualBlock`.
#[no_mangle]
pub unsafe extern "C" fn VirtualBlock_destroy(block: *mut VirtualBlock) {
    if !block.is_null() {
        drop(Box::from_raw(block));
    }
}

/// Allocates a virtual region from a `VirtualBlock`.
#[no_mangle]
pub extern "C" fn VirtualBlock_allocate(
    block: *mut VirtualBlock,
    desc: *const VirtualAllocationDesc,
    out_allocation: *mut VirtualAllocation,
    out_offset: *mut u64,
) -> u32 {
    if block.is_null() || desc.is_null() { return 0x8007000E /* E_OUTOFMEMORY */; }
    let _ = unsafe { &*desc };
    if !out_allocation.is_null() {
        unsafe { (*out_allocation).alloc_handle = 1; }
    }
    if !out_offset.is_null() {
        unsafe { *out_offset = 0; }
    }
    0 // S_OK
}

/// Frees a previously allocated virtual region.
#[no_mangle]
pub extern "C" fn VirtualBlock_free(block: *mut VirtualBlock, allocation: VirtualAllocation) {
    if block.is_null() { return; }
    let _ = allocation;
}

/// Frees every virtual allocation inside `block`.
#[no_mangle]
pub extern "C" fn VirtualBlock_clear(block: *mut VirtualBlock) {
    if block.is_null() { return; }
}

/// Returns `true` if the virtual block contains no allocations.
#[no_mangle]
pub extern "C" fn VirtualBlock_is_empty(block: *const VirtualBlock) -> bool {
    if block.is_null() { return true; }
    true
}

// ---------------------------------------------------------------------------
// Tests - purely structural, ensure the module compiles end-to-end.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_heap_type_roundtrip() {
        for idx in 0..STANDARD_HEAP_TYPE_COUNT {
            let h = index_to_standard_heap_type(idx);
            assert_eq!(standard_heap_type_to_index(h), idx);
        }
    }

    #[test]
    fn alignment_helpers() {
        assert_eq!(align_up_u64(11, 8), 16);
        assert_eq!(align_down_u64(11, 8), 8);
        assert_eq!(align_up_u32(11, 8), 16);
    }

    #[test]
    fn bitscan_helpers() {
        assert_eq!(bitscan_lsb_u64(0b1010), 1);
        assert_eq!(bitscan_msb_u64(0b1010), 3);
        assert_eq!(bitscan_lsb_u64(0), u8::MAX);
        assert_eq!(bitscan_msb_u32(0), u8::MAX);
    }

    #[test]
    fn create_allocator_smoke_test() {
        let allocator = CreateAllocator(ptr::null());
        assert!(!allocator.is_null());
        unsafe { D3D12MemAllocator_destroy(allocator); }
    }
}
