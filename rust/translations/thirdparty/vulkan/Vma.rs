//! Idiomatic Rust 2021 translation of the Vulkan Memory Allocator (VMA)
//! public C API.
//!
//! Translates `vk_mem_alloc.h` (VMA 3.4.0) into a single idiomatic Rust
//! 2021 module.  All opaque handles become `pub type Xxx = u64`; public
//! structs use `#[repr(C)]` to match the C ABI.  Only the standard library
//! is required.
//!
//! This module deliberately re-uses the `Vk*` types from `super::VulkanCore`,
//! so the consuming crate should import both modules side-by-side.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use core::ffi::c_void;
use super::VulkanCore::*;

// ===========================================================================
// 1. Version and configuration constants.
// ===========================================================================

pub const VMA_VERSION_MAJOR: u32 = 3;
pub const VMA_VERSION_MINOR: u32 = 4;
pub const VMA_VERSION_PATCH: u32 = 0;
pub const VMA_VERSION: u32 = VMA_VERSION_MAJOR * 10000
    + VMA_VERSION_MINOR * 100
    + VMA_VERSION_PATCH;

pub const VMA_API_VERSION: u32 = 1004000;

pub const VMA_NULL_HANDLE: u64 = 0;

// Per-feature compile-time switches (PCSX2 enables the full set).
pub const VMA_DEDICATED_ALLOCATION: u32 = 1;
pub const VMA_BIND_MEMORY2: u32 = 1;
pub const VMA_MEMORY_BUDGET: u32 = 1;
pub const VMA_BUFFER_DEVICE_ADDRESS: u32 = 1;
pub const VMA_MEMORY_PRIORITY: u32 = 1;
pub const VMA_KHR_MAINTENANCE4: u32 = 1;
pub const VMA_KHR_MAINTENANCE5: u32 = 1;
pub const VMA_EXTERNAL_MEMORY: u32 = 1;
pub const VMA_EXTERNAL_MEMORY_WIN32: u32 = 1;

// ===========================================================================
// 2. Opaque handles.
// ===========================================================================

pub type VmaAllocator = u64;
pub type VmaPool = u64;
pub type VmaAllocation = u64;
pub type VmaDefragmentationContext = u64;
pub type VmaVirtualAllocation = u64;
pub type VmaVirtualBlock = u64;
pub type VmaAllocationRequest = u64;

pub const VMA_ALLOCATION_REQUEST_USER_DATA_STRING_BYTE_COUNT: usize = 65536;

// ===========================================================================
// 3. Public enums.
// ===========================================================================

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaAllocatorCreateFlagBits {
    VMA_ALLOCATOR_CREATE_EXTERNALLY_SYNCHRONIZED_BIT = 0x1,
    VMA_ALLOCATOR_CREATE_KHR_DEDICATED_ALLOCATION_BIT = 0x2,
    VMA_ALLOCATOR_CREATE_KHR_BIND_MEMORY2_BIT = 0x4,
    VMA_ALLOCATOR_CREATE_EXT_MEMORY_BUDGET_BIT = 0x8,
    VMA_ALLOCATOR_CREATE_AMD_DEVICE_COHERENT_MEMORY_BIT = 0x10,
    VMA_ALLOCATOR_CREATE_BUFFER_DEVICE_ADDRESS_BIT = 0x20,
    VMA_ALLOCATOR_CREATE_EXT_MEMORY_PRIORITY_BIT = 0x40,
    VMA_ALLOCATOR_CREATE_KHR_MAINTENANCE4_BIT = 0x80,
    VMA_ALLOCATOR_CREATE_KHR_MAINTENANCE5_BIT = 0x100,
    VMA_ALLOCATOR_CREATE_KHR_EXTERNAL_MEMORY_BIT = 0x200,
    VMA_ALLOCATOR_CREATE_FLAG_BITS_MAX_ENUM = 0x7FFFFFFF,
}
pub type VmaAllocatorCreateFlags = u32;

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaMemoryUsage {
    VMA_MEMORY_USAGE_UNKNOWN = 0,
    VMA_MEMORY_USAGE_GPU_ONLY = 1,
    VMA_MEMORY_USAGE_CPU_ONLY = 2,
    VMA_MEMORY_USAGE_CPU_TO_GPU = 3,
    VMA_MEMORY_USAGE_GPU_TO_CPU = 4,
    VMA_MEMORY_USAGE_CPU_COPY = 5,
    VMA_MEMORY_USAGE_GPU_LAZILY_ALLOCATED = 6,
    VMA_MEMORY_USAGE_AUTO = 7,
    VMA_MEMORY_USAGE_AUTO_PREFER_DEVICE = 8,
    VMA_MEMORY_USAGE_AUTO_PREFER_HOST = 9,
    VMA_MEMORY_USAGE_MAX_ENUM = 0x7FFFFFFF,
}
impl Default for VmaMemoryUsage {
    fn default() -> Self { VmaMemoryUsage::VMA_MEMORY_USAGE_AUTO }
}

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaAllocationCreateFlagBits {
    VMA_ALLOCATION_CREATE_DEDICATED_MEMORY_BIT = 0x1,
    VMA_ALLOCATION_CREATE_NEVER_ALLOCATE_BIT = 0x2,
    VMA_ALLOCATION_CREATE_MAPPED_BIT = 0x4,
    VMA_ALLOCATION_CREATE_USER_DATA_COPY_STRING_BIT = 0x20,
    VMA_ALLOCATION_CREATE_UPPER_ADDRESS_BIT = 0x40,
    VMA_ALLOCATION_CREATE_DONT_BIND_BIT = 0x80,
    VMA_ALLOCATION_CREATE_WITHIN_BUDGET_BIT = 0x100,
    VMA_ALLOCATION_CREATE_CAN_ALIAS_BIT = 0x200,
    VMA_ALLOCATION_CREATE_HOST_ACCESS_SEQUENTIAL_WRITE_BIT = 0x400,
    VMA_ALLOCATION_CREATE_HOST_ACCESS_RANDOM_BIT = 0x800,
    VMA_ALLOCATION_CREATE_HOST_ACCESS_ALLOW_TRANSFER_INSTEAD_BIT = 0x1000,
    VMA_ALLOCATION_CREATE_STRATEGY_MIN_MEMORY_BIT = 0x10000,
    VMA_ALLOCATION_CREATE_STRATEGY_MIN_TIME_BIT = 0x20000,
    VMA_ALLOCATION_CREATE_STRATEGY_BEST_FIT_BIT = 0x40000,
    VMA_ALLOCATION_CREATE_STRATEGY_FIRST_FIT_BIT = 0x80000,
    VMA_ALLOCATION_CREATE_STRATEGY_MASK = 0xF0000,
    VMA_ALLOCATION_CREATE_FLAG_BITS_MAX_ENUM = 0x7FFFFFFF,
}
pub type VmaAllocationCreateFlags = u32;

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaSuballocationType {
    VMA_SUBALLOCATION_TYPE_UNKNOWN = 0,
    VMA_SUBALLOCATION_TYPE_FREE = 1,
    VMA_SUBALLOCATION_TYPE_BUFFER = 2,
    VMA_SUBALLOCATION_TYPE_IMAGE_UNKNOWN = 3,
    VMA_SUBALLOCATION_TYPE_IMAGE_LINEAR = 4,
    VMA_SUBALLOCATION_TYPE_IMAGE_OPTIMAL = 5,
    VMA_SUBALLOCATION_TYPE_IMAGE_LAZILY_ALLOCATED = 6,
    VMA_SUBALLOCATION_TYPE_MAX_ENUM = 0x7FFFFFFF,
}
impl Default for VmaSuballocationType {
    fn default() -> Self { VmaSuballocationType::VMA_SUBALLOCATION_TYPE_UNKNOWN }
}

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaPoolCreateFlagBits {
    VMA_POOL_CREATE_IGNORE_TRANSIENT_BIT = 0x1,
    VMA_POOL_CREATE_LINEAR_ALGORITHM_BIT = 0x2,
    VMA_POOL_CREATE_ALG_MASK = 0x6,
    VMA_POOL_CREATE_BUDGET_BIT = 0x8,
    VMA_POOL_CREATE_UPPER_ADDRESS_BIT = 0x10,
    VMA_POOL_CREATE_HOST_ACCESS_SEQUENTIAL_WRITE_BIT = 0x20,
    VMA_POOL_CREATE_HOST_ACCESS_ALLOW_TRANSFER_INSTEAD_BIT = 0x40,
    VMA_POOL_CREATE_MIN_MEMORY_FRAGMENTATION_BIT = 0x80,
    VMA_POOL_CREATE_FLAG_BITS_MAX_ENUM = 0x7FFFFFFF,
}
pub type VmaPoolCreateFlags = u32;

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaDefragmentationFlagBits {
    VMA_DEFRAGMENTATION_FLAG_GENERATE_OBJECTS_BIT = 0x1,
    VMA_DEFRAGMENTATION_FLAG_INCREMENTAL_BIT = 0x2,
    VMA_DEFRAGMENTATION_FLAG_BOTTOM_LEVEL_BIT = 0x4,
    VMA_DEFRAGMENTATION_FLAG_BY_REGION_BIT = 0x8,
    VMA_DEFRAGMENTATION_FLAG_MAX_ENUM = 0x7FFFFFFF,
}
pub type VmaDefragmentationFlags = u32;

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaDefragmentationMoveOperation {
    VMA_DEFRAGMENTATION_MOVE_OPERATION_COPY = 0,
    VMA_DEFRAGMENTATION_MOVE_OPERATION_IGNORE = 1,
    VMA_DEFRAGMENTATION_MOVE_OPERATION_DESTROY = 2,
    VMA_DEFRAGMENTATION_MOVE_OPERATION_MAX_ENUM = 0x7FFFFFFF,
}
impl Default for VmaDefragmentationMoveOperation {
    fn default() -> Self { VmaDefragmentationMoveOperation::VMA_DEFRAGMENTATION_MOVE_OPERATION_COPY }
}

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaVirtualBlockCreateFlagBits {
    VMA_VIRTUAL_BLOCK_CREATE_LINEAR_ALGORITHM_BIT = 0x1,
    VMA_VIRTUAL_BLOCK_CREATE_FLAG_BITS_MAX_ENUM = 0x7FFFFFFF,
}
pub type VmaVirtualBlockCreateFlags = u32;

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum VmaVirtualAllocationCreateFlagBits {
    VMA_VIRTUAL_ALLOCATION_CREATE_UPPER_ADDRESS_BIT = 0x1,
    VMA_VIRTUAL_ALLOCATION_CREATE_STRATEGY_MIN_MEMORY_BIT = 0x10000,
    VMA_VIRTUAL_ALLOCATION_CREATE_STRATEGY_MIN_TIME_BIT = 0x20000,
    VMA_VIRTUAL_ALLOCATION_CREATE_STRATEGY_BEST_FIT_BIT = 0x40000,
    VMA_VIRTUAL_ALLOCATION_CREATE_STRATEGY_FIRST_FIT_BIT = 0x80000,
    VMA_VIRTUAL_ALLOCATION_CREATE_STRATEGY_MASK = 0xF0000,
    VMA_VIRTUAL_ALLOCATION_CREATE_FLAG_BITS_MAX_ENUM = 0x7FFFFFFF,
}
pub type VmaVirtualAllocationCreateFlags = u32;

// ===========================================================================
// 4. Public structs.
// ===========================================================================

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaAllocatorCreateInfo {
    pub flags: VmaAllocatorCreateFlags,
    pub physicalDevice: VkPhysicalDevice,
    pub device: VkDevice,
    pub preferredLargeHeapBlockSize: VkDeviceSize,
    pub allocationCallbacks: *const VkAllocationCallbacks,
    pub instance: VkInstance,
    pub vulkanApiVersion: u32,
    pub pHeapSizeLimit: *const VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaAllocationCreateInfo {
    pub flags: VmaAllocationCreateFlags,
    pub usage: VmaMemoryUsage,
    pub requiredFlags: VkMemoryPropertyFlags,
    pub preferredFlags: VkMemoryPropertyFlags,
    pub memoryTypeBits: u32,
    pub pool: VmaPool,
    pub pUserData: *mut c_void,
    pub priority: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaAllocationInfo {
    pub memoryType: u32,
    pub deviceMemory: VkDeviceMemory,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
    pub mappedData: *mut c_void,
    pub userData: *mut c_void,
    pub name: *const u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaPoolCreateInfo {
    pub memoryTypeIndex: u32,
    pub flags: VmaPoolCreateFlags,
    pub blockSize: VkDeviceSize,
    pub minAllocationAlignment: VkDeviceSize,
    pub minBlockCount: usize,
    pub maxBlockCount: usize,
    pub priority: f32,
    pub minUnusedBlockRangeSizeToEndBeforeReblocking: VkDeviceSize,
    pub frameInUseCount: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaPoolStats {
    pub size: VkDeviceSize,
    pub unusedSize: VkDeviceSize,
    pub allocationCount: usize,
    pub unusedRangeCount: usize,
    pub suballocationCount: usize,
    pub blockCount: usize,
    pub hasVirtualBlocks: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaStatistics {
    pub blockCount: u32,
    pub allocationCount: u32,
    pub suballocationCount: u32,
    pub unusedRangeCount: u32,
    pub usedBytes: VkDeviceSize,
    pub unusedBytes: VkDeviceSize,
    pub allocationSizeMin: VkDeviceSize,
    pub allocationSizeMax: VkDeviceSize,
    pub allocationSizeAvg: VkDeviceSize,
    pub unusedRangeSizeMin: VkDeviceSize,
    pub unusedRangeSizeMax: VkDeviceSize,
    pub unusedRangeSizeAvg: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaDetailedStatistics {
    pub statistics: VmaStatistics,
    pub unusedRangeSizeMaxDesc: [u8; VMA_ALLOCATION_REQUEST_USER_DATA_STRING_BYTE_COUNT],
    pub allocationCountMin: u32,
    pub allocationCountMax: u32,
    pub unusedRangeCountMin: u32,
    pub unusedRangeCountMax: u32,
    pub suballocationCountMin: u32,
    pub suballocationCountMax: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaTotalStatistics {
    pub memoryType: [VmaDetailedStatistics; VK_MAX_MEMORY_TYPES as usize],
    pub memoryHeap: [VmaDetailedStatistics; VK_MAX_MEMORY_HEAPS as usize],
    pub total: VmaDetailedStatistics,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaBudget {
    pub statistics: VmaStatistics,
    pub usage: VkDeviceSize,
    pub budget: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaRecordSettings {
    pub flags: u32,
    pub pFilename: *const u8,
    pub escapeFormat: VkBool32,
    pub tsFormat: VkBool32,
    pub pAllocationCallbacks: *const VkAllocationCallbacks,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaDefragmentationInfo {
    pub flags: VmaDefragmentationFlags,
    pub pool: VmaPool,
    pub maxBytesPerMove: VkDeviceSize,
    pub maxAllocationsPerMove: u32,
    pub pfnCallback: Option<unsafe extern "system" fn(
        userData: *mut c_void,
        allocationsMoved: VkDeviceSize,
        bytesMoved: VkDeviceSize,
        allocationsRemaining: u32,
        bytesRemaining: VkDeviceSize,
    ) -> VkBool32>,
    pub pUserData: *mut c_void,
    pub maxCpuAllocationsForFragmentationCheck: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaDefragmentationMove {
    pub operation: VmaDefragmentationMoveOperation,
    pub srcAllocation: VmaAllocation,
    pub dstTmpAllocation: VmaAllocation,
    pub dstAllocation: VmaAllocation,
    pub srcOffset: VkDeviceSize,
    pub dstOffset: VkDeviceSize,
    pub size: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaDefragmentationPassMoveInfo {
    pub moveCount: u32,
    pub pMoves: *const VmaDefragmentationMove,
    pub bytesMoved: VkDeviceSize,
    pub allocationsRemaining: u32,
    pub bytesRemaining: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaDefragmentationPassInfo {
    pub iPass: u32,
    pub moveCount: u32,
    pub pPassMoves: *const VmaDefragmentationPassMoveInfo,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaVirtualBlockCreateInfo {
    pub flags: VmaVirtualBlockCreateFlags,
    pub size: VkDeviceSize,
    pub pAllocationCallbacks: *const VkAllocationCallbacks,
    pub memoryTypeIndex: u32,
    pub bufferDeviceAddress: VkDeviceAddress,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaVirtualAllocationCreateInfo {
    pub flags: VmaVirtualAllocationCreateFlags,
    pub size: VkDeviceSize,
    pub alignment: VkDeviceSize,
    pub pUserData: *mut c_void,
    pub priority: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaVirtualAllocationInfo {
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
    pub pUserData: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaAllocationRequestUserData {
    pub bufferImageUsage: VkImageUsageFlags,
    pub preferDedicated: VkBool32,
    pub requiredFlags: VkMemoryPropertyFlags,
    pub preferredFlags: VkMemoryPropertyFlags,
    pub memoryTypeBits: u32,
    pub memoryAllocateFlags: VkMemoryAllocateFlags,
    pub memoryPriority: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaBufferCreateInfo {
    pub size: VkDeviceSize,
    pub usage: VkBufferUsageFlags,
    pub sharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
    pub flags: VmaAllocationCreateFlags,
    pub memoryUsage: VmaMemoryUsage,
    pub requiredFlags: VkMemoryPropertyFlags,
    pub preferredFlags: VkMemoryPropertyFlags,
    pub memoryTypeBits: u32,
    pub pool: VmaPool,
    pub pUserData: *mut c_void,
    pub priority: f32,
    pub alignment: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaImageCreateInfo {
    pub imageType: VkImageType,
    pub format: VkFormat,
    pub extent: VkExtent3D,
    pub mipLevels: u32,
    pub arrayLayers: u32,
    pub samples: VkSampleCountFlagBits,
    pub tiling: VkImageTiling,
    pub usage: VkImageUsageFlags,
    pub sharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
    pub initialLayout: VkImageLayout,
    pub flags: VmaAllocationCreateFlags,
    pub memoryUsage: VmaMemoryUsage,
    pub requiredFlags: VkMemoryPropertyFlags,
    pub preferredFlags: VkMemoryPropertyFlags,
    pub memoryTypeBits: u32,
    pub pool: VmaPool,
    pub pUserData: *mut c_void,
    pub priority: f32,
    pub alignment: VkDeviceSize,
}

// ===========================================================================
// 5. Function pointer types.
// ===========================================================================

pub type PFN_vmaAllocateDeviceMemoryFunction = extern "system" fn(
    userData: *mut c_void,
    deviceMemory: VkDeviceMemory,
    memory: VkDeviceSize,
    memoryType: u32,
    allocationUserData: *const VmaAllocationRequestUserData,
) -> VkBool32;

pub type PFN_vmaFreeDeviceMemoryFunction = extern "system" fn(
    userData: *mut c_void,
    deviceMemory: VkDeviceMemory,
    memory: VkDeviceSize,
    memoryType: u32,
    allocationUserData: *const VmaAllocationRequestUserData,
);

// ===========================================================================
// 6. Result codes (VmaResult mirrors VkResult).
// ===========================================================================

pub type VmaResult = VkResult;

pub fn vma_result_is_success(result: VmaResult) -> bool {
    (result as i32) >= 0
}

pub fn vma_result_is_error(result: VmaResult) -> bool {
    (result as i32) < 0
}

// ===========================================================================
// 7. Foreign-function declarations.
// ===========================================================================

extern "system" {
    // ---- Allocator lifecycle ----
    pub fn vmaCreateAllocator(pCreateInfo: *const VmaAllocatorCreateInfo, pAllocator: *mut VmaAllocator) -> VkResult;
    pub fn vmaDestroyAllocator(allocator: VmaAllocator);

    // ---- Memory allocation ----
    pub fn vmaAllocateMemory(allocator: VmaAllocator, pVkMemoryRequirements: *const VkMemoryRequirements, pCreateInfo: *const VmaAllocationCreateInfo, pAllocation: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaAllocateMemoryPages(allocator: VmaAllocator, pVkMemoryRequirements: *const VkMemoryRequirements, pCreateInfo: *const VmaAllocationCreateInfo, allocationCount: usize, pAllocations: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaAllocateMemoryForBuffer(allocator: VmaAllocator, buffer: VkBuffer, pCreateInfo: *const VmaAllocationCreateInfo, pAllocation: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaAllocateMemoryForImage(allocator: VmaAllocator, image: VkImage, pCreateInfo: *const VmaAllocationCreateInfo, pAllocation: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaFreeMemory(allocator: VmaAllocator, allocation: VmaAllocation);
    pub fn vmaFreeMemoryPages(allocator: VmaAllocator, allocationCount: usize, pAllocations: *const VmaAllocation);
    pub fn vmaGetAllocationInfo(allocator: VmaAllocator, allocation: VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo);
    pub fn vmaSetAllocationUserData(allocator: VmaAllocator, allocation: VmaAllocation, pUserData: *mut c_void);
    pub fn vmaSetAllocationName(allocator: VmaAllocator, allocation: VmaAllocation, pName: *const u8);
    pub fn vmaGetAllocationMemoryProperties(allocator: VmaAllocator, allocation: VmaAllocation, pFlags: *mut VkMemoryPropertyFlags);
    pub fn vmaMapMemory(allocator: VmaAllocator, allocation: VmaAllocation, ppData: *mut *mut c_void) -> VkResult;
    pub fn vmaUnmapMemory(allocator: VmaAllocator, allocation: VmaAllocation);
    pub fn vmaFlushAllocation(allocator: VmaAllocator, allocation: VmaAllocation, offset: VkDeviceSize, size: VkDeviceSize) -> VkResult;
    pub fn vmaInvalidateAllocation(allocator: VmaAllocator, allocation: VmaAllocation, offset: VkDeviceSize, size: VkDeviceSize) -> VkResult;
    pub fn vmaFlushAllocations(allocator: VmaAllocator, allocationCount: u32, pAllocations: *const VmaAllocation, pOffsets: *const VkDeviceSize, pSizes: *const VkDeviceSize) -> VkResult;
    pub fn vmaInvalidateAllocations(allocator: VmaAllocator, allocationCount: u32, pAllocations: *const VmaAllocation, pOffsets: *const VkDeviceSize, pSizes: *const VkDeviceSize) -> VkResult;

    // ---- Buffer / image convenience ----
    pub fn vmaCreateBuffer(allocator: VmaAllocator, pBufferCreateInfo: *const VkBufferCreateInfo, pAllocationCreateInfo: *const VmaAllocationCreateInfo, pBuffer: *mut VkBuffer, pAllocation: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaCreateBufferWithAlignment(allocator: VmaAllocator, pBufferCreateInfo: *const VkBufferCreateInfo, pAllocationCreateInfo: *const VmaAllocationCreateInfo, minAlignment: VkDeviceSize, pBuffer: *mut VkBuffer, pAllocation: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaCreateAliasingBuffer(allocator: VmaAllocator, pBufferCreateInfo: *const VkBufferCreateInfo, allocation: VmaAllocation, pBuffer: *mut VkBuffer) -> VkResult;
    pub fn vmaDestroyBuffer(allocator: VmaAllocator, buffer: VkBuffer, allocation: VmaAllocation);
    pub fn vmaCreateImage(allocator: VmaAllocator, pImageCreateInfo: *const VkImageCreateInfo, pAllocationCreateInfo: *const VmaAllocationCreateInfo, pImage: *mut VkImage, pAllocation: *mut VmaAllocation, pAllocationInfo: *mut VmaAllocationInfo) -> VkResult;
    pub fn vmaCreateAliasingImage(allocator: VmaAllocator, pImageCreateInfo: *const VkImageCreateInfo, allocation: VmaAllocation, pImage: *mut VkImage) -> VkResult;
    pub fn vmaDestroyImage(allocator: VmaAllocator, image: VkImage, allocation: VmaAllocation);

    // ---- Pools ----
    pub fn vmaCreatePool(allocator: VmaAllocator, pCreateInfo: *const VmaPoolCreateInfo, pPool: *mut VmaPool) -> VkResult;
    pub fn vmaDestroyPool(allocator: VmaAllocator, pool: VmaPool);
    pub fn vmaGetPoolStats(allocator: VmaAllocator, pool: VmaPool, pPoolStats: *mut VmaPoolStats);

    // ---- Defragmentation ----
    pub fn vmaBeginDefragmentation(allocator: VmaAllocator, pInfo: *const VmaDefragmentationInfo, pContext: *mut VmaDefragmentationContext) -> VkResult;
    pub fn vmaEndDefragmentation(allocator: VmaAllocator, context: VmaDefragmentationContext, pStats: *mut VmaDefragmentationPassInfo) -> VkResult;
    pub fn vmaBeginDefragmentationPass(allocator: VmaAllocator, context: VmaDefragmentationContext, pPassInfo: *mut VmaDefragmentationPassInfo) -> VkResult;
    pub fn vmaEndDefragmentationPass(allocator: VmaAllocator, context: VmaDefragmentationContext, pPassInfo: *mut VmaDefragmentationPassInfo) -> VkResult;

    // ---- Statistics ----
    pub fn vmaGetPhysicalDeviceProperties(allocator: VmaAllocator, pPhysicalDeviceProperties: *mut VkPhysicalDeviceProperties);
    pub fn vmaGetMemoryProperties(allocator: VmaAllocator, pPhysicalDeviceMemoryProperties: *mut VkPhysicalDeviceMemoryProperties);
    pub fn vmaGetMemoryTypeProperties(allocator: VmaAllocator, memoryTypeIndex: u32, pFlags: *mut VkMemoryPropertyFlags) -> VkResult;
    pub fn vmaGetTotalStatistics(allocator: VmaAllocator, pStats: *mut VmaTotalStatistics);
    pub fn vmaGetPoolStatistics(allocator: VmaAllocator, pool: VmaPool, pStats: *mut VmaStatistics);
    pub fn vmaGetAllocationStatistics(allocator: VmaAllocator, allocation: VmaAllocation, pStats: *mut VmaStatistics);
    pub fn vmaGetDetailedStatistics(allocator: VmaAllocator, pStats: *mut VmaTotalStatistics);
    pub fn vmaGetBudget(allocator: VmaAllocator, pBudget: *mut VmaBudget);
    pub fn vmaCalculateStatistics(allocator: VmaAllocator, pStats: *mut VmaTotalStatistics);
    pub fn vmaCalculatePoolStatistics(allocator: VmaAllocator, pool: VmaPool, pStats: *mut VmaStatistics);
    pub fn vmaGetVirtualBlockStatistics(allocator: VmaAllocator, block: VmaVirtualBlock, pStats: *mut VmaStatistics);

    // ---- Virtual allocator ----
    pub fn vmaCreateVirtualBlock(pCreateInfo: *const VmaVirtualBlockCreateInfo, pVirtualBlock: *mut VmaVirtualBlock) -> VkResult;
    pub fn vmaDestroyVirtualBlock(virtualBlock: VmaVirtualBlock);
    pub fn vmaIsVirtualBlockEmpty(virtualBlock: VmaVirtualBlock) -> VkBool32;
    pub fn vmaVirtualAllocate(virtualBlock: VmaVirtualBlock, pCreateInfo: *const VmaVirtualAllocationCreateInfo, pAllocation: *mut VmaVirtualAllocation, pOffset: *mut VkDeviceSize) -> VkResult;
    pub fn vmaVirtualFree(virtualBlock: VmaVirtualBlock, allocation: VmaVirtualAllocation) -> VkResult;
    pub fn vmaSetVirtualAllocationUserData(virtualBlock: VmaVirtualBlock, allocation: VmaVirtualAllocation, pUserData: *mut c_void);
    pub fn vmaGetVirtualAllocationInfo(virtualBlock: VmaVirtualBlock, allocation: VmaVirtualAllocation, pVirtualAllocInfo: *mut VmaVirtualAllocationInfo);
    pub fn vmaClearVirtualBlock(virtualBlock: VmaVirtualBlock);
    pub fn vmaGetVirtualBlockAllocationCount(virtualBlock: VmaVirtualBlock) -> VkDeviceSize;
    pub fn vmaBuildVirtualBlockFlags(flags: *mut VmaVirtualBlockCreateFlags, memoryTypeIndex: u32, bufferDeviceAddress: VkDeviceAddress) -> VkResult;

    // ---- String / version helpers ----
    pub fn vmaGetAllocatorInfo(allocator: VmaAllocator, pInfo: *mut VmaAllocatorCreateInfo);
    pub fn vmaGetBufferDeviceAddress(allocator: VmaAllocator, buffer: VkBuffer) -> VkDeviceAddress;
    pub fn vmaGetVulkanFunctions(allocator: VmaAllocator) -> *const c_void;
    pub fn vmaGetCurrentFrameIndex(allocator: VmaAllocator) -> u32;
    pub fn vmaSetCurrentFrameIndex(allocator: VmaAllocator, frameIndex: u32);
    pub fn vmaGetInstanceProcAddr(allocator: VmaAllocator, pName: *const u8) -> *const c_void;
    pub fn vmaGetDeviceProcAddr(allocator: VmaAllocator, pName: *const u8) -> *const c_void;

    // ---- Device memory callbacks ----
    pub fn vmaSetAllocatorDeviceMemoryCallbacks(allocator: VmaAllocator, pCallbacks: *const VmaDeviceMemoryCallbacks);
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct VmaDeviceMemoryCallbacks {
    pub pfnAllocate: PFN_vmaAllocateDeviceMemoryFunction,
    pub pfnFree: PFN_vmaFreeDeviceMemoryFunction,
    pub pUserData: *mut c_void,
}
