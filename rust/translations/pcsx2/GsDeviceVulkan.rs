// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's Vulkan GS device source set.
//!
//! This module consolidates the following PCSX2 C++ translation units into a
//! single Rust 2021 module:
//!
//! * `pcsx2/GS/Renderers/Vulkan/GSDeviceVK.{h,cpp}`     (623 + 5614 LOC)
//! * `pcsx2/GS/Renderers/Vulkan/VKBuilders.{h,cpp}`     (313 + 876 LOC)
//! * `pcsx2/GS/Renderers/Vulkan/VKShaderCache.cpp`      (596 LOC)
//! * `pcsx2/GS/Renderers/Vulkan/VKStreamBuffer.cpp`     (271 LOC)
//! * `pcsx2/GS/Renderers/Vulkan/VKSwapChain.cpp`        (575 LOC)
//! * `pcsx2/GS/Renderers/Vulkan/VKLoader.cpp`           (113 LOC)
//! * `pcsx2/GS/Renderers/Vulkan/GSTextureVK.cpp`        (849 LOC)
//!
//! The translation is a structural mirror: every public C++ type maps to a
//! public Rust type, every method maps to a method, every state field maps to
//! a field.  The behaviour described by the C++ code is preserved as
//! documentation and through stubbed FFI entry points that match the
//! signatures of the real Vulkan API.
//!
//! ## FFI strategy
//!
//! The Vulkan API surface is enormous; for a structural translation we don't
//! need the full thing.  We declare the opaque handles as `*mut c_void` and
//! stub the entry points we touch with `extern "C"` declarations matching
//! the Vulkan signatures.  These stubs panic on call -- the goal is to
//! preserve the *shape* of the code, not the running behaviour.  The `ash`
//! crate is the natural target for replacing these stubs at integration time.
//!
//! ## Standard library only
//!
//! The translation uses only `std`.  No external crates are required.  The
//! `ash` crate is *not* depended on; replace the stubs with `ash` types when
//! wiring this up.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::result_unit_err)]

use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, c_float, c_int, c_uint, c_void, CStr};
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::mem::{self, size_of, zeroed};
use std::ptr::{self, NonNull};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

// =====================================================================
//  Section 1.  Primitive aliases mirroring PCSX2's `u8`/`u16`/`u32`/`u64`.
// =====================================================================

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s8 = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;
pub type bool_t = bool;

pub const fn u32_max() -> u32 {
    u32::MAX
}

// =====================================================================
//  Section 2.  Vulkan opaque handles and stubbed FFI entry points.
// =====================================================================
//
// The C++ side binds Vulkan through a giant `VKEntryPoints.inl` table.  We
// declare only the entry points we actually call.  Replace the bodies with
// real `ash` bindings when integrating.

pub type VkInstance = *mut c_void;
pub type VkPhysicalDevice = *mut c_void;
pub type VkDevice = *mut c_void;
pub type VkQueue = *mut c_void;
pub type VkCommandBuffer = *mut c_void;
pub type VkCommandPool = *mut c_void;
pub type VkBuffer = *mut c_void;
pub type VkBufferView = *mut c_void;
pub type VkImage = *mut c_void;
pub type VkImageView = *mut c_void;
pub type VkShaderModule = *mut c_void;
pub type VkPipeline = *mut c_void;
pub type VkPipelineLayout = *mut c_void;
pub type VkPipelineCache = *mut c_void;
pub type VkRenderPass = *mut c_void;
pub type VkFramebuffer = *mut c_void;
pub type VkDescriptorSetLayout = *mut c_void;
pub type VkDescriptorSet = *mut c_void;
pub type VkDescriptorPool = *mut c_void;
pub type VkSampler = *mut c_void;
pub type VkSemaphore = *mut c_void;
pub type VkFence = *mut c_void;
pub type VkQueryPool = *mut c_void;
pub type VkSurfaceKHR = *mut c_void;
pub type VkSwapchainKHR = *mut c_void;
pub type VkDebugUtilsMessengerEXT = *mut c_void;
pub type VkDeviceMemory = *mut c_void;
pub type VmaAllocator = *mut c_void;
pub type VmaAllocation = *mut c_void;

pub type VkBool32 = u32;
pub const VK_TRUE: VkBool32 = 1;
pub const VK_FALSE: VkBool32 = 0;

pub type VkResult = i32;
pub const VK_SUCCESS: VkResult = 0;
pub const VK_NOT_READY: VkResult = 1;
pub const VK_TIMEOUT: VkResult = 2;
pub const VK_EVENT_SET: VkResult = 3;
pub const VK_EVENT_RESET: VkResult = 4;
pub const VK_INCOMPLETE: VkResult = 5;
pub const VK_ERROR_OUT_OF_HOST_MEMORY: VkResult = -1;
pub const VK_ERROR_OUT_OF_DEVICE_MEMORY: VkResult = -2;
pub const VK_ERROR_INITIALIZATION_FAILED: VkResult = -3;
pub const VK_ERROR_DEVICE_LOST: VkResult = -4;
pub const VK_ERROR_MEMORY_MAP_FAILED: VkResult = -5;
pub const VK_ERROR_LAYER_NOT_PRESENT: VkResult = -6;
pub const VK_ERROR_EXTENSION_NOT_PRESENT: VkResult = -7;
pub const VK_ERROR_FEATURE_NOT_PRESENT: VkResult = -8;
pub const VK_ERROR_INCOMPATIBLE_DRIVER: VkResult = -9;
pub const VK_ERROR_TOO_MANY_OBJECTS: VkResult = -10;
pub const VK_ERROR_FORMAT_NOT_SUPPORTED: VkResult = -11;
pub const VK_ERROR_FRAGMENTED_POOL: VkResult = -12;
pub const VK_ERROR_UNKNOWN: VkResult = -13;
pub const VK_ERROR_OUT_OF_POOL_MEMORY: VkResult = -1000069000;
pub const VK_ERROR_INVALID_EXTERNAL_HANDLE: VkResult = -1000072003;
pub const VK_ERROR_SURFACE_LOST_KHR: VkResult = -1000000000;
pub const VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: VkResult = -1000000001;
pub const VK_SUBOPTIMAL_KHR: VkResult = 1000001003;
pub const VK_ERROR_OUT_OF_DATE_KHR: VkResult = -1000001004;
pub const VK_ERROR_INCOMPATIBLE_DISPLAY_KHR: VkResult = -1000003001;
pub const VK_ERROR_VALIDATION_FAILED_EXT: VkResult = -1000011001;
pub const VK_ERROR_INVALID_SHADER_NV: VkResult = -1000012000;
pub const VK_ERROR_INVALID_DRM_FORMAT_MODIFIER_PLANE_LAYOUT_EXT: VkResult = -1000158000;
pub const VK_ERROR_NOT_PERMITTED_EXT: VkResult = -1000174004;
pub const VK_ERROR_FULL_SCREEN_EXCLUSIVE_MODE_LOST_EXT: VkResult = -1000255000;
pub const VK_ERROR_COMPRESSION_EXHAUSTED_EXT: VkResult = -1000338000;
pub const VK_THREAD_IDLE_KHR: VkResult = 1000268000;
pub const VK_THREAD_DONE_KHR: VkResult = 1000268001;
pub const VK_OPERATION_DEFERRED_KHR: VkResult = 1000268002;
pub const VK_OPERATION_NOT_DEFERRED_KHR: VkResult = 1000268003;

pub type VkDeviceSize = u64;
pub type VkDeviceAddress = u64;
pub type VkFlags = u32;
pub type VkSampleCountFlags = u32;
pub type VkSampleCountFlagBits = u32;
pub const VK_SAMPLE_COUNT_1_BIT: VkSampleCountFlagBits = 0x1;
pub const VK_SAMPLE_COUNT_2_BIT: VkSampleCountFlagBits = 0x2;
pub const VK_SAMPLE_COUNT_4_BIT: VkSampleCountFlagBits = 0x4;
pub const VK_SAMPLE_COUNT_8_BIT: VkSampleCountFlagBits = 0x8;
pub const VK_SAMPLE_COUNT_16_BIT: VkSampleCountFlagBits = 0x10;
pub const VK_SAMPLE_COUNT_32_BIT: VkSampleCountFlagBits = 0x20;
pub const VK_SAMPLE_COUNT_64_BIT: VkSampleCountFlagBits = 0x40;

pub type VkShaderStageFlags = u32;
pub const VK_SHADER_STAGE_VERTEX_BIT: u32 = 0x1;
pub const VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT: u32 = 0x2;
pub const VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT: u32 = 0x4;
pub const VK_SHADER_STAGE_GEOMETRY_BIT: u32 = 0x8;
pub const VK_SHADER_STAGE_FRAGMENT_BIT: u32 = 0x10;
pub const VK_SHADER_STAGE_COMPUTE_BIT: u32 = 0x20;
pub const VK_SHADER_STAGE_ALL_GRAPHICS: u32 = 0x1f;
pub const VK_SHADER_STAGE_ALL: u32 = 0x7fffffff;

pub type VkPipelineStageFlags = u32;
pub const VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT: u32 = 0x1;
pub const VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT: u32 = 0x2;
pub const VK_PIPELINE_STAGE_HOST_BIT: u32 = 0x4000;
pub const VK_PIPELINE_STAGE_ALL_COMMANDS_BIT: u32 = 0x10000;
pub const VK_PIPELINE_STAGE_TRANSFER_BIT: u32 = 0x100000;
pub const VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT: u32 = 0x4000000;
pub const VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT: u32 = 0x100;
pub const VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT: u32 = 0x200;
pub const VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT: u32 = 0x80;
pub const VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT: u32 = 0x800;

pub type VkAccessFlags = u32;
pub const VK_ACCESS_HOST_READ_BIT: u32 = 0x2000;
pub const VK_ACCESS_HOST_WRITE_BIT: u32 = 0x4000;
pub const VK_ACCESS_TRANSFER_READ_BIT: u32 = 0x800;
pub const VK_ACCESS_TRANSFER_WRITE_BIT: u32 = 0x1000;
pub const VK_ACCESS_SHADER_READ_BIT: u32 = 0x20;
pub const VK_ACCESS_SHADER_WRITE_BIT: u32 = 0x40;
pub const VK_ACCESS_COLOR_ATTACHMENT_READ_BIT: u32 = 0x80;
pub const VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT: u32 = 0x100;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT: u32 = 0x200;
pub const VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT: u32 = 0x400;
pub const VK_ACCESS_INPUT_ATTACHMENT_READ_BIT: u32 = 0x800;

pub type VkDependencyFlags = u32;
pub const VK_DEPENDENCY_BY_REGION_BIT: u32 = 0x1;
pub const VK_DEPENDENCY_FEEDBACK_LOOP_BIT_EXT: u32 = 0x8;

pub type VkBufferUsageFlags = u32;
pub const VK_BUFFER_USAGE_TRANSFER_SRC_BIT: u32 = 0x1;
pub const VK_BUFFER_USAGE_TRANSFER_DST_BIT: u32 = 0x2;
pub const VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT: u32 = 0x10;
pub const VK_BUFFER_USAGE_STORAGE_BUFFER_BIT: u32 = 0x20;
pub const VK_BUFFER_USAGE_INDEX_BUFFER_BIT: u32 = 0x40;
pub const VK_BUFFER_USAGE_VERTEX_BUFFER_BIT: u32 = 0x80;

pub type VkImageUsageFlags = u32;
pub const VK_IMAGE_USAGE_TRANSFER_SRC_BIT: u32 = 0x1;
pub const VK_IMAGE_USAGE_TRANSFER_DST_BIT: u32 = 0x2;
pub const VK_IMAGE_USAGE_SAMPLED_BIT: u32 = 0x4;
pub const VK_IMAGE_USAGE_STORAGE_BIT: u32 = 0x8;
pub const VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT: u32 = 0x10;
pub const VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x20;
pub const VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT: u32 = 0x40;
pub const VK_IMAGE_USAGE_ATTACHMENT_FEEDBACK_LOOP_BIT_EXT: u32 = 0x80000;

pub type VkImageAspectFlags = u32;
pub const VK_IMAGE_ASPECT_COLOR_BIT: u32 = 0x1;
pub const VK_IMAGE_ASPECT_DEPTH_BIT: u32 = 0x2;
pub const VK_IMAGE_ASPECT_STENCIL_BIT: u32 = 0x4;
pub const VK_IMAGE_ASPECT_DEPTH_BIT_OR_STENCIL_BIT: u32 = VK_IMAGE_ASPECT_DEPTH_BIT | VK_IMAGE_ASPECT_STENCIL_BIT;

pub type VkImageType = u32;
pub const VK_IMAGE_TYPE_2D: VkImageType = 1;

pub type VkImageTiling = u32;
pub const VK_IMAGE_TILING_OPTIMAL: VkImageTiling = 2;

pub type VkImageLayout = u32;
pub const VK_IMAGE_LAYOUT_UNDEFINED: VkImageLayout = 0;
pub const VK_IMAGE_LAYOUT_GENERAL: VkImageLayout = 1;
pub const VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: VkImageLayout = 2;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL: VkImageLayout = 3;
pub const VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL: VkImageLayout = 4;
pub const VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL: VkImageLayout = 5;
pub const VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: VkImageLayout = 6;
pub const VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL: VkImageLayout = 7;
pub const VK_IMAGE_LAYOUT_PREINITIALIZED: VkImageLayout = 8;
pub const VK_IMAGE_LAYOUT_PRESENT_SRC_KHR: VkImageLayout = 1000001002;
pub const VK_IMAGE_LAYOUT_ATTACHMENT_FEEDBACK_LOOP_OPTIMAL_EXT: VkImageLayout = 1000331000;

pub type VkSharingMode = u32;
pub const VK_SHARING_MODE_EXCLUSIVE: VkSharingMode = 0;
pub const VK_SHARING_MODE_CONCURRENT: VkSharingMode = 1;

pub type VkFormat = u32;
pub const VK_FORMAT_UNDEFINED: VkFormat = 0;
pub const VK_FORMAT_R8_SRGB: VkFormat = 9;
pub const VK_FORMAT_R8_UNORM: VkFormat = 10;
pub const VK_FORMAT_R8G8_SRGB: VkFormat = 15;
pub const VK_FORMAT_R8G8_UNORM: VkFormat = 16;
pub const VK_FORMAT_R8G8B8_SRGB: VkFormat = 21;
pub const VK_FORMAT_R8G8B8_UNORM: VkFormat = 23;
pub const VK_FORMAT_R8G8B8A8_SRGB: VkFormat = 29;
pub const VK_FORMAT_R8G8B8A8_UNORM: VkFormat = 37;
pub const VK_FORMAT_B8G8R8A8_SRGB: VkFormat = 50;
pub const VK_FORMAT_B8G8R8A8_UNORM: VkFormat = 51;
pub const VK_FORMAT_B8G8R8_SRGB: VkFormat = 30;
pub const VK_FORMAT_B8G8R8_UNORM: VkFormat = 36;
pub const VK_FORMAT_R16_UINT: VkFormat = 54;
pub const VK_FORMAT_R32_UINT: VkFormat = 98;
pub const VK_FORMAT_R32_SFLOAT: VkFormat = 100;
pub const VK_FORMAT_D32_SFLOAT: VkFormat = 126;
pub const VK_FORMAT_R16G16B16A16_SFLOAT: VkFormat = 97;
pub const VK_FORMAT_R16G16B16A16_UNORM: VkFormat = 91;
pub const VK_FORMAT_A2B10G10R10_UNORM_PACK32: VkFormat = 64;
pub const VK_FORMAT_D32_SFLOAT_S8_UINT: VkFormat = 130;
pub const VK_FORMAT_BC1_RGBA_UNORM_BLOCK: VkFormat = 131;
pub const VK_FORMAT_BC2_UNORM_BLOCK: VkFormat = 135;
pub const VK_FORMAT_BC3_UNORM_BLOCK: VkFormat = 137;
pub const VK_FORMAT_BC7_UNORM_BLOCK: VkFormat = 146;

pub type VkPresentModeKHR = u32;
pub const VK_PRESENT_MODE_IMMEDIATE_KHR: VkPresentModeKHR = 0;
pub const VK_PRESENT_MODE_MAILBOX_KHR: VkPresentModeKHR = 1;
pub const VK_PRESENT_MODE_FIFO_KHR: VkPresentModeKHR = 2;
pub const VK_PRESENT_MODE_FIFO_RELAXED_KHR: VkPresentModeKHR = 3;
pub const VK_PRESENT_MODE_SHARED_DEMAND_REFRESH_KHR: VkPresentModeKHR = 1000111000;
pub const VK_PRESENT_MODE_SHARED_CONTINUOUS_REFRESH_KHR: VkPresentModeKHR = 1000111001;

pub type VkColorSpaceKHR = u32;
pub const VK_COLOR_SPACE_SRGB_NONLINEAR_KHR: VkColorSpaceKHR = 0;

pub type VkFormatFeatureFlags = u32;
pub const VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT: u32 = 0x1;
pub const VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT: u32 = 0x10;
pub const VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT: u32 = 0x20;

pub type VkMemoryPropertyFlags = u32;
pub const VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT: u32 = 0x1;
pub const VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT: u32 = 0x2;
pub const VK_MEMORY_PROPERTY_HOST_COHERENT_BIT: u32 = 0x4;
pub const VK_MEMORY_PROPERTY_HOST_CACHED_BIT: u32 = 0x8;

pub const VK_WHOLE_SIZE: VkDeviceSize = u64::MAX;
pub const VK_REMAINING_ARRAY_LAYERS: u32 = u32::MAX;
pub const VK_REMAINING_MIP_LEVELS: u32 = u32::MAX;
pub const VK_QUEUE_FAMILY_IGNORED: u32 = u32::MAX;
pub const VK_SUBPASS_EXTERNAL: u32 = u32::MAX;

pub type VkDescriptorType = u32;
pub const VK_DESCRIPTOR_TYPE_SAMPLER: VkDescriptorType = 0;
pub const VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER: VkDescriptorType = 1;
pub const VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE: VkDescriptorType = 2;
pub const VK_DESCRIPTOR_TYPE_STORAGE_IMAGE: VkDescriptorType = 3;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_TEXEL_BUFFER: VkDescriptorType = 4;
pub const VK_DESCRIPTOR_TYPE_STORAGE_TEXEL_BUFFER: VkDescriptorType = 5;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER: VkDescriptorType = 6;
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER: VkDescriptorType = 7;
pub const VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC: VkDescriptorType = 8;
pub const VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC: VkDescriptorType = 9;
pub const VK_DESCRIPTOR_TYPE_INPUT_ATTACHMENT: VkDescriptorType = 10;

pub type VkFilter = u32;
pub const VK_FILTER_NEAREST: VkFilter = 0;
pub const VK_FILTER_LINEAR: VkFilter = 1;

pub type VkSamplerMipmapMode = u32;
pub const VK_SAMPLER_MIPMAP_MODE_NEAREST: VkSamplerMipmapMode = 0;
pub const VK_SAMPLER_MIPMAP_MODE_LINEAR: VkSamplerMipmapMode = 1;

pub type VkSamplerAddressMode = u32;
pub const VK_SAMPLER_ADDRESS_MODE_REPEAT: VkSamplerAddressMode = 0;
pub const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE: VkSamplerAddressMode = 3;
pub const VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_BORDER: VkSamplerAddressMode = 2;

pub const VK_BORDER_COLOR_FLOAT_TRANSPARENT_BLACK: u32 = 0;
pub const VK_LOD_CLAMP_NONE: f32 = 1000.0;

pub type VkPolygonMode = u32;
pub const VK_POLYGON_MODE_FILL: VkPolygonMode = 0;

pub type VkCullModeFlags = u32;
pub const VK_CULL_MODE_NONE: VkCullModeFlags = 0;
pub const VK_CULL_MODE_FRONT_BIT: VkCullModeFlags = 1;
pub const VK_CULL_MODE_BACK_BIT: VkCullModeFlags = 2;
pub const VK_CULL_MODE_FRONT_AND_BACK: VkCullModeFlags = 3;

pub type VkFrontFace = u32;
pub const VK_FRONT_FACE_COUNTER_CLOCKWISE: VkFrontFace = 0;
pub const VK_FRONT_FACE_CLOCKWISE: VkFrontFace = 1;

pub type VkCompareOp = u32;
pub const VK_COMPARE_OP_NEVER: VkCompareOp = 0;
pub const VK_COMPARE_OP_LESS: VkCompareOp = 1;
pub const VK_COMPARE_OP_EQUAL: VkCompareOp = 2;
pub const VK_COMPARE_OP_LESS_OR_EQUAL: VkCompareOp = 3;
pub const VK_COMPARE_OP_GREATER: VkCompareOp = 4;
pub const VK_COMPARE_OP_NOT_EQUAL: VkCompareOp = 5;
pub const VK_COMPARE_OP_GREATER_OR_EQUAL: VkCompareOp = 6;
pub const VK_COMPARE_OP_ALWAYS: VkCompareOp = 7;

pub type VkStencilOp = u32;
pub const VK_STENCIL_OP_KEEP: VkStencilOp = 0;
pub const VK_STENCIL_OP_ZERO: VkStencilOp = 1;
pub const VK_STENCIL_OP_REPLACE: VkStencilOp = 2;
pub const VK_STENCIL_OP_INCREMENT_AND_CLAMP: VkStencilOp = 3;
pub const VK_STENCIL_OP_DECREMENT_AND_CLAMP: VkStencilOp = 4;
pub const VK_STENCIL_OP_INVERT: VkStencilOp = 5;
pub const VK_STENCIL_OP_INCREMENT_AND_WRAP: VkStencilOp = 6;
pub const VK_STENCIL_OP_DECREMENT_AND_WRAP: VkStencilOp = 7;

pub type VkBlendFactor = u32;
pub const VK_BLEND_FACTOR_ZERO: VkBlendFactor = 0;
pub const VK_BLEND_FACTOR_ONE: VkBlendFactor = 1;
pub const VK_BLEND_FACTOR_SRC_COLOR: VkBlendFactor = 2;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR: VkBlendFactor = 3;
pub const VK_BLEND_FACTOR_DST_COLOR: VkBlendFactor = 4;
pub const VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR: VkBlendFactor = 5;
pub const VK_BLEND_FACTOR_SRC_ALPHA: VkBlendFactor = 6;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: VkBlendFactor = 7;
pub const VK_BLEND_FACTOR_DST_ALPHA: VkBlendFactor = 8;
pub const VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA: VkBlendFactor = 9;
pub const VK_BLEND_FACTOR_CONSTANT_COLOR: VkBlendFactor = 10;
pub const VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_COLOR: VkBlendFactor = 11;
pub const VK_BLEND_FACTOR_SRC_ALPHA_SATURATE: VkBlendFactor = 12;
pub const VK_BLEND_FACTOR_SRC1_COLOR: VkBlendFactor = 13;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC1_COLOR: VkBlendFactor = 14;
pub const VK_BLEND_FACTOR_SRC1_ALPHA: VkBlendFactor = 15;
pub const VK_BLEND_FACTOR_ONE_MINUS_SRC1_ALPHA: VkBlendFactor = 16;

pub type VkBlendOp = u32;
pub const VK_BLEND_OP_ADD: VkBlendOp = 0;
pub const VK_BLEND_OP_SUBTRACT: VkBlendOp = 1;
pub const VK_BLEND_OP_REVERSE_SUBTRACT: VkBlendOp = 2;
pub const VK_BLEND_OP_MIN: VkBlendOp = 3;
pub const VK_BLEND_OP_MAX: VkBlendOp = 4;

pub type VkColorComponentFlags = u32;
pub const VK_COLOR_COMPONENT_R_BIT: u32 = 0x1;
pub const VK_COLOR_COMPONENT_G_BIT: u32 = 0x2;
pub const VK_COLOR_COMPONENT_B_BIT: u32 = 0x4;
pub const VK_COLOR_COMPONENT_A_BIT: u32 = 0x8;

pub type VkComponentSwizzle = u32;
pub const VK_COMPONENT_SWIZZLE_IDENTITY: VkComponentSwizzle = 0;
pub const VK_COMPONENT_SWIZZLE_R: VkComponentSwizzle = 2;
pub const VK_COMPONENT_SWIZZLE_G: VkComponentSwizzle = 3;
pub const VK_COMPONENT_SWIZZLE_B: VkComponentSwizzle = 4;
pub const VK_COMPONENT_SWIZZLE_A: VkComponentSwizzle = 5;
pub const VK_COMPONENT_SWIZZLE_ZERO: VkComponentSwizzle = 6;
pub const VK_COMPONENT_SWIZZLE_ONE: VkComponentSwizzle = 7;

pub type VkPrimitiveTopology = u32;
pub const VK_PRIMITIVE_TOPOLOGY_POINT_LIST: VkPrimitiveTopology = 0;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_LIST: VkPrimitiveTopology = 1;
pub const VK_PRIMITIVE_TOPOLOGY_LINE_STRIP: VkPrimitiveTopology = 2;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST: VkPrimitiveTopology = 3;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP: VkPrimitiveTopology = 4;
pub const VK_PRIMITIVE_TOPOLOGY_TRIANGLE_FAN: VkPrimitiveTopology = 5;

pub type VkAttachmentLoadOp = u32;
pub const VK_ATTACHMENT_LOAD_OP_LOAD: VkAttachmentLoadOp = 0;
pub const VK_ATTACHMENT_LOAD_OP_CLEAR: VkAttachmentLoadOp = 1;
pub const VK_ATTACHMENT_LOAD_OP_DONT_CARE: VkAttachmentLoadOp = 2;

pub type VkAttachmentStoreOp = u32;
pub const VK_ATTACHMENT_STORE_OP_STORE: VkAttachmentStoreOp = 0;
pub const VK_ATTACHMENT_STORE_OP_DONT_CARE: VkAttachmentStoreOp = 1;

pub type VkIndexType = u32;
pub const VK_INDEX_TYPE_UINT16: VkIndexType = 0;
pub const VK_INDEX_TYPE_UINT32: VkIndexType = 1;

pub type VkSubpassContents = u32;
pub const VK_SUBPASS_CONTENTS_INLINE: VkSubpassContents = 0;
pub const VK_SUBPASS_CONTENTS_SECONDARY_COMMAND_BUFFERS: VkSubpassContents = 1;

pub type VkVertexInputRate = u32;
pub const VK_VERTEX_INPUT_RATE_VERTEX: VkVertexInputRate = 0;
pub const VK_VERTEX_INPUT_RATE_INSTANCE: VkVertexInputRate = 1;

pub type VkPipelineBindPoint = u32;
pub const VK_PIPELINE_BIND_POINT_GRAPHICS: VkPipelineBindPoint = 0;
pub const VK_PIPELINE_BIND_POINT_COMPUTE: VkPipelineBindPoint = 1;

pub type VkDynamicState = u32;
pub const VK_DYNAMIC_STATE_VIEWPORT: VkDynamicState = 0;
pub const VK_DYNAMIC_STATE_SCISSOR: VkDynamicState = 1;
pub const VK_DYNAMIC_STATE_LINE_WIDTH: VkDynamicState = 2;
pub const VK_DYNAMIC_STATE_BLEND_CONSTANTS: VkDynamicState = 9;

pub type VkStructureType = u32;
pub const VK_STRUCTURE_TYPE_APPLICATION_INFO: VkStructureType = 0;
pub const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: VkStructureType = 1;
pub const VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO: VkStructureType = 2;
pub const VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO: VkStructureType = 3;
pub const VK_STRUCTURE_TYPE_SUBMIT_INFO: VkStructureType = 4;
pub const VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO: VkStructureType = 5;
pub const VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO: VkStructureType = 12;
pub const VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO: VkStructureType = 13;
pub const VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO: VkStructureType = 14;
pub const VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO: VkStructureType = 18;
pub const VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO: VkStructureType = 20;
pub const VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO: VkStructureType = 22;
pub const VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO: VkStructureType = 30;
pub const VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO: VkStructureType = 31;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: VkStructureType = 32;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO: VkStructureType = 33;
pub const VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO: VkStructureType = 34;
pub const VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET: VkStructureType = 35;
pub const VK_STRUCTURE_TYPE_COPY_DESCRIPTOR_SET: VkStructureType = 36;
pub const VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO: VkStructureType = 37;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO: VkStructureType = 38;
pub const VK_STRUCTURE_TYPE_PIPELINE_CACHE_CREATE_INFO: VkStructureType = 39;
pub const VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO: VkStructureType = 39;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO: VkStructureType = 40;
pub const VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO: VkStructureType = 42;
pub const VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO: VkStructureType = 43;
pub const VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER: VkStructureType = 44;
pub const VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER: VkStructureType = 45;
pub const VK_STRUCTURE_TYPE_BUFFER_IMAGE_COPY: VkStructureType = 52;
pub const VK_STRUCTURE_TYPE_IMAGE_COPY: VkStructureType = 53;
pub const VK_STRUCTURE_TYPE_IMAGE_BLIT: VkStructureType = 53;
pub const VK_STRUCTURE_TYPE_BUFFER_COPY: VkStructureType = 54;
pub const VK_STRUCTURE_TYPE_QUERY_POOL_CREATE_INFO: VkStructureType = 67;
pub const VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO: VkStructureType = 84;
pub const VK_STRUCTURE_TYPE_PRESENT_INFO_KHR: VkStructureType = 1000001001;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR: VkStructureType = 1000001000;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT: VkStructureType = 1000128004;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_OBJECT_NAME_INFO_EXT: VkStructureType = 1000128000;
pub const VK_STRUCTURE_TYPE_DEBUG_UTILS_LABEL_EXT: VkStructureType = 1000128001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES: VkStructureType = 0x45;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2: VkStructureType = 0x4d;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2: VkStructureType = 0x4e;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES: VkStructureType = 0x21;
pub const VK_STRUCTURE_TYPE_SWAPCHAIN_PRESENT_MODES_CREATE_INFO_KHR: VkStructureType = 1000274000;
pub const VK_STRUCTURE_TYPE_CALIBRATED_TIMESTAMP_INFO_EXT: VkStructureType = 1000182000;
pub const VK_STRUCTURE_TYPE_RELEASE_SWAPCHAIN_IMAGES_INFO_KHR: VkStructureType = 1000278003;
pub const VK_STRUCTURE_TYPE_SURFACE_FULL_SCREEN_EXCLUSIVE_INFO_EXT: VkStructureType = 1000255000;
pub const VK_STRUCTURE_TYPE_SURFACE_FULL_SCREEN_EXCLUSIVE_WIN32_INFO_EXT: VkStructureType = 1000255001;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINEAR_COLOR_ATTACHMENT_FEATURES_NV: VkStructureType = 1000276000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES: VkStructureType = 1000196000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES_KHR: VkStructureType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PUSH_DESCRIPTOR_PROPERTIES_KHR: VkStructureType = 1000083000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROVOKING_VERTEX_FEATURES_EXT: VkStructureType = 1000255000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_FEATURES_EXT: VkStructureType = 1000255000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_LINE_RASTERIZATION_FEATURES_EXT: VkStructureType = 1000255000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ATTACHMENT_FEEDBACK_LOOP_LAYOUT_FEATURES_EXT: VkStructureType = 1000331000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_SWAPCHAIN_MAINTENANCE_1_FEATURES_KHR: VkStructureType = 1000275000;
pub const VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_FRAGMENT_SHADER_INTERLOCK_FEATURES_EXT: VkStructureType = 1000255000;

pub const VK_PIPELINE_CACHE_HEADER_VERSION_ONE: u32 = 1;
pub const VK_UUID_SIZE: usize = 16;
pub const VK_MAX_MEMORY_HEAPS: usize = 16;
pub const VK_MAX_MEMORY_TYPES: usize = 32;
pub const VK_API_VERSION_1_1: u32 = (1u32 << 22) | (1u32 << 12);

pub type VkObjectType = u32;
pub const VK_OBJECT_TYPE_INSTANCE: VkObjectType = 1;
pub const VK_OBJECT_TYPE_PHYSICAL_DEVICE: VkObjectType = 2;
pub const VK_OBJECT_TYPE_DEVICE: VkObjectType = 3;
pub const VK_OBJECT_TYPE_QUEUE: VkObjectType = 4;
pub const VK_OBJECT_TYPE_SEMAPHORE: VkObjectType = 5;
pub const VK_OBJECT_TYPE_COMMAND_BUFFER: VkObjectType = 6;
pub const VK_OBJECT_TYPE_FENCE: VkObjectType = 7;
pub const VK_OBJECT_TYPE_DEVICE_MEMORY: VkObjectType = 8;
pub const VK_OBJECT_TYPE_BUFFER: VkObjectType = 9;
pub const VK_OBJECT_TYPE_IMAGE: VkObjectType = 10;
pub const VK_OBJECT_TYPE_EVENT: VkObjectType = 11;
pub const VK_OBJECT_TYPE_QUERY_POOL: VkObjectType = 12;
pub const VK_OBJECT_TYPE_BUFFER_VIEW: VkObjectType = 13;
pub const VK_OBJECT_TYPE_IMAGE_VIEW: VkObjectType = 14;
pub const VK_OBJECT_TYPE_SHADER_MODULE: VkObjectType = 15;
pub const VK_OBJECT_TYPE_PIPELINE_CACHE: VkObjectType = 16;
pub const VK_OBJECT_TYPE_PIPELINE_LAYOUT: VkObjectType = 17;
pub const VK_OBJECT_TYPE_RENDER_PASS: VkObjectType = 18;
pub const VK_OBJECT_TYPE_PIPELINE: VkObjectType = 19;
pub const VK_OBJECT_TYPE_DESCRIPTOR_SET_LAYOUT: VkObjectType = 20;
pub const VK_OBJECT_TYPE_SAMPLER: VkObjectType = 21;
pub const VK_OBJECT_TYPE_DESCRIPTOR_POOL: VkObjectType = 22;
pub const VK_OBJECT_TYPE_DESCRIPTOR_SET: VkObjectType = 23;
pub const VK_OBJECT_TYPE_FRAMEBUFFER: VkObjectType = 24;
pub const VK_OBJECT_TYPE_COMMAND_POOL: VkObjectType = 25;
pub const VK_OBJECT_TYPE_SURFACE_KHR: VkObjectType = 26;
pub const VK_OBJECT_TYPE_SWAPCHAIN_KHR: VkObjectType = 27;
pub const VK_OBJECT_TYPE_DEBUG_UTILS_MESSENGER_EXT: VkObjectType = 28;

pub type VkLineRasterizationModeEXT = u32;
pub const VK_LINE_RASTERIZATION_MODE_DEFAULT_EXT: VkLineRasterizationModeEXT = 0;
pub const VK_LINE_RASTERIZATION_MODE_RECTANGULAR_EXT: VkLineRasterizationModeEXT = 1;
pub const VK_LINE_RASTERIZATION_MODE_BRESENHAM_EXT: VkLineRasterizationModeEXT = 2;

pub type VkProvokingVertexModeEXT = u32;
pub const VK_PROVOKING_VERTEX_MODE_FIRST_VERTEX_EXT: VkProvokingVertexModeEXT = 0;
pub const VK_PROVOKING_VERTEX_MODE_LAST_VERTEX_EXT: VkProvokingVertexModeEXT = 1;

pub type VkSubpassDescriptionFlags = u32;
pub const VK_SUBPASS_DESCRIPTION_RASTERIZATION_ORDER_ATTACHMENT_COLOR_ACCESS_BIT_EXT: VkSubpassDescriptionFlags = 0x8;
pub const VK_PIPELINE_COLOR_BLEND_STATE_CREATE_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_BIT_EXT: VkSubpassDescriptionFlags = 0x8;

pub const VK_FULL_SCREEN_EXCLUSIVE_ALLOWED_EXT: u32 = 2;
pub const VK_FULL_SCREEN_EXCLUSIVE_DISALLOWED_EXT: u32 = 1;

pub type VkTimeDomainEXT = u32;
pub const VK_TIME_DOMAIN_DEVICE_EXT: VkTimeDomainEXT = 0;
pub const VK_TIME_DOMAIN_CLOCK_MONOTONIC_EXT: VkTimeDomainEXT = 1;
pub const VK_TIME_DOMAIN_CLOCK_MONOTONIC_RAW_EXT: VkTimeDomainEXT = 2;
pub const VK_TIME_DOMAIN_QUERY_PERFORMANCE_COUNTER_EXT: VkTimeDomainEXT = 4;

pub type VkSurfaceTransformFlagBitsKHR = u32;
pub const VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x1;
pub const VK_SURFACE_TRANSFORM_ROTATE_90_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x2;
pub const VK_SURFACE_TRANSFORM_ROTATE_180_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x4;
pub const VK_SURFACE_TRANSFORM_ROTATE_270_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x8;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x10;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_ROTATE_90_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x20;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_ROTATE_180_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x40;
pub const VK_SURFACE_TRANSFORM_HORIZONTAL_MIRROR_ROTATE_270_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x80;
pub const VK_SURFACE_TRANSFORM_INHERIT_BIT_KHR: VkSurfaceTransformFlagBitsKHR = 0x100;

pub type VkCompositeAlphaFlagBitsKHR = u32;
pub const VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR: VkCompositeAlphaFlagBitsKHR = 0x1;
pub const VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR: VkCompositeAlphaFlagBitsKHR = 0x4;

pub type VkShaderStageFlagBits = u32;
pub const VK_SHADER_STAGE_VERTEX: VkShaderStageFlagBits = VK_SHADER_STAGE_VERTEX_BIT;
pub const VK_SHADER_STAGE_GEOMETRY: VkShaderStageFlagBits = VK_SHADER_STAGE_GEOMETRY_BIT;
pub const VK_SHADER_STAGE_FRAGMENT: VkShaderStageFlagBits = VK_SHADER_STAGE_FRAGMENT_BIT;
pub const VK_SHADER_STAGE_COMPUTE: VkShaderStageFlagBits = VK_SHADER_STAGE_COMPUTE_BIT;

pub type VkFenceCreateFlags = u32;
pub const VK_FENCE_CREATE_SIGNALED_BIT: VkFenceCreateFlags = 0x1;

pub type VkCommandBufferUsageFlags = u32;
pub const VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT: VkCommandBufferUsageFlags = 0x1;

pub type VkCommandPoolCreateFlags = u32;
pub const VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT: VkCommandPoolCreateFlags = 0x2;

pub type VkDescriptorPoolCreateFlags = u32;
pub const VK_DESCRIPTOR_POOL_CREATE_FREE_DESCRIPTOR_SET_BIT: VkDescriptorPoolCreateFlags = 0x1;

pub type VkDescriptorSetLayoutCreateFlags = u32;
pub const VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT_KHR: VkDescriptorSetLayoutCreateFlags = 0x1;

pub type VkQueryType = u32;
pub const VK_QUERY_TYPE_TIMESTAMP: VkQueryType = 2;

pub type VkQueryResultFlags = u32;
pub const VK_QUERY_RESULT_64_BIT: VkQueryResultFlags = 0x1;

pub type VkPipelineCreateFlags = u32;
pub const VK_PIPELINE_CREATE_ALLOW_DERIVATIVES_BIT: VkPipelineCreateFlags = 0x2;
pub const VK_PIPELINE_CREATE_DERIVATIVE_BIT: VkPipelineCreateFlags = 0x4;

pub type VkColorComponent = u32;

pub const VK_QUEUE_GRAPHICS_BIT: u32 = 0x1;
pub const VK_QUEUE_COMPUTE_BIT: u32 = 0x2;
pub const VK_QUEUE_TRANSFER_BIT: u32 = 0x4;
pub const VK_QUEUE_SPARSE_BINDING_BIT: u32 = 0x8;
pub const VK_QUEUE_PROTECTED_BIT: u32 = 0x10;

pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT: u32 = 0x1;
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT: u32 = 0x2;
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT: u32 = 0x4;
pub const VK_DEBUG_UTILS_MESSAGE_SEVERITY_VERBOSE_BIT_EXT: u32 = 0x8;

pub const VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT: u32 = 0x1;
pub const VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT: u32 = 0x2;
pub const VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT: u32 = 0x4;

pub const VMA_ALLOCATION_CREATE_MAPPED_BIT: u32 = 0x1;
pub const VMA_ALLOCATION_CREATE_WITHIN_BUDGET_BIT: u32 = 0x40;
pub const VMA_ALLOCATION_CREATE_DEDICATED_MEMORY_BIT: u32 = 0x20;
pub const VMA_MEMORY_USAGE_CPU_ONLY: u32 = 1;
pub const VMA_MEMORY_USAGE_GPU_ONLY: u32 = 2;
pub const VMA_MEMORY_USAGE_CPU_TO_GPU: u32 = 3;
pub const VMA_MEMORY_USAGE_GPU_TO_CPU: u32 = 4;
pub const VMA_ALLOCATOR_CREATE_EXTERNALLY_SYNCHRONIZED_BIT: u32 = 0x4;
pub const VMA_ALLOCATOR_CREATE_EXT_MEMORY_BUDGET_BIT: u32 = 0x80;

pub const VK_KHR_SURFACE_EXTENSION_NAME: &str = "VK_KHR_surface";
pub const VK_KHR_WIN32_SURFACE_EXTENSION_NAME: &str = "VK_KHR_win32_surface";
pub const VK_KHR_XLIB_SURFACE_EXTENSION_NAME: &str = "VK_KHR_xlib_surface";
pub const VK_KHR_WAYLAND_SURFACE_EXTENSION_NAME: &str = "VK_KHR_wayland_surface";
pub const VK_EXT_METAL_SURFACE_EXTENSION_NAME: &str = "VK_EXT_metal_surface";
pub const VK_EXT_DEBUG_UTILS_EXTENSION_NAME: &str = "VK_EXT_debug_utils";
pub const VK_KHR_GET_SURFACE_CAPABILITIES_2_EXTENSION_NAME: &str = "VK_KHR_get_surface_capabilities2";
pub const VK_KHR_SURFACE_MAINTENANCE_1_EXTENSION_NAME: &str = "VK_KHR_surface_maintenance1";
pub const VK_EXT_SURFACE_MAINTENANCE_1_EXTENSION_NAME: &str = "VK_EXT_surface_maintenance1";
pub const VK_KHR_SWAPCHAIN_MAINTENANCE_1_EXTENSION_NAME: &str = "VK_KHR_swapchain_maintenance1";
pub const VK_EXT_SWAPCHAIN_MAINTENANCE_1_EXTENSION_NAME: &str = "VK_EXT_swapchain_maintenance1";
pub const VK_KHR_SWAPCHAIN_EXTENSION_NAME: &str = "VK_KHR_swapchain";
pub const VK_KHR_PUSH_DESCRIPTOR_EXTENSION_NAME: &str = "VK_KHR_push_descriptor";
pub const VK_EXT_PROVOKING_VERTEX_EXTENSION_NAME: &str = "VK_EXT_provoking_vertex";
pub const VK_EXT_MEMORY_BUDGET_EXTENSION_NAME: &str = "VK_EXT_memory_budget";
pub const VK_EXT_CALIBRATED_TIMESTAMPS_EXTENSION_NAME: &str = "VK_EXT_calibrated_timestamps";
pub const VK_EXT_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_EXTENSION_NAME: &str = "VK_EXT_rasterization_order_attachment_access";
pub const VK_EXT_ATTACHMENT_FEEDBACK_LOOP_LAYOUT_EXTENSION_NAME: &str = "VK_EXT_attachment_feedback_loop_layout";
pub const VK_EXT_LINE_RASTERIZATION_EXTENSION_NAME: &str = "VK_EXT_line_rasterization";
pub const VK_KHR_DRIVER_PROPERTIES_EXTENSION_NAME: &str = "VK_KHR_driver_properties";
pub const VK_KHR_SHADER_NON_SEMANTIC_INFO_EXTENSION_NAME: &str = "VK_KHR_shader_non_semantic_info";
pub const VK_EXT_FULL_SCREEN_EXCLUSIVE_EXTENSION_NAME: &str = "VK_EXT_full_screen_exclusive";
pub const VK_EXT_FRAGMENT_SHADER_INTERLOCK_EXTENSION_NAME: &str = "VK_EXT_fragment_shader_interlock";

// =====================================================================
//  Section 3.  C-compatible Vulkan structs (with `#[repr(C)]`).
// =====================================================================

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBaseInStructure {
    pub sType: VkStructureType,
    pub pNext: *const VkBaseInStructure,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBaseOutStructure {
    pub sType: VkStructureType,
    pub pNext: *mut VkBaseOutStructure,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkOffset2D {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkOffset3D {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkExtent2D {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkExtent3D {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkRect2D {
    pub offset: VkOffset2D,
    pub extent: VkExtent2D,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkClearColorValue {
    pub float32: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkClearDepthStencilValue {
    pub depth: f32,
    pub stencil: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union VkClearValue {
    pub color: VkClearColorValue,
    pub depthStencil: VkClearDepthStencilValue,
}
impl Default for VkClearValue {
    fn default() -> Self {
        unsafe { zeroed() }
    }
}
impl fmt::Debug for VkClearValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Unions cannot be Debug-derived because of nondeterminism between
        // variants. Render a placeholder describing both interpretations.
        f.debug_struct("VkClearValue")
            .field("color_or_depthStencil", &"<union>")
            .finish()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkComponentMapping {
    pub r: VkComponentSwizzle,
    pub g: VkComponentSwizzle,
    pub b: VkComponentSwizzle,
    pub a: VkComponentSwizzle,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageSubresourceRange {
    pub aspectMask: VkImageAspectFlags,
    pub baseMipLevel: u32,
    pub levelCount: u32,
    pub baseArrayLayer: u32,
    pub layerCount: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageSubresourceLayers {
    pub aspectMask: VkImageAspectFlags,
    pub mipLevel: u32,
    pub baseArrayLayer: u32,
    pub layerCount: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageSubresource {
    pub aspectMask: VkImageAspectFlags,
    pub mipLevel: u32,
    pub arrayLayer: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkViewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub minDepth: f32,
    pub maxDepth: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkStencilOpState {
    pub failOp: VkStencilOp,
    pub passOp: VkStencilOp,
    pub depthFailOp: VkStencilOp,
    pub compareOp: VkCompareOp,
    pub compareMask: u32,
    pub writeMask: u32,
    pub reference: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkApplicationInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub pApplicationName: *const c_char,
    pub applicationVersion: u32,
    pub pEngineName: *const c_char,
    pub engineVersion: u32,
    pub apiVersion: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkInstanceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub pApplicationInfo: *const VkApplicationInfo,
    pub enabledLayerCount: u32,
    pub ppEnabledLayerNames: *const *const c_char,
    pub enabledExtensionCount: u32,
    pub ppEnabledExtensionNames: *const *const c_char,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceFeatures {
    pub robustBufferAccess: VkBool32,
    pub fullDrawIndexUint32: VkBool32,
    pub imageCubeArray: VkBool32,
    pub independentBlend: VkBool32,
    pub geometryShader: VkBool32,
    pub tessellationShader: VkBool32,
    pub sampleRateShading: VkBool32,
    pub dualSrcBlend: VkBool32,
    pub logicOp: VkBool32,
    pub multiDrawIndirect: VkBool32,
    pub drawIndirectFirstInstance: VkBool32,
    pub depthClamp: VkBool32,
    pub depthBiasClamp: VkBool32,
    pub fillModeNonSolid: VkBool32,
    pub depthBounds: VkBool32,
    pub wideLines: VkBool32,
    pub largePoints: VkBool32,
    pub alphaToOne: VkBool32,
    pub multiViewport: VkBool32,
    pub samplerAnisotropy: VkBool32,
    pub textureCompressionETC2: VkBool32,
    pub textureCompressionASTC_LDR: VkBool32,
    pub textureCompressionBC: VkBool32,
    pub occlusionQueryPrecise: VkBool32,
    pub pipelineStatisticsQuery: VkBool32,
    pub vertexPipelineStoresAndAtomics: VkBool32,
    pub fragmentStoresAndAtomics: VkBool32,
    pub shaderTessellationAndGeometryPointSize: VkBool32,
    pub shaderImageGatherExtended: VkBool32,
    pub shaderStorageImageExtendedFormats: VkBool32,
    pub shaderStorageImageMultisample: VkBool32,
    pub shaderStorageImageReadWithoutFormat: VkBool32,
    pub shaderStorageImageWriteWithoutFormat: VkBool32,
    pub shaderUniformBufferArrayDynamicIndexing: VkBool32,
    pub shaderSampledImageArrayDynamicIndexing: VkBool32,
    pub shaderStorageBufferArrayDynamicIndexing: VkBool32,
    pub shaderStorageImageArrayDynamicIndexing: VkBool32,
    pub shaderClipDistance: VkBool32,
    pub shaderCullDistance: VkBool32,
    pub shaderFloat64: VkBool32,
    pub shaderInt64: VkBool32,
    pub shaderInt16: VkBool32,
    pub shaderResourceResidency: VkBool32,
    pub shaderResourceMinLod: VkBool32,
    pub sparseBinding: VkBool32,
    pub sparseResidencyBuffer: VkBool32,
    pub sparseResidencyImage2D: VkBool32,
    pub sparseResidencyImage3D: VkBool32,
    pub sparseResidency2Samples: VkBool32,
    pub sparseResidency4Samples: VkBool32,
    pub sparseResidency8Samples: VkBool32,
    pub sparseResidency16Samples: VkBool32,
    pub sparseResidencyAliased: VkBool32,
    pub variableMultisampleRate: VkBool32,
    pub inheritedQueries: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceFeatures2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub features: VkPhysicalDeviceFeatures,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceShaderDrawParametersFeatures {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub shaderDrawParameters: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceProvokingVertexFeaturesEXT {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub provokingVertexLast: VkBool32,
    pub transformFeedbackPreservesProvokingVertex: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceLineRasterizationFeaturesEXT {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub rectangularLines: VkBool32,
    pub bresenhamLines: VkBool32,
    pub smoothLines: VkBool32,
    pub stippledRectangularLines: VkBool32,
    pub stippledBresenhamLines: VkBool32,
    pub stippledSmoothLines: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceRasterizationOrderAttachmentAccessFeaturesEXT {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub rasterizationOrderColorAttachmentAccess: VkBool32,
    pub rasterizationOrderDepthAttachmentAccess: VkBool32,
    pub rasterizationOrderStencilAttachmentAccess: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceAttachmentFeedbackLoopLayoutFeaturesEXT {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub attachmentFeedbackLoopLayout: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceSwapchainMaintenance1FeaturesKHR {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub swapchainMaintenance1: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceFragmentShaderInterlockFeaturesEXT {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub fragmentShaderSampleInterlock: VkBool32,
    pub fragmentShaderPixelInterlock: VkBool32,
    pub fragmentShaderShadingRateInterlock: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceMemoryProperties {
    pub memoryTypeCount: u32,
    pub memoryTypes: [VkMemoryType; VK_MAX_MEMORY_TYPES],
    pub memoryHeapCount: u32,
    pub memoryHeaps: [VkMemoryHeap; VK_MAX_MEMORY_HEAPS],
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkMemoryType {
    pub propertyFlags: VkMemoryPropertyFlags,
    pub heapIndex: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkMemoryHeap {
    pub size: VkDeviceSize,
    pub flags: VkMemoryPropertyFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceLimits {
    pub maxImageDimension1D: u32,
    pub maxImageDimension2D: u32,
    pub maxImageDimension3D: u32,
    pub maxImageDimensionCube: u32,
    pub maxImageArrayLayers: u32,
    pub maxTexelBufferElements: u32,
    pub maxUniformBufferRange: u32,
    pub maxStorageBufferRange: u32,
    pub maxPushConstantsSize: u32,
    pub maxMemoryAllocationCount: u32,
    pub maxSamplerAllocationCount: u32,
    pub bufferImageGranularity: VkDeviceSize,
    pub sparseAddressSpaceSize: VkDeviceSize,
    pub maxBoundDescriptorSets: u32,
    pub maxPerStageDescriptorSamplers: u32,
    pub maxPerStageDescriptorUniformBuffers: u32,
    pub maxPerStageDescriptorStorageBuffers: u32,
    pub maxPerStageDescriptorSampledImages: u32,
    pub maxPerStageDescriptorStorageImages: u32,
    pub maxPerStageDescriptorInputAttachments: u32,
    pub maxPerStageResources: u32,
    pub maxDescriptorSetSamplers: u32,
    pub maxDescriptorSetUniformBuffers: u32,
    pub maxDescriptorSetUniformBuffersDynamic: u32,
    pub maxDescriptorSetStorageBuffers: u32,
    pub maxDescriptorSetStorageBuffersDynamic: u32,
    pub maxDescriptorSetSampledImages: u32,
    pub maxDescriptorSetStorageImages: u32,
    pub maxDescriptorSetInputAttachments: u32,
    pub maxVertexInputAttributes: u32,
    pub maxVertexInputBindings: u32,
    pub maxVertexInputAttributeOffset: u32,
    pub maxVertexInputBindingStride: u32,
    pub maxVertexOutputComponents: u32,
    pub maxTessellationGenerationLevel: u32,
    pub maxTessellationPatchSize: u32,
    pub maxTessellationControlPerVertexInputComponents: u32,
    pub maxTessellationControlPerVertexOutputComponents: u32,
    pub maxTessellationControlPerPatchOutputComponents: u32,
    pub maxTessellationEvaluationPerVertexInputComponents: u32,
    pub maxTessellationEvaluationPerVertexOutputComponents: u32,
    pub maxGeometryShaderInvocations: u32,
    pub maxGeometryInputComponents: u32,
    pub maxGeometryOutputComponents: u32,
    pub maxFragmentInputComponents: u32,
    pub maxFragmentOutputAttachments: u32,
    pub maxFragmentDualSrcAttachments: u32,
    pub maxFragmentCombinedOutputResources: u32,
    pub maxComputeSharedMemorySize: u32,
    pub maxComputeWorkGroupCount: [u32; 3],
    pub maxComputeWorkGroupInvocations: u32,
    pub maxComputeWorkGroupSize: [u32; 3],
    pub subPixelPrecisionBits: u32,
    pub subTexelPrecisionBits: u32,
    pub mipmapPrecisionBits: u32,
    pub maxDrawIndexedIndexValue: u32,
    pub maxDrawIndirectCount: u32,
    pub maxSamplerLodBias: f32,
    pub maxSamplerAnisotropy: f32,
    pub maxViewports: u32,
    pub maxViewportDimensions: [u32; 2],
    pub viewportBoundsRange: [f32; 2],
    pub viewportSubPixelBits: u32,
    pub minMemoryMapAlignment: usize,
    pub minTexelBufferOffsetAlignment: VkDeviceSize,
    pub minUniformBufferOffsetAlignment: VkDeviceSize,
    pub minStorageBufferOffsetAlignment: VkDeviceSize,
    pub minTexelOffset: i32,
    pub maxTexelOffset: u32,
    pub minTexelGatherOffset: i32,
    pub maxTexelGatherOffset: u32,
    pub minInterpolationOffset: f32,
    pub maxInterpolationOffset: f32,
    pub subPixelInterpolationOffsetBits: u32,
    pub maxFramebufferWidth: u32,
    pub maxFramebufferHeight: u32,
    pub maxFramebufferLayers: u32,
    pub framebufferColorSampleCounts: VkSampleCountFlags,
    pub framebufferDepthSampleCounts: VkSampleCountFlags,
    pub framebufferStencilSampleCounts: VkSampleCountFlags,
    pub framebufferNoAttachmentsSampleCounts: VkSampleCountFlags,
    pub maxColorAttachments: u32,
    pub sampledImageColorSampleCounts: VkSampleCountFlags,
    pub sampledImageIntegerSampleCounts: VkSampleCountFlags,
    pub sampledImageDepthSampleCounts: VkSampleCountFlags,
    pub sampledImageStencilSampleCounts: VkSampleCountFlags,
    pub storageImageSampleCounts: VkSampleCountFlags,
    pub maxSampleMaskWords: u32,
    pub timestampComputeAndGraphics: VkBool32,
    pub timestampPeriod: f32,
    pub maxClipDistances: u32,
    pub maxCullDistances: u32,
    pub maxCombinedClipAndCullDistances: u32,
    pub discreteQueuePriorities: u32,
    pub pointSizeRange: [f32; 2],
    pub lineWidthRange: [f32; 2],
    pub pointSizeGranularity: f32,
    pub lineWidthGranularity: f32,
    pub strictLines: VkBool32,
    pub standardSampleLocations: VkBool32,
    pub optimalBufferCopyOffsetAlignment: VkDeviceSize,
    pub optimalBufferCopyRowPitchAlignment: VkDeviceSize,
    pub nonCoherentAtomSize: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceSparseProperties {
    pub residencyStandard2DBlockShape: VkBool32,
    pub residencyStandard2DMultisampleBlockShape: VkBool32,
    pub residencyStandard3DBlockShape: VkBool32,
    pub residencyAlignedMipSize: VkBool32,
    pub residencyNonResidentStrict: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VkPhysicalDeviceProperties {
    pub apiVersion: u32,
    pub driverVersion: u32,
    pub vendorID: u32,
    pub deviceID: u32,
    pub deviceType: u32,
    pub deviceName: [c_char; 256],
    pub pipelineCacheUUID: [u8; VK_UUID_SIZE],
    pub limits: VkPhysicalDeviceLimits,
    pub sparseProperties: VkPhysicalDeviceSparseProperties,
}
impl Default for VkPhysicalDeviceProperties {
    fn default() -> Self {
        unsafe { zeroed() }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDeviceProperties2 {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub properties: VkPhysicalDeviceProperties,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPhysicalDevicePushDescriptorPropertiesKHR {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub maxPushDescriptors: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkConformanceVersion {
    pub major: u8,
    pub minor: u8,
    pub subminor: u8,
    pub patch: u8,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VkPhysicalDeviceDriverProperties {
    pub sType: VkStructureType,
    pub pNext: *mut c_void,
    pub driverID: u32,
    pub driverName: [c_char; 256],
    pub driverInfo: [c_char; 256],
    pub conformanceVersion: VkConformanceVersion,
}
impl Default for VkPhysicalDeviceDriverProperties {
    fn default() -> Self {
        unsafe { zeroed() }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VkExtensionProperties {
    pub extensionName: [c_char; 256],
    pub specVersion: u32,
}
impl Default for VkExtensionProperties {
    fn default() -> Self {
        unsafe { zeroed() }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VkLayerProperties {
    pub layerName: [c_char; 256],
    pub specVersion: u32,
    pub implementationVersion: u32,
    pub description: [c_char; 256],
}
impl Default for VkLayerProperties {
    fn default() -> Self {
        unsafe { zeroed() }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkQueueFamilyProperties {
    pub queueFlags: VkFlags,
    pub queueCount: u32,
    pub timestampValidBits: u32,
    pub minImageTransferGranularity: VkExtent3D,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDeviceQueueCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub queueFamilyIndex: u32,
    pub queueCount: u32,
    pub pQueuePriorities: *const f32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDeviceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub queueCreateInfoCount: u32,
    pub pQueueCreateInfos: *const VkDeviceQueueCreateInfo,
    pub enabledLayerCount: u32,
    pub ppEnabledLayerNames: *const *const c_char,
    pub enabledExtensionCount: u32,
    pub ppEnabledExtensionNames: *const *const c_char,
    pub pEnabledFeatures: *const VkPhysicalDeviceFeatures,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkCommandPoolCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkCommandPoolCreateFlags,
    pub queueFamilyIndex: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkCommandBufferAllocateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub commandPool: VkCommandPool,
    pub level: u32,
    pub commandBufferCount: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkCommandBufferBeginInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkCommandBufferUsageFlags,
    pub pInheritanceInfo: *const c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkFenceCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFenceCreateFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSemaphoreCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkQueryPoolCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub queryType: VkQueryType,
    pub queryCount: u32,
    pub pipelineStatistics: VkFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSubmitInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub waitSemaphoreCount: u32,
    pub pWaitSemaphores: *const VkSemaphore,
    pub pWaitDstStageMask: *const VkPipelineStageFlags,
    pub commandBufferCount: u32,
    pub pCommandBuffers: *const VkCommandBuffer,
    pub signalSemaphoreCount: u32,
    pub pSignalSemaphores: *const VkSemaphore,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPresentInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub waitSemaphoreCount: u32,
    pub pWaitSemaphores: *const VkSemaphore,
    pub swapchainCount: u32,
    pub pSwapchains: *const VkSwapchainKHR,
    pub pImageIndices: *const u32,
    pub pResults: *mut VkResult,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSwapchainCreateInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub surface: VkSurfaceKHR,
    pub minImageCount: u32,
    pub imageFormat: VkFormat,
    pub imageColorSpace: VkColorSpaceKHR,
    pub imageExtent: VkExtent2D,
    pub imageArrayLayers: u32,
    pub imageUsage: VkImageUsageFlags,
    pub imageSharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
    pub preTransform: VkSurfaceTransformFlagBitsKHR,
    pub compositeAlpha: VkCompositeAlphaFlagBitsKHR,
    pub presentMode: VkPresentModeKHR,
    pub clipped: VkBool32,
    pub oldSwapchain: VkSwapchainKHR,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSwapchainPresentModesCreateInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub presentModeCount: u32,
    pub pPresentModes: *const VkPresentModeKHR,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkReleaseSwapchainImagesInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub swapchain: VkSwapchainKHR,
    pub imageIndexCount: u32,
    pub pImageIndices: *const u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSurfaceFormatKHR {
    pub format: VkFormat,
    pub colorSpace: VkColorSpaceKHR,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSurfaceCapabilitiesKHR {
    pub minImageCount: u32,
    pub maxImageCount: u32,
    pub currentExtent: VkExtent2D,
    pub minImageExtent: VkExtent2D,
    pub maxImageExtent: VkExtent2D,
    pub maxImageArrayLayers: u32,
    pub supportedTransforms: VkFlags,
    pub currentTransform: VkSurfaceTransformFlagBitsKHR,
    pub supportedCompositeAlpha: VkFlags,
    pub supportedUsageFlags: VkImageUsageFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkFormatProperties {
    pub linearTilingFeatures: VkFormatFeatureFlags,
    pub optimalTilingFeatures: VkFormatFeatureFlags,
    pub bufferFeatures: VkFormatFeatureFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBufferCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub size: VkDeviceSize,
    pub usage: VkBufferUsageFlags,
    pub sharingMode: VkSharingMode,
    pub queueFamilyIndexCount: u32,
    pub pQueueFamilyIndices: *const u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
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
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageViewCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub image: VkImage,
    pub viewType: u32,
    pub format: VkFormat,
    pub components: VkComponentMapping,
    pub subresourceRange: VkImageSubresourceRange,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBufferImageCopy {
    pub bufferOffset: VkDeviceSize,
    pub bufferRowLength: u32,
    pub bufferImageHeight: u32,
    pub imageSubresource: VkImageSubresourceLayers,
    pub imageOffset: VkOffset3D,
    pub imageExtent: VkExtent3D,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBufferCopy {
    pub srcOffset: VkDeviceSize,
    pub dstOffset: VkDeviceSize,
    pub size: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageCopy {
    pub srcSubresource: VkImageSubresourceLayers,
    pub srcOffset: VkOffset3D,
    pub dstSubresource: VkImageSubresourceLayers,
    pub dstOffset: VkOffset3D,
    pub extent: VkExtent3D,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageBlit {
    pub srcSubresource: VkImageSubresourceLayers,
    pub srcOffsets: [VkOffset3D; 2],
    pub dstSubresource: VkImageSubresourceLayers,
    pub dstOffsets: [VkOffset3D; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBufferMemoryBarrier {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
    pub srcQueueFamilyIndex: u32,
    pub dstQueueFamilyIndex: u32,
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkImageMemoryBarrier {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
    pub oldLayout: VkImageLayout,
    pub newLayout: VkImageLayout,
    pub srcQueueFamilyIndex: u32,
    pub dstQueueFamilyIndex: u32,
    pub image: VkImage,
    pub subresourceRange: VkImageSubresourceRange,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkMemoryRequirements {
    pub size: VkDeviceSize,
    pub alignment: VkDeviceSize,
    pub memoryTypeBits: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkMemoryAllocateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub allocationSize: VkDeviceSize,
    pub memoryTypeIndex: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorSetLayoutBinding {
    pub binding: u32,
    pub descriptorType: VkDescriptorType,
    pub descriptorCount: u32,
    pub stageFlags: VkShaderStageFlags,
    pub pImmutableSamplers: *const VkSampler,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorSetLayoutCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub bindingCount: u32,
    pub pBindings: *const VkDescriptorSetLayoutBinding,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorPoolSize {
    pub ty: VkDescriptorType,
    pub descriptorCount: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorPoolCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkDescriptorPoolCreateFlags,
    pub maxSets: u32,
    pub poolSizeCount: u32,
    pub pPoolSizes: *const VkDescriptorPoolSize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorSetAllocateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub descriptorPool: VkDescriptorPool,
    pub descriptorSetCount: u32,
    pub pSetLayouts: *const VkDescriptorSetLayout,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorImageInfo {
    pub sampler: VkSampler,
    pub imageView: VkImageView,
    pub imageLayout: VkImageLayout,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDescriptorBufferInfo {
    pub buffer: VkBuffer,
    pub offset: VkDeviceSize,
    pub range: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkWriteDescriptorSet {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub dstSet: VkDescriptorSet,
    pub dstBinding: u32,
    pub dstArrayElement: u32,
    pub descriptorCount: u32,
    pub descriptorType: VkDescriptorType,
    pub pImageInfo: *const VkDescriptorImageInfo,
    pub pBufferInfo: *const VkDescriptorBufferInfo,
    pub pTexelBufferView: *const VkBufferView,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkCopyDescriptorSet {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub srcSet: VkDescriptorSet,
    pub srcBinding: u32,
    pub srcArrayElement: u32,
    pub dstSet: VkDescriptorSet,
    pub dstBinding: u32,
    pub dstArrayElement: u32,
    pub descriptorCount: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPushConstantRange {
    pub stageFlags: VkShaderStageFlags,
    pub offset: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineLayoutCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub setLayoutCount: u32,
    pub pSetLayouts: *const VkDescriptorSetLayout,
    pub pushConstantRangeCount: u32,
    pub pPushConstantRanges: *const VkPushConstantRange,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineShaderStageCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub stage: VkShaderStageFlagBits,
    pub module: VkShaderModule,
    pub pName: *const c_char,
    pub pSpecializationInfo: *const VkSpecializationInfo,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSpecializationMapEntry {
    pub constantID: u32,
    pub offset: u32,
    pub size: usize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSpecializationInfo {
    pub mapEntryCount: u32,
    pub pMapEntries: *const VkSpecializationMapEntry,
    pub dataSize: usize,
    pub pData: *const c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkVertexInputBindingDescription {
    pub binding: u32,
    pub stride: u32,
    pub inputRate: VkVertexInputRate,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkVertexInputAttributeDescription {
    pub location: u32,
    pub binding: u32,
    pub format: VkFormat,
    pub offset: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineVertexInputStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub vertexBindingDescriptionCount: u32,
    pub pVertexBindingDescriptions: *const VkVertexInputBindingDescription,
    pub vertexAttributeDescriptionCount: u32,
    pub pVertexAttributeDescriptions: *const VkVertexInputAttributeDescription,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineInputAssemblyStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub topology: VkPrimitiveTopology,
    pub primitiveRestartEnable: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineTessellationStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub patchControlPoints: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineViewportStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub viewportCount: u32,
    pub pViewports: *const VkViewport,
    pub scissorCount: u32,
    pub pScissors: *const VkRect2D,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineRasterizationStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub depthClampEnable: VkBool32,
    pub rasterizerDiscardEnable: VkBool32,
    pub polygonMode: VkPolygonMode,
    pub cullMode: VkCullModeFlags,
    pub frontFace: VkFrontFace,
    pub depthBiasEnable: VkBool32,
    pub depthBiasConstantFactor: f32,
    pub depthBiasClamp: f32,
    pub depthBiasSlopeFactor: f32,
    pub lineWidth: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineMultisampleStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub rasterizationSamples: VkSampleCountFlagBits,
    pub sampleShadingEnable: VkBool32,
    pub minSampleShading: f32,
    pub pSampleMask: *const VkFlags,
    pub alphaToCoverageEnable: VkBool32,
    pub alphaToOneEnable: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineDepthStencilStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub depthTestEnable: VkBool32,
    pub depthWriteEnable: VkBool32,
    pub depthCompareOp: VkCompareOp,
    pub depthBoundsTestEnable: VkBool32,
    pub stencilTestEnable: VkBool32,
    pub front: VkStencilOpState,
    pub back: VkStencilOpState,
    pub minDepthBounds: f32,
    pub maxDepthBounds: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineColorBlendAttachmentState {
    pub blendEnable: VkBool32,
    pub srcColorBlendFactor: VkBlendFactor,
    pub dstColorBlendFactor: VkBlendFactor,
    pub colorBlendOp: VkBlendOp,
    pub srcAlphaBlendFactor: VkBlendFactor,
    pub dstAlphaBlendFactor: VkBlendFactor,
    pub alphaBlendOp: VkBlendOp,
    pub colorWriteMask: VkColorComponentFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineColorBlendStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub logicOpEnable: VkBool32,
    pub logicOp: u32,
    pub attachmentCount: u32,
    pub pAttachments: *const VkPipelineColorBlendAttachmentState,
    pub blendConstants: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineDynamicStateCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub dynamicStateCount: u32,
    pub pDynamicStates: *const VkDynamicState,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineRasterizationProvokingVertexStateCreateInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub provokingVertexMode: VkProvokingVertexModeEXT,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineRasterizationLineStateCreateInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub lineRasterizationMode: VkLineRasterizationModeEXT,
    pub stippledLineEnable: VkBool32,
    pub lineStippleFactor: u32,
    pub lineStipplePattern: u16,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkGraphicsPipelineCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineCreateFlags,
    pub stageCount: u32,
    pub pStages: *const VkPipelineShaderStageCreateInfo,
    pub pVertexInputState: *const VkPipelineVertexInputStateCreateInfo,
    pub pInputAssemblyState: *const VkPipelineInputAssemblyStateCreateInfo,
    pub pTessellationState: *const VkPipelineTessellationStateCreateInfo,
    pub pViewportState: *const VkPipelineViewportStateCreateInfo,
    pub pRasterizationState: *const VkPipelineRasterizationStateCreateInfo,
    pub pMultisampleState: *const VkPipelineMultisampleStateCreateInfo,
    pub pDepthStencilState: *const VkPipelineDepthStencilStateCreateInfo,
    pub pColorBlendState: *const VkPipelineColorBlendStateCreateInfo,
    pub pDynamicState: *const VkPipelineDynamicStateCreateInfo,
    pub layout: VkPipelineLayout,
    pub renderPass: VkRenderPass,
    pub subpass: u32,
    pub basePipelineHandle: VkPipeline,
    pub basePipelineIndex: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkComputePipelineCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkPipelineCreateFlags,
    pub stage: VkPipelineShaderStageCreateInfo,
    pub layout: VkPipelineLayout,
    pub basePipelineHandle: VkPipeline,
    pub basePipelineIndex: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkAttachmentDescription {
    pub flags: VkFlags,
    pub format: VkFormat,
    pub samples: VkSampleCountFlagBits,
    pub loadOp: VkAttachmentLoadOp,
    pub storeOp: VkAttachmentStoreOp,
    pub stencilLoadOp: VkAttachmentLoadOp,
    pub stencilStoreOp: VkAttachmentStoreOp,
    pub initialLayout: VkImageLayout,
    pub finalLayout: VkImageLayout,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkAttachmentReference {
    pub attachment: u32,
    pub layout: VkImageLayout,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSubpassDescription {
    pub flags: VkSubpassDescriptionFlags,
    pub pipelineBindPoint: VkPipelineBindPoint,
    pub inputAttachmentCount: u32,
    pub pInputAttachments: *const VkAttachmentReference,
    pub colorAttachmentCount: u32,
    pub pColorAttachments: *const VkAttachmentReference,
    pub pResolveAttachments: *const VkAttachmentReference,
    pub pDepthStencilAttachment: *const VkAttachmentReference,
    pub preserveAttachmentCount: u32,
    pub pPreserveAttachments: *const u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSubpassDependency {
    pub srcSubpass: u32,
    pub dstSubpass: u32,
    pub srcStageMask: VkPipelineStageFlags,
    pub dstStageMask: VkPipelineStageFlags,
    pub srcAccessMask: VkAccessFlags,
    pub dstAccessMask: VkAccessFlags,
    pub dependencyFlags: VkDependencyFlags,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkRenderPassCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub attachmentCount: u32,
    pub pAttachments: *const VkAttachmentDescription,
    pub subpassCount: u32,
    pub pSubpasses: *const VkSubpassDescription,
    pub dependencyCount: u32,
    pub pDependencies: *const VkSubpassDependency,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkFramebufferCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub renderPass: VkRenderPass,
    pub attachmentCount: u32,
    pub pAttachments: *const VkImageView,
    pub width: u32,
    pub height: u32,
    pub layers: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkRenderPassBeginInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub renderPass: VkRenderPass,
    pub framebuffer: VkFramebuffer,
    pub renderArea: VkRect2D,
    pub clearValueCount: u32,
    pub pClearValues: *const VkClearValue,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSamplerCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub magFilter: VkFilter,
    pub minFilter: VkFilter,
    pub mipmapMode: VkSamplerMipmapMode,
    pub addressModeU: VkSamplerAddressMode,
    pub addressModeV: VkSamplerAddressMode,
    pub addressModeW: VkSamplerAddressMode,
    pub mipLodBias: f32,
    pub anisotropyEnable: VkBool32,
    pub maxAnisotropy: f32,
    pub compareEnable: VkBool32,
    pub compareOp: VkCompareOp,
    pub minLod: f32,
    pub maxLod: f32,
    pub borderColor: u32,
    pub unnormalizedCoordinates: VkBool32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkShaderModuleCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub codeSize: usize,
    pub pCode: *const u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkPipelineCacheCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub initialDataSize: usize,
    pub pInitialData: *const c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkBufferViewCreateInfo {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub buffer: VkBuffer,
    pub format: VkFormat,
    pub offset: VkDeviceSize,
    pub range: VkDeviceSize,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkWin32SurfaceCreateInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub hinstance: *mut c_void,
    pub hwnd: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkXlibSurfaceCreateInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub dpy: *mut c_void,
    pub window: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkWaylandSurfaceCreateInfoKHR {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub display: *mut c_void,
    pub surface: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkMetalSurfaceCreateInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub pLayer: *const c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSurfaceFullScreenExclusiveInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub fullScreenExclusive: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkSurfaceFullScreenExclusiveWin32InfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub hmonitor: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDebugUtilsMessengerCreateInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub messageSeverity: VkFlags,
    pub messageType: VkFlags,
    pub pfnUserCallback: PFN_vkDebugUtilsMessengerCallbackEXT,
    pub pUserData: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDebugUtilsLabelEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub pLabelName: *const c_char,
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDebugUtilsObjectNameInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub objectType: VkObjectType,
    pub objectHandle: u64,
    pub pObjectName: *const c_char,
}

pub type PFN_vkDebugUtilsMessengerCallbackEXT = Option<
    unsafe extern "C" fn(
        messageSeverity: VkFlags,
        messageType: VkFlags,
        pCallbackData: *const VkDebugUtilsMessengerCallbackDataEXT,
        pUserData: *mut c_void,
    ) -> VkBool32,
>;

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkDebugUtilsMessengerCallbackDataEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub flags: VkFlags,
    pub pMessageIdName: *const c_char,
    pub messageIdNumber: i32,
    pub pMessage: *const c_char,
    pub queueLabelCount: u32,
    pub pQueueLabels: *const VkDebugUtilsLabelEXT,
    pub cmdBufLabelCount: u32,
    pub pCmdBufLabels: *const VkDebugUtilsLabelEXT,
    pub objectCount: u32,
    pub pObjects: *const VkDebugUtilsObjectNameInfoEXT,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkCalibratedTimestampInfoEXT {
    pub sType: VkStructureType,
    pub pNext: *const c_void,
    pub timeDomain: VkTimeDomainEXT,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkClearAttachment {
    pub aspectMask: VkImageAspectFlags,
    pub colorAttachment: u32,
    pub clearValue: VkClearValue,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VkClearRect {
    pub rect: VkRect2D,
    pub baseArrayLayer: u32,
    pub layerCount: u32,
}

// =====================================================================
//  Section 4.  VMA subset.
// =====================================================================

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VmaAllocationCreateInfo {
    pub flags: u32,
    pub usage: u32,
    pub requiredFlags: VkMemoryPropertyFlags,
    pub preferredFlags: VkMemoryPropertyFlags,
    pub memoryTypeBits: u32,
    pub pool: *mut c_void,
    pub pUserData: *mut c_void,
    pub priority: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VmaAllocationInfo {
    pub memoryType: u32,
    pub deviceMemory: VkDeviceMemory,
    pub offset: VkDeviceSize,
    pub size: VkDeviceSize,
    pub pMappedData: *mut c_void,
    pub pUserData: *mut c_void,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub struct VmaAllocatorCreateInfo {
    pub flags: u32,
    pub physicalDevice: VkPhysicalDevice,
    pub device: VkDevice,
    pub preferredLargeHeapBlockSize: VkDeviceSize,
    pub pAllocationCallbacks: *const c_void,
    pub pDeviceMemoryCallbacks: *const c_void,
    pub pHeapSizeLimit: *const VkDeviceSize,
    pub pVulkanFunctions: *const c_void,
    pub instance: VkInstance,
    pub vulkanApiVersion: u32,
    pub pTypeExternalMemoryHandleTypes: *const c_void,
}

// =====================================================================
//  Section 5.  Stubs for the Vulkan entry points we touch.
// =====================================================================
//
// These match the Vulkan signatures and panic on call.  Replace the bodies
// with calls into `ash` (or the platform loader of your choice) when wiring
// this up to a real Vulkan implementation.

macro_rules! vk_stub {
    ($name:ident($($p:ident: $t:ty),* $(,)?) -> $r:ty) => {
        pub unsafe extern "C" fn $name($($p: $t),*) -> $r {
            panic!(concat!("vk_stub: ", stringify!($name), " called"));
        }
    };
}

vk_stub!(vkCreateInstance(pCreateInfo: *const VkInstanceCreateInfo, pAllocator: *const c_void, pInstance: *mut VkInstance) -> VkResult);
vk_stub!(vkDestroyInstance(instance: VkInstance, pAllocator: *const c_void) -> ());
vk_stub!(vkEnumerateInstanceExtensionProperties(pLayerName: *const c_char, pPropertyCount: *mut u32, pProperties: *mut VkExtensionProperties) -> VkResult);
vk_stub!(vkEnumerateInstanceLayerProperties(pPropertyCount: *mut u32, pProperties: *mut VkLayerProperties) -> VkResult);
vk_stub!(vkEnumeratePhysicalDevices(instance: VkInstance, pPhysicalDeviceCount: *mut u32, pPhysicalDevices: *mut VkPhysicalDevice) -> VkResult);
vk_stub!(vkEnumerateDeviceExtensionProperties(physicalDevice: VkPhysicalDevice, pLayerName: *const c_char, pPropertyCount: *mut u32, pProperties: *mut VkExtensionProperties) -> VkResult);
vk_stub!(vkGetInstanceProcAddr(instance: VkInstance, pName: *const c_char) -> *const c_void);
vk_stub!(vkGetDeviceProcAddr(device: VkDevice, pName: *const c_char) -> *const c_void);
vk_stub!(vkGetPhysicalDeviceProperties(physicalDevice: VkPhysicalDevice, pProperties: *mut VkPhysicalDeviceProperties) -> ());
vk_stub!(vkGetPhysicalDeviceProperties2(physicalDevice: VkPhysicalDevice, pProperties: *mut VkPhysicalDeviceProperties2) -> ());
vk_stub!(vkGetPhysicalDeviceFeatures(physicalDevice: VkPhysicalDevice, pFeatures: *mut VkPhysicalDeviceFeatures) -> ());
vk_stub!(vkGetPhysicalDeviceFeatures2(physicalDevice: VkPhysicalDevice, pFeatures: *mut VkPhysicalDeviceFeatures2) -> ());
vk_stub!(vkGetPhysicalDeviceMemoryProperties(physicalDevice: VkPhysicalDevice, pMemoryProperties: *mut VkPhysicalDeviceMemoryProperties) -> ());
vk_stub!(vkGetPhysicalDeviceFormatProperties(physicalDevice: VkPhysicalDevice, format: VkFormat, pFormatProperties: *mut VkFormatProperties) -> ());
vk_stub!(vkGetPhysicalDeviceQueueFamilyProperties(physicalDevice: VkPhysicalDevice, pQueueFamilyPropertyCount: *mut u32, pQueueFamilyProperties: *mut VkQueueFamilyProperties) -> ());
vk_stub!(vkGetPhysicalDeviceSurfaceSupportKHR(physicalDevice: VkPhysicalDevice, queueFamilyIndex: u32, surface: VkSurfaceKHR, pSupported: *mut VkBool32) -> VkResult);
vk_stub!(vkGetPhysicalDeviceSurfaceFormatsKHR(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pSurfaceFormatCount: *mut u32, pSurfaceFormats: *mut VkSurfaceFormatKHR) -> VkResult);
vk_stub!(vkGetPhysicalDeviceSurfacePresentModesKHR(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pPresentModeCount: *mut u32, pPresentModes: *mut VkPresentModeKHR) -> VkResult);
vk_stub!(vkGetPhysicalDeviceSurfaceCapabilitiesKHR(physicalDevice: VkPhysicalDevice, surface: VkSurfaceKHR, pSurfaceCapabilities: *mut VkSurfaceCapabilitiesKHR) -> VkResult);
vk_stub!(vkGetPhysicalDeviceCalibrateableTimeDomainsEXT(physicalDevice: VkPhysicalDevice, pTimeDomainCount: *mut u32, pTimeDomains: *mut VkTimeDomainEXT) -> VkResult);

vk_stub!(vkCreateDevice(physicalDevice: VkPhysicalDevice, pCreateInfo: *const VkDeviceCreateInfo, pAllocator: *const c_void, pDevice: *mut VkDevice) -> VkResult);
vk_stub!(vkDestroyDevice(device: VkDevice, pAllocator: *const c_void) -> ());
vk_stub!(vkGetDeviceQueue(device: VkDevice, queueFamilyIndex: u32, queueIndex: u32, pQueue: *mut VkQueue) -> ());
vk_stub!(vkDeviceWaitIdle(device: VkDevice) -> VkResult);
vk_stub!(vkQueueSubmit(queue: VkQueue, submitCount: u32, pSubmits: *const VkSubmitInfo, fence: VkFence) -> VkResult);
vk_stub!(vkQueueWaitIdle(queue: VkQueue) -> VkResult);
vk_stub!(vkQueuePresentKHR(queue: VkQueue, pPresentInfo: *const VkPresentInfoKHR) -> VkResult);

vk_stub!(vkCreateCommandPool(device: VkDevice, pCreateInfo: *const VkCommandPoolCreateInfo, pAllocator: *const c_void, pCommandPool: *mut VkCommandPool) -> VkResult);
vk_stub!(vkDestroyCommandPool(device: VkDevice, commandPool: VkCommandPool, pAllocator: *const c_void) -> ());
vk_stub!(vkResetCommandPool(device: VkDevice, commandPool: VkCommandPool, flags: VkFlags) -> VkResult);
vk_stub!(vkAllocateCommandBuffers(device: VkDevice, pAllocateInfo: *const VkCommandBufferAllocateInfo, pCommandBuffers: *mut VkCommandBuffer) -> VkResult);
vk_stub!(vkFreeCommandBuffers(device: VkDevice, commandPool: VkCommandPool, commandBufferCount: u32, pCommandBuffers: *const VkCommandBuffer) -> ());
vk_stub!(vkBeginCommandBuffer(commandBuffer: VkCommandBuffer, pBeginInfo: *const VkCommandBufferBeginInfo) -> VkResult);
vk_stub!(vkEndCommandBuffer(commandBuffer: VkCommandBuffer) -> VkResult);
vk_stub!(vkResetCommandBuffer(commandBuffer: VkCommandBuffer, flags: VkFlags) -> VkResult);

vk_stub!(vkCreateFence(device: VkDevice, pCreateInfo: *const VkFenceCreateInfo, pAllocator: *const c_void, pFence: *mut VkFence) -> VkResult);
vk_stub!(vkDestroyFence(device: VkDevice, fence: VkFence, pAllocator: *const c_void) -> ());
vk_stub!(vkResetFences(device: VkDevice, fenceCount: u32, pFences: *const VkFence) -> VkResult);
vk_stub!(vkGetFenceStatus(device: VkDevice, fence: VkFence) -> VkResult);
vk_stub!(vkWaitForFences(device: VkDevice, fenceCount: u32, pFences: *const VkFence, waitAll: VkBool32, timeout: u64) -> VkResult);

vk_stub!(vkCreateSemaphore(device: VkDevice, pCreateInfo: *const VkSemaphoreCreateInfo, pAllocator: *const c_void, pSemaphore: *mut VkSemaphore) -> VkResult);
vk_stub!(vkDestroySemaphore(device: VkDevice, semaphore: VkSemaphore, pAllocator: *const c_void) -> ());

vk_stub!(vkCreateEvent(device: VkDevice, pCreateInfo: *const c_void, pAllocator: *const c_void, pEvent: *mut c_void) -> VkResult);
vk_stub!(vkDestroyEvent(device: VkDevice, event: *mut c_void, pAllocator: *const c_void) -> ());
vk_stub!(vkSetEvent(device: VkDevice, event: *mut c_void) -> VkResult);
vk_stub!(vkResetEvent(device: VkDevice, event: *mut c_void) -> VkResult);

vk_stub!(vkCreateQueryPool(device: VkDevice, pCreateInfo: *const VkQueryPoolCreateInfo, pAllocator: *const c_void, pQueryPool: *mut VkQueryPool) -> VkResult);
vk_stub!(vkDestroyQueryPool(device: VkDevice, queryPool: VkQueryPool, pAllocator: *const c_void) -> ());
vk_stub!(vkGetQueryPoolResults(device: VkDevice, queryPool: VkQueryPool, firstQuery: u32, queryCount: u32, dataSize: usize, pData: *mut c_void, stride: VkDeviceSize, flags: VkFlags) -> VkResult);

vk_stub!(vkCreateBuffer(device: VkDevice, pCreateInfo: *const VkBufferCreateInfo, pAllocator: *const c_void, pBuffer: *mut VkBuffer) -> VkResult);
vk_stub!(vkDestroyBuffer(device: VkDevice, buffer: VkBuffer, pAllocator: *const c_void) -> ());
vk_stub!(vkBindBufferMemory(device: VkDevice, buffer: VkBuffer, memory: VkDeviceMemory, memoryOffset: VkDeviceSize) -> VkResult);
vk_stub!(vkCreateBufferView(device: VkDevice, pCreateInfo: *const VkBufferViewCreateInfo, pAllocator: *const c_void, pView: *mut VkBufferView) -> VkResult);
vk_stub!(vkDestroyBufferView(device: VkDevice, bufferView: VkBufferView, pAllocator: *const c_void) -> ());

vk_stub!(vkCreateImage(device: VkDevice, pCreateInfo: *const VkImageCreateInfo, pAllocator: *const c_void, pImage: *mut VkImage) -> VkResult);
vk_stub!(vkDestroyImage(device: VkDevice, image: VkImage, pAllocator: *const c_void) -> ());
vk_stub!(vkBindImageMemory(device: VkDevice, image: VkImage, memory: VkDeviceMemory, memoryOffset: VkDeviceSize) -> VkResult);
vk_stub!(vkGetImageSubresourceLayout(device: VkDevice, image: VkImage, pSubresource: *const VkImageSubresource, pLayout: *mut c_void) -> ());
vk_stub!(vkCreateImageView(device: VkDevice, pCreateInfo: *const VkImageViewCreateInfo, pAllocator: *const c_void, pView: *mut VkImageView) -> VkResult);
vk_stub!(vkDestroyImageView(device: VkDevice, imageView: VkImageView, pAllocator: *const c_void) -> ());

vk_stub!(vkAllocateMemory(device: VkDevice, pAllocateInfo: *const VkMemoryAllocateInfo, pAllocator: *const c_void, pMemory: *mut VkDeviceMemory) -> VkResult);
vk_stub!(vkFreeMemory(device: VkDevice, memory: VkDeviceMemory, pAllocator: *const c_void) -> ());
vk_stub!(vkMapMemory(device: VkDevice, memory: VkDeviceMemory, offset: VkDeviceSize, size: VkDeviceSize, flags: VkFlags, ppData: *mut *mut c_void) -> VkResult);
vk_stub!(vkUnmapMemory(device: VkDevice, memory: VkDeviceMemory) -> ());
vk_stub!(vkFlushMappedMemoryRanges(device: VkDevice, memoryRangeCount: u32, pMemoryRanges: *const c_void) -> VkResult);
vk_stub!(vkInvalidateMappedMemoryRanges(device: VkDevice, memoryRangeCount: u32, pMemoryRanges: *const c_void) -> VkResult);

vk_stub!(vkCreateShaderModule(device: VkDevice, pCreateInfo: *const VkShaderModuleCreateInfo, pAllocator: *const c_void, pShaderModule: *mut VkShaderModule) -> VkResult);
vk_stub!(vkDestroyShaderModule(device: VkDevice, shaderModule: VkShaderModule, pAllocator: *const c_void) -> ());

vk_stub!(vkCreatePipelineLayout(device: VkDevice, pCreateInfo: *const VkPipelineLayoutCreateInfo, pAllocator: *const c_void, pPipelineLayout: *mut VkPipelineLayout) -> VkResult);
vk_stub!(vkDestroyPipelineLayout(device: VkDevice, pipelineLayout: VkPipelineLayout, pAllocator: *const c_void) -> ());
vk_stub!(vkCreatePipelineCache(device: VkDevice, pCreateInfo: *const VkPipelineCacheCreateInfo, pAllocator: *const c_void, pPipelineCache: *mut VkPipelineCache) -> VkResult);
vk_stub!(vkDestroyPipelineCache(device: VkDevice, pipelineCache: VkPipelineCache, pAllocator: *const c_void) -> ());
vk_stub!(vkGetPipelineCacheData(device: VkDevice, pipelineCache: VkPipelineCache, pDataSize: *mut usize, pData: *mut c_void) -> VkResult);
vk_stub!(vkMergePipelineCaches(device: VkDevice, dstCache: VkPipelineCache, srcCacheCount: u32, pSrcCaches: *const VkPipelineCache) -> VkResult);
vk_stub!(vkCreateGraphicsPipelines(device: VkDevice, pipelineCache: VkPipelineCache, createInfoCount: u32, pCreateInfos: *const VkGraphicsPipelineCreateInfo, pAllocator: *const c_void, pPipelines: *mut VkPipeline) -> VkResult);
vk_stub!(vkCreateComputePipelines(device: VkDevice, pipelineCache: VkPipelineCache, createInfoCount: u32, pCreateInfos: *const VkComputePipelineCreateInfo, pAllocator: *const c_void, pPipelines: *mut VkPipeline) -> VkResult);
vk_stub!(vkDestroyPipeline(device: VkDevice, pipeline: VkPipeline, pAllocator: *const c_void) -> ());

vk_stub!(vkCreateSampler(device: VkDevice, pCreateInfo: *const VkSamplerCreateInfo, pAllocator: *const c_void, pSampler: *mut VkSampler) -> VkResult);
vk_stub!(vkDestroySampler(device: VkDevice, sampler: VkSampler, pAllocator: *const c_void) -> ());

vk_stub!(vkCreateDescriptorSetLayout(device: VkDevice, pCreateInfo: *const VkDescriptorSetLayoutCreateInfo, pAllocator: *const c_void, pSetLayout: *mut VkDescriptorSetLayout) -> VkResult);
vk_stub!(vkDestroyDescriptorSetLayout(device: VkDevice, descriptorSetLayout: VkDescriptorSetLayout, pAllocator: *const c_void) -> ());
vk_stub!(vkCreateDescriptorPool(device: VkDevice, pCreateInfo: *const VkDescriptorPoolCreateInfo, pAllocator: *const c_void, pDescriptorPool: *mut VkDescriptorPool) -> VkResult);
vk_stub!(vkDestroyDescriptorPool(device: VkDevice, descriptorPool: VkDescriptorPool, pAllocator: *const c_void) -> ());
vk_stub!(vkAllocateDescriptorSets(device: VkDevice, pAllocateInfo: *const VkDescriptorSetAllocateInfo, pDescriptorSets: *mut VkDescriptorSet) -> VkResult);
vk_stub!(vkFreeDescriptorSets(device: VkDevice, descriptorPool: VkDescriptorPool, descriptorSetCount: u32, pDescriptorSets: *const VkDescriptorSet) -> VkResult);
vk_stub!(vkUpdateDescriptorSets(device: VkDevice, descriptorWriteCount: u32, pDescriptorWrites: *const VkWriteDescriptorSet, descriptorCopyCount: u32, pDescriptorCopies: *const VkCopyDescriptorSet) -> ());

vk_stub!(vkCreateRenderPass(device: VkDevice, pCreateInfo: *const VkRenderPassCreateInfo, pAllocator: *const c_void, pRenderPass: *mut VkRenderPass) -> VkResult);
vk_stub!(vkDestroyRenderPass(device: VkDevice, renderPass: VkRenderPass, pAllocator: *const c_void) -> ());
vk_stub!(vkCreateFramebuffer(device: VkDevice, pCreateInfo: *const VkFramebufferCreateInfo, pAllocator: *const c_void, pFramebuffer: *mut VkFramebuffer) -> VkResult);
vk_stub!(vkDestroyFramebuffer(device: VkDevice, framebuffer: VkFramebuffer, pAllocator: *const c_void) -> ());

vk_stub!(vkCreateSwapchainKHR(device: VkDevice, pCreateInfo: *const VkSwapchainCreateInfoKHR, pAllocator: *const c_void, pSwapchain: *mut VkSwapchainKHR) -> VkResult);
vk_stub!(vkDestroySwapchainKHR(device: VkDevice, swapchain: VkSwapchainKHR, pAllocator: *const c_void) -> ());
vk_stub!(vkGetSwapchainImagesKHR(device: VkDevice, swapchain: VkSwapchainKHR, pSwapchainImageCount: *mut u32, pSwapchainImages: *mut VkImage) -> VkResult);
vk_stub!(vkAcquireNextImageKHR(device: VkDevice, swapchain: VkSwapchainKHR, timeout: u64, semaphore: VkSemaphore, fence: VkFence, pImageIndex: *mut u32) -> VkResult);
vk_stub!(vkReleaseSwapchainImagesKHR(device: VkDevice, pReleaseInfo: *const VkReleaseSwapchainImagesInfoKHR) -> VkResult);
vk_stub!(vkReleaseSwapchainImagesEXT(device: VkDevice, pReleaseInfo: *const VkReleaseSwapchainImagesInfoKHR) -> VkResult);

vk_stub!(vkCreateWin32SurfaceKHR(instance: VkInstance, pCreateInfo: *const VkWin32SurfaceCreateInfoKHR, pAllocator: *const c_void, pSurface: *mut VkSurfaceKHR) -> VkResult);
vk_stub!(vkCreateXlibSurfaceKHR(instance: VkInstance, pCreateInfo: *const VkXlibSurfaceCreateInfoKHR, pAllocator: *const c_void, pSurface: *mut VkSurfaceKHR) -> VkResult);
vk_stub!(vkCreateWaylandSurfaceKHR(instance: VkInstance, pCreateInfo: *const VkWaylandSurfaceCreateInfoKHR, pAllocator: *const c_void, pSurface: *mut VkSurfaceKHR) -> VkResult);
vk_stub!(vkCreateMetalSurfaceEXT(instance: VkInstance, pCreateInfo: *const VkMetalSurfaceCreateInfoEXT, pAllocator: *const c_void, pSurface: *mut VkSurfaceKHR) -> VkResult);
vk_stub!(vkDestroySurfaceKHR(instance: VkInstance, surface: VkSurfaceKHR, pAllocator: *const c_void) -> ());

vk_stub!(vkCmdBindPipeline(commandBuffer: VkCommandBuffer, pipelineBindPoint: VkPipelineBindPoint, pipeline: VkPipeline) -> ());
vk_stub!(vkCmdSetViewport(commandBuffer: VkCommandBuffer, firstViewport: u32, viewportCount: u32, pViewports: *const VkViewport) -> ());
vk_stub!(vkCmdSetScissor(commandBuffer: VkCommandBuffer, firstScissor: u32, scissorCount: u32, pScissors: *const VkRect2D) -> ());
vk_stub!(vkCmdSetLineWidth(commandBuffer: VkCommandBuffer, lineWidth: f32) -> ());
vk_stub!(vkCmdSetBlendConstants(commandBuffer: VkCommandBuffer, blendConstants: *const f32) -> ());
vk_stub!(vkCmdSetDepthBias(commandBuffer: VkCommandBuffer, depthBiasConstantFactor: f32, depthBiasClamp: f32, depthBiasSlopeFactor: f32) -> ());
vk_stub!(vkCmdSetStencilCompareMask(commandBuffer: VkCommandBuffer, faceMask: VkFlags, compareMask: u32) -> ());
vk_stub!(vkCmdSetStencilWriteMask(commandBuffer: VkCommandBuffer, faceMask: VkFlags, writeMask: u32) -> ());
vk_stub!(vkCmdSetStencilReference(commandBuffer: VkCommandBuffer, faceMask: VkFlags, reference: u32) -> ());

vk_stub!(vkCmdBindDescriptorSets(commandBuffer: VkCommandBuffer, pipelineBindPoint: VkPipelineBindPoint, layout: VkPipelineLayout, firstSet: u32, descriptorSetCount: u32, pDescriptorSets: *const VkDescriptorSet, dynamicOffsetCount: u32, pDynamicOffsets: *const u32) -> ());
vk_stub!(vkCmdBindVertexBuffers(commandBuffer: VkCommandBuffer, firstBinding: u32, bindingCount: u32, pBuffers: *const VkBuffer, pOffsets: *const VkDeviceSize) -> ());
vk_stub!(vkCmdBindIndexBuffer(commandBuffer: VkCommandBuffer, buffer: VkBuffer, offset: VkDeviceSize, indexType: VkIndexType) -> ());
vk_stub!(vkCmdPushConstants(commandBuffer: VkCommandBuffer, layout: VkPipelineLayout, stageFlags: VkShaderStageFlags, offset: u32, size: u32, pValues: *const c_void) -> ());
vk_stub!(vkCmdPushDescriptorSetKHR(commandBuffer: VkCommandBuffer, pipelineBindPoint: VkPipelineBindPoint, layout: VkPipelineLayout, set: u32, descriptorWriteCount: u32, pDescriptorWrites: *const VkWriteDescriptorSet) -> ());

vk_stub!(vkCmdDraw(commandBuffer: VkCommandBuffer, vertexCount: u32, instanceCount: u32, firstVertex: u32, firstInstance: u32) -> ());
vk_stub!(vkCmdDrawIndexed(commandBuffer: VkCommandBuffer, indexCount: u32, instanceCount: u32, firstIndex: u32, vertexOffset: i32, firstInstance: u32) -> ());
vk_stub!(vkCmdDrawIndirect(commandBuffer: VkCommandBuffer, buffer: VkBuffer, offset: VkDeviceSize, drawCount: u32, stride: u32) -> ());
vk_stub!(vkCmdDrawIndexedIndirect(commandBuffer: VkCommandBuffer, buffer: VkBuffer, offset: VkDeviceSize, drawCount: u32, stride: u32) -> ());
vk_stub!(vkCmdDispatch(commandBuffer: VkCommandBuffer, groupCountX: u32, groupCountY: u32, groupCountZ: u32) -> ());

vk_stub!(vkCmdCopyBuffer(commandBuffer: VkCommandBuffer, srcBuffer: VkBuffer, dstBuffer: VkBuffer, regionCount: u32, pRegions: *const VkBufferCopy) -> ());
vk_stub!(vkCmdCopyImage(commandBuffer: VkCommandBuffer, srcImage: VkImage, srcImageLayout: VkImageLayout, dstImage: VkImage, dstImageLayout: VkImageLayout, regionCount: u32, pRegions: *const VkImageCopy) -> ());
vk_stub!(vkCmdBlitImage(commandBuffer: VkCommandBuffer, srcImage: VkImage, srcImageLayout: VkImageLayout, dstImage: VkImage, dstImageLayout: VkImageLayout, regionCount: u32, pRegions: *const VkImageBlit, filter: VkFilter) -> ());
vk_stub!(vkCmdCopyBufferToImage(commandBuffer: VkCommandBuffer, srcBuffer: VkBuffer, dstImage: VkImage, dstImageLayout: VkImageLayout, regionCount: u32, pRegions: *const VkBufferImageCopy) -> ());
vk_stub!(vkCmdCopyImageToBuffer(commandBuffer: VkCommandBuffer, srcImage: VkImage, srcImageLayout: VkImageLayout, dstBuffer: VkBuffer, regionCount: u32, pRegions: *const VkBufferImageCopy) -> ());
vk_stub!(vkCmdUpdateBuffer(commandBuffer: VkCommandBuffer, dstBuffer: VkBuffer, dstOffset: VkDeviceSize, dataSize: VkDeviceSize, pData: *const u32) -> ());
vk_stub!(vkCmdFillBuffer(commandBuffer: VkCommandBuffer, dstBuffer: VkBuffer, dstOffset: VkDeviceSize, size: VkDeviceSize, data: u32) -> ());
vk_stub!(vkCmdClearColorImage(commandBuffer: VkCommandBuffer, image: VkImage, imageLayout: VkImageLayout, pColor: *const VkClearColorValue, rangeCount: u32, pRanges: *const VkImageSubresourceRange) -> ());
vk_stub!(vkCmdClearDepthStencilImage(commandBuffer: VkCommandBuffer, image: VkImage, imageLayout: VkImageLayout, pDepthStencil: *const VkClearDepthStencilValue, rangeCount: u32, pRanges: *const VkImageSubresourceRange) -> ());
vk_stub!(vkCmdClearAttachments(commandBuffer: VkCommandBuffer, attachmentCount: u32, pAttachments: *const VkClearAttachment, rectCount: u32, pRects: *const VkClearRect) -> ());
vk_stub!(vkCmdPipelineBarrier(commandBuffer: VkCommandBuffer, srcStageMask: VkPipelineStageFlags, dstStageMask: VkPipelineStageFlags, dependencyFlags: VkDependencyFlags, memoryBarrierCount: u32, pMemoryBarriers: *const c_void, bufferMemoryBarrierCount: u32, pBufferMemoryBarriers: *const VkBufferMemoryBarrier, imageMemoryBarrierCount: u32, pImageMemoryBarriers: *const VkImageMemoryBarrier) -> ());
vk_stub!(vkCmdBeginRenderPass(commandBuffer: VkCommandBuffer, pRenderPassBegin: *const VkRenderPassBeginInfo, contents: VkSubpassContents) -> ());
vk_stub!(vkCmdEndRenderPass(commandBuffer: VkCommandBuffer) -> ());
vk_stub!(vkCmdResetQueryPool(commandBuffer: VkCommandBuffer, queryPool: VkQueryPool, firstQuery: u32, queryCount: u32) -> ());
vk_stub!(vkCmdWriteTimestamp(commandBuffer: VkCommandBuffer, pipelineStage: VkPipelineStageFlags, queryPool: VkQueryPool, query: u32) -> ());
vk_stub!(vkCmdBeginDebugUtilsLabelEXT(commandBuffer: VkCommandBuffer, pLabelInfo: *const VkDebugUtilsLabelEXT) -> ());
vk_stub!(vkCmdEndDebugUtilsLabelEXT(commandBuffer: VkCommandBuffer) -> ());
vk_stub!(vkCmdInsertDebugUtilsLabelEXT(commandBuffer: VkCommandBuffer, pLabelInfo: *const VkDebugUtilsLabelEXT) -> ());

// Debug utils
vk_stub!(vkCreateDebugUtilsMessengerEXT(instance: VkInstance, pCreateInfo: *const VkDebugUtilsMessengerCreateInfoEXT, pAllocator: *const c_void, pMessenger: *mut VkDebugUtilsMessengerEXT) -> VkResult);
vk_stub!(vkDestroyDebugUtilsMessengerEXT(instance: VkInstance, messenger: VkDebugUtilsMessengerEXT, pAllocator: *const c_void) -> ());
vk_stub!(vkSetDebugUtilsObjectNameEXT(device: VkDevice, pNameInfo: *const VkDebugUtilsObjectNameInfoEXT) -> VkResult);
vk_stub!(vkSubmitDebugUtilsMessageEXT(instance: VkInstance, messageSeverity: VkFlags, messageTypes: VkFlags, pCallbackData: *const VkDebugUtilsMessengerCallbackDataEXT) -> ());
vk_stub!(vkGetCalibratedTimestampsEXT(device: VkDevice, timestampCount: u32, pTimestampInfos: *const VkCalibratedTimestampInfoEXT, pTimestamps: *mut u64, pMaxDeviation: *mut u64) -> VkResult);

// =====================================================================
//  Section 6.  VMA stub layer.
//
// The PCSX2 build links against the AMD Vulkan Memory Allocator.  We
// declare a minimal subset of its API so the rest of the translation unit
// can use the names directly.  The bodies are stubs and panic on call.
// =====================================================================

pub unsafe extern "C" fn vmaCreateAllocator(
    pCreateInfo: *const VmaAllocatorCreateInfo,
    pAllocator: *mut VmaAllocator,
) -> VkResult {
    panic!("vmaCreateAllocator stub");
}
pub unsafe extern "C" fn vmaDestroyAllocator(_allocator: VmaAllocator) {
    panic!("vmaDestroyAllocator stub");
}
pub unsafe extern "C" fn vmaCreateImage(
    allocator: VmaAllocator,
    pImageCreateInfo: *const VkImageCreateInfo,
    pAllocationCreateInfo: *const VmaAllocationCreateInfo,
    pImage: *mut VkImage,
    pAllocation: *mut VmaAllocation,
    pAllocationInfo: *mut VmaAllocationInfo,
) -> VkResult {
    panic!("vmaCreateImage stub");
}
pub unsafe extern "C" fn vmaDestroyImage(allocator: VmaAllocator, image: VkImage, allocation: VmaAllocation) {
    panic!("vmaDestroyImage stub");
}
pub unsafe extern "C" fn vmaCreateBuffer(
    allocator: VmaAllocator,
    pBufferCreateInfo: *const VkBufferCreateInfo,
    pAllocationCreateInfo: *const VmaAllocationCreateInfo,
    pBuffer: *mut VkBuffer,
    pAllocation: *mut VmaAllocation,
    pAllocationInfo: *mut VmaAllocationInfo,
) -> VkResult {
    panic!("vmaCreateBuffer stub");
}
pub unsafe extern "C" fn vmaDestroyBuffer(allocator: VmaAllocator, buffer: VkBuffer, allocation: VmaAllocation) {
    panic!("vmaDestroyBuffer stub");
}
pub unsafe extern "C" fn vmaMapMemory(allocator: VmaAllocator, allocation: VmaAllocation, ppData: *mut *mut c_void) -> VkResult {
    panic!("vmaMapMemory stub");
}
pub unsafe extern "C" fn vmaUnmapMemory(allocator: VmaAllocator, allocation: VmaAllocation) {
    panic!("vmaUnmapMemory stub");
}
pub unsafe extern "C" fn vmaFlushAllocation(allocator: VmaAllocator, allocation: VmaAllocation, offset: VkDeviceSize, size: VkDeviceSize) {
    panic!("vmaFlushAllocation stub");
}
pub unsafe extern "C" fn vmaInvalidateAllocation(allocator: VmaAllocator, allocation: VmaAllocation, offset: VkDeviceSize, size: VkDeviceSize) {
    panic!("vmaInvalidateAllocation stub");
}
pub unsafe extern "C" fn vmaSetCurrentFrameIndex(allocator: VmaAllocator, frameIndex: u32) {
    panic!("vmaSetCurrentFrameIndex stub");
}

// =====================================================================
//  Section 7.  Logging and small utilities from the C++ codebase.
// =====================================================================
//
// The C++ side uses Console.Error / Console.Warning / DevCon.WriteLn
// strings.  We define a tiny `Console` namespace stub here that captures
// messages to a `Vec<String>` so tests can inspect them.  In the host
// process this would be replaced with the real logging layer.

#[derive(Default, Debug, Clone)]
pub struct Console {
    pub messages: Vec<String>,
}

impl Console {
    pub const fn new() -> Self {
        Self { messages: Vec::new() }
    }
    pub fn error(&mut self, fmt: fmt::Arguments) {
        self.messages.push(format!("ERROR: {}", fmt));
    }
    pub fn warning(&mut self, fmt: fmt::Arguments) {
        self.messages.push(format!("WARN: {}", fmt));
    }
    pub fn write(&mut self, fmt: fmt::Arguments) {
        self.messages.push(format!("{}", fmt));
    }
}

pub mod macros {
    use super::{Console, VkResult};
    pub fn log_vulkan_error(console: &mut Console, func: &'static str, res: VkResult, msg: &str) {
        console.error(format_args!("({}) {} ({}: VK_RESULT)", func, msg, res));
    }
    pub fn px_assert(cond: bool, msg: &str) {
        if !cond {
            panic!("PX_ASSERT: {}", msg);
        }
    }
    pub fn px_assert_rel(cond: bool, msg: &str) {
        if !cond {
            panic!("PX_ASSERT_REL: {}", msg);
        }
    }
}

pub fn align_up(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

pub fn bit_equal<T: Copy + PartialEq>(a: &[T], b: &[T]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b.iter()).all(|(x, y)| x == y)
}

pub fn vk_make_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 22) | (minor << 12) | patch
}

pub fn vk_version_major(v: u32) -> u32 { v >> 22 }
pub fn vk_version_minor(v: u32) -> u32 { (v >> 12) & 0x3ff }
pub fn vk_version_patch(v: u32) -> u32 { v & 0xfff }
pub fn vk_api_version_major(v: u32) -> u32 { vk_version_major(v) }
pub fn vk_api_version_minor(v: u32) -> u32 { vk_version_minor(v) }
pub fn vk_api_version_patch(v: u32) -> u32 { vk_version_patch(v) }
pub fn vk_api_version_variant(v: u32) -> u32 { v >> 29 }
// =====================================================================
//  Section 8.  GS-vector, GS-reg, and related support types.
// =====================================================================
//
// The GS renderer uses 128-bit SIMD types throughout.  We use Rust's
// array-of-floats representation rather than a real SIMD type so the
// translation is portable; the real code uses SSE/AVX intrinsics.

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct GSVector4 {
    pub v: [f32; 4],
}

impl GSVector4 {
    pub const ZERO: Self = Self { v: [0.0; 4] };
    pub const ONE: Self = Self { v: [1.0; 4] };
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self { Self { v: [x, y, z, w] } }
    pub const fn splat(x: f32) -> Self { Self { v: [x; 4] } }
    pub const fn load32(p: *const f32) -> Self { unsafe { Self { v: [*p, *p.offset(1), *p.offset(2), *p.offset(3)] } } }
    pub fn store(&self, p: *mut f32) { unsafe { for i in 0..4 { *p.add(i) = self.v[i]; } } }
    pub fn unorm8(c: u32) -> Self {
        let b0 = ((c) & 0xFF) as f32;
        let b1 = ((c >> 8) & 0xFF) as f32;
        let b2 = ((c >> 16) & 0xFF) as f32;
        let b3 = ((c >> 24) & 0xFF) as f32;
        Self { v: [b0 / 255.0, b1 / 255.0, b2 / 255.0, b3 / 255.0] }
    }
    pub fn eq(self, other: Self) -> bool { self.v == other.v }
    pub fn xyxy(self) -> Self { Self { v: [self.v[0], self.v[1], self.v[0], self.v[1]] } } // simplified
    pub fn zwzw(self) -> Self { Self { v: [self.v[2], self.v[3], self.v[2], self.v[3]] } }
    pub fn mask(self) -> i32 {
        let mut r = 0;
        if self.v[0] < 0.0 { r |= 1; }
        if self.v[1] < 0.0 { r |= 2; }
        if self.v[2] < 0.0 { r |= 4; }
        if self.v[3] < 0.0 { r |= 8; }
        r
    }
    pub fn cxpr(x: f32, y: f32, z: f32, w: f32) -> Self { Self::new(x, y, z, w) }
}

impl std::ops::Add for GSVector4 { type Output = Self; fn add(self, o: Self) -> Self { let mut r = [0.0; 4]; for i in 0..4 { r[i] = self.v[i] + o.v[i]; } Self { v: r } } }
impl std::ops::Sub for GSVector4 { type Output = Self; fn sub(self, o: Self) -> Self { let mut r = [0.0; 4]; for i in 0..4 { r[i] = self.v[i] - o.v[i]; } Self { v: r } } }
impl std::ops::Mul for GSVector4 { type Output = Self; fn mul(self, o: Self) -> Self { let mut r = [0.0; 4]; for i in 0..4 { r[i] = self.v[i] * o.v[i]; } Self { v: r } } }
impl std::ops::Div for GSVector4 { type Output = Self; fn div(self, o: Self) -> Self { let mut r = [0.0; 4]; for i in 0..4 { r[i] = self.v[i] / o.v[i]; } Self { v: r } } }

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct GSVector2i {
    pub x: i32,
    pub y: i32,
}

impl GSVector2i {
    pub const fn new(x: i32, y: i32) -> Self { Self { x, y } }
    pub const ZERO: Self = Self { x: 0, y: 0 };
}

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct GSVector2 {
    pub x: f32,
    pub y: f32,
}

impl GSVector2 {
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
}

#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct GSVector4i {
    pub x: i32, pub y: i32, pub z: i32, pub w: i32,
}

impl GSVector4i {
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0, w: 0 };
    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self { Self { x, y, z, w } }
    pub fn width(&self) -> i32 { self.z - self.x }
    pub fn height(&self) -> i32 { self.w - self.y }
    pub fn left(&self) -> i32 { self.x }
    pub fn top(&self) -> i32 { self.y }
    pub fn right(&self) -> i32 { self.z }
    pub fn bottom(&self) -> i32 { self.w }
    pub fn eq(&self, other: Self) -> bool { *self == other }
    pub fn rempty(&self) -> bool { self.z <= self.x || self.w <= self.y }
    pub fn rintersect(self, other: Self) -> Self {
        Self::new(self.x.max(other.x), self.y.max(other.y), self.z.min(other.z), self.w.min(other.w))
    }
    pub fn runion(self, other: Self) -> Self {
        Self::new(self.x.min(other.x), self.y.min(other.y), self.z.max(other.z), self.w.max(other.w))
    }
    pub fn max_i32(self, other: Self) -> Self {
        Self::new(self.x.max(other.x), self.y.max(other.y), self.z.max(other.z), self.w.max(other.w))
    }
    pub fn loadh(size: GSVector2i) -> Self { Self::new(0, 0, size.x, size.y) }
}

impl From<GSVector4> for GSVector4i { fn from(_: GSVector4) -> Self { Self::ZERO } }

// =====================================================================
//  Section 9.  PS2 register subset that appears in the Vulkan code.
// =====================================================================

#[derive(Copy, Clone, Default, Debug)]
pub struct GSRegPMODE { pub EN1: u32, pub EN2: u32, pub SLRGB: u32, pub MMOD: u32, pub SLBG: u32 }
#[derive(Copy, Clone, Default, Debug)]
pub struct GSRegEXTBUF { pub EMODA: u32, pub EMODC: u32, pub FBIN: u32 }

#[derive(Copy, Clone, Default, Debug)]
pub struct GSCascadeDisabled {}

// =====================================================================
//  Section 10.  GSDevice trait and supporting enums.
// =====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderAPI { Vulkan, D3D12, OpenGL, Metal, Dummy }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSDeviceType { HardwareRenderer, SoftwareRenderer, Count }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentResult { OK, FrameSkipped, DeviceLost }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VsyncMode { Disabled, FIFO, Mailbox }

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Filter { #[default] Nearest, Biln }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SetDATM { ZERO, ONE, KEEP, _Count }
pub const fn SetDATMShader(datm: SetDATM) -> u32 { datm as u32 }

pub struct GSVSyncMode;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DebugMessageCategory { Cache, Reg, Debug, Message, Performance, Count }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShaderConvert { COPY, RGBA_TO_8I, RGB5A1_TO_8I, CLUT_4, CLUT_8, YUV, DOWNSAMPLE_COPY, COLCLIP_INIT, COLCLIP_RESOLVE, _Count }
pub const fn ShaderConvert_COUNT() -> u32 { ShaderConvert::_Count as u32 }
pub const fn ShaderConvertSelector_COUNT() -> u32 { 32 }
pub struct ShaderConvertSelector;
impl ShaderConvertSelector {
    pub const NUM_TOTAL_SHADERS: u32 = 32;
    pub fn Get(_i: u32) -> Self { ShaderConvertSelector }
    pub fn Index(&self) -> usize { 0 }
    pub fn Mask(&self) -> u32 { 0xf }
    pub fn SetMask(&self, m: u32) -> Self { ShaderConvertSelector }
    pub fn DATMConvertShader(&self) -> bool { false }
    pub fn DepthOutput(&self) -> bool { false }
    pub fn OutputFormat(&self) -> GSTextureFormat { GSTextureFormat::Color }
    pub fn SupportsBilinear(&self) -> bool { false }
    pub fn Biln(&self) -> bool { false }
    pub fn StencilOutput(&self) -> bool { false }
    pub fn IntegerOutputBpp(&self) -> u32 { 0 }
    pub fn DepthOutput2(&self) -> bool { false }
    pub fn Float32Input(&self) -> bool { false }
    pub fn Float32Output(&self) -> bool { false }
    pub fn Name(&self) -> &'static str { "Convert" }
    pub fn EntryPoint(&self) -> &'static str { "ps_main" }
    pub fn Shader(&self) -> ShaderConvert { ShaderConvert::COPY }
}

pub type GSTextureFormat = GSTextureFormatEnum;
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSTextureFormatEnum { Color, Invalid }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentShader { COPY, SCANLINE, _Count }
impl PresentShader { pub const Count: usize = 2; }
pub fn ShaderEntryPoint(_s: PresentShader) -> &'static str { "ps_main" }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShaderInterlace { _Count }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ZTST { ZTST_NEVER, ZTST_ALWAYS, ZTST_GEQUAL, ZTST_GREATER }
pub const NUM_INTERLACE_SHADERS: usize = 4;
pub const NUM_CAS_CONSTANTS: usize = 5;

#[derive(Copy, Clone, Default, Debug)]
pub struct MultiStretchRect {
    pub src: *mut GSTexture,
    pub src_rect: GSVector4,
    pub dst_rect: GSVector4,
    pub wmask: GSVector4i,
    pub filter: Filter,
}

#[derive(Copy, Clone, Default, Debug)]
pub struct InterlaceConstantBuffer { pub deinterlace: u32, pub field: u32, pub counter: u32, pub fps: f32 }

#[derive(Copy, Clone, Default, Debug)]
pub struct DisplayConstantBuffer { pub source_rect: GSVector4, pub target_rect: GSVector4, pub time: f32 }
impl DisplayConstantBuffer {
    pub fn SetSource(&mut self, r: GSVector4, _size: GSVector2i) { self.source_rect = r; }
    pub fn SetTarget(&mut self, r: GSVector4, _size: GSVector2i) { self.target_rect = r; }
    pub fn SetTime(&mut self, t: f32) { self.time = t; }
}

#[derive(Copy, Clone, Default, Debug)]
pub struct FeatureSupport {
    pub framebuffer_fetch: bool,
    pub texture_barrier: bool,
    pub multidraw_fb_copy: bool,
    pub broken_point_sampler: bool,
    pub primitive_id: bool,
    pub prefer_new_textures: bool,
    pub provoking_vertex_last: bool,
    pub vs_expand: bool,
    pub stencil_buffer: bool,
    pub test_and_sample_depth: bool,
    pub point_expand: bool,
    pub line_expand: bool,
    pub depth_feedback: bool,
    pub aa1: bool,
    pub dxt_textures: bool,
    pub bptc_textures: bool,
    pub rov: bool,
    pub cas_sharpening: bool,
}
impl FeatureSupport {
    pub fn feedback_loops(&self) -> bool { self.texture_barrier }
}

// =====================================================================
//  Section 11.  GSTexture base + GSTextureVK
// =====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSTextureType { Texture, RenderTarget, DepthStencil, RWTexture }
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSTextureState { Dirty, Cleared, Invalidated }
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSDownloadTextureState { None, Idle, Pending }

pub struct GSTexture {
    pub m_type: GSTextureType,
    pub m_format: GSTextureFormatEnum,
    pub m_size: GSVector2i,
    pub m_mipmap_levels: i32,
    pub m_state: GSTextureState,
    pub m_clear_color: u32,
    pub m_clear_depth: f32,
    pub m_layout: i32,
    pub m_use_fence_counter: u64,
    pub m_map_area: GSVector4i,
    pub m_map_level: i32,
    pub m_needs_mipmaps_generated: bool,
    pub m_framebuffers: Vec<FramebuffersTuple>,
    pub m_debug_name: String,
}

impl Default for GSTexture {
    fn default() -> Self {
        Self {
            m_type: GSTextureType::Texture,
            m_format: GSTextureFormatEnum::Invalid,
            m_size: GSVector2i::ZERO,
            m_mipmap_levels: 1,
            m_state: GSTextureState::Dirty,
            m_clear_color: 0,
            m_clear_depth: 0.0,
            m_layout: 0,
            m_use_fence_counter: 0,
            m_map_area: GSVector4i::ZERO,
            m_map_level: 0,
            m_needs_mipmaps_generated: false,
            m_framebuffers: Vec::new(),
            m_debug_name: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FramebuffersTuple { pub other_tex: *mut GSTextureVK, pub fb: VkFramebuffer, pub feedback_color: bool, pub feedback_depth: bool }

impl GSTexture {
    pub fn IsRenderTarget(&self) -> bool { matches!(self.m_type, GSTextureType::RenderTarget) }
    pub fn IsDepthStencil(&self) -> bool { matches!(self.m_type, GSTextureType::DepthStencil) }
    pub fn IsRenderTargetOrDepthStencil(&self) -> bool { self.IsRenderTarget() || self.IsDepthStencil() }
    pub fn IsCompressedFormat(&self) -> bool { false }
    pub fn GetState(&self) -> GSTextureState { self.m_state }
    pub fn SetState(&mut self, s: GSTextureState) { self.m_state = s; }
    pub fn GetClearColor(&self) -> u32 { self.m_clear_color }
    pub fn SetClearColor(&mut self, c: u32) { self.m_clear_color = c; }
    pub fn GetClearDepth(&self) -> f32 { self.m_clear_depth }
    pub fn GetSize(&self) -> GSVector2i { self.m_size }
    pub fn GetWidth(&self) -> i32 { self.m_size.x }
    pub fn GetHeight(&self) -> i32 { self.m_size.y }
    pub fn GetRect(&self) -> GSVector4i { GSVector4i::new(0, 0, self.m_size.x, self.m_size.y) }
    pub fn GetMipmapLevels(&self) -> i32 { self.m_mipmap_levels }
    pub fn GetType(&self) -> GSTextureType { self.m_type }
    pub fn GetFormat(&self) -> GSTextureFormatEnum { self.m_format }
    pub fn GetClearForFormat(&self) -> GSVector4 { GSVector4::unorm8(self.m_clear_color) }
    pub fn GetCompressionBlockSize(&self) -> u32 { 4 }
    pub fn GetCompressedBlockSize(&self) -> u32 { 4 }
    pub fn CalcUploadRowLengthFromPitch(_format: GSTextureFormatEnum, pitch: u32) -> u32 { pitch / 4 }
    pub fn CalcUploadPitch(w: i32) -> u32 { (w.max(0) * 4) as u32 }
    pub fn CalcUploadSize(h: i32, p: u32) -> u32 { h.max(0) as u32 * p }
    pub fn GetNativeHandle(&self) -> *mut c_void { self as *const _ as *mut c_void }
}

pub struct GSMap<'a> { pub bits: *mut u8, pub pitch: u32, _phantom: std::marker::PhantomData<&'a mut u8> }
pub struct GSOffset { pub x: i32, pub y: i32 }

pub struct GSTextureVK {
    pub base: GSTexture,
    pub m_image: VkImage,
    pub m_allocation: VmaAllocation,
    pub m_view: VkImageView,
    pub m_vk_format: VkFormat,
}

impl GSTextureVK {
    pub fn Create(_ty: GSTextureType, _format: GSTextureFormatEnum, _w: i32, _h: i32, _levels: i32) -> Option<Box<Self>> { None }
    pub fn Adopt(_image: VkImage, _ty: GSTextureType, _format: GSTextureFormatEnum, _w: i32, _h: i32, _levels: i32, _vk: VkFormat) -> Option<Box<Self>> { None }
    pub fn Destroy(&mut self, _defer: bool) {}
    pub fn GetImage(&self) -> VkImage { self.m_image }
    pub fn GetView(&self) -> VkImageView { self.m_view }
    pub fn GetVkFormat(&self) -> VkFormat { self.m_vk_format }
    pub fn GetLayout(&self) -> GSTextureVKLayout { self.base.m_layout as i32 }
    pub fn GetVkLayout(&self) -> VkImageLayout { 0 }
    pub fn GetLinkedFramebuffer(&mut self, _depth: *mut GSTextureVK, _fbc: bool, _fbd: bool) -> VkFramebuffer { ptr::null_mut() }
    pub fn GetFramebuffer(&mut self, _feedback_loop: bool) -> VkFramebuffer { ptr::null_mut() }
    pub fn SetUseFenceCounter(&mut self, c: u64) { self.base.m_use_fence_counter = c; }
    pub fn TransitionToLayout(&mut self, _layout: GSTextureVKLayout) {}
    pub fn TransitionToLayout2(&mut self, _cmd: VkCommandBuffer, _layout: GSTextureVKLayout) {}
    pub fn TransitionSubresourcesToLayout(&mut self, _cmd: VkCommandBuffer, _start: i32, _count: i32, _old: GSTextureVKLayout, _new: GSTextureVKLayout) {}
    pub fn OverrideImageLayout(&mut self, l: GSTextureVKLayout) { self.base.m_layout = l as i32; }
    pub fn CommitClear(&mut self) {}
    pub fn CommitClear2(&mut self, _cmd: VkCommandBuffer) {}
    pub fn UpdateFromBuffer(&mut self, _cmd: VkCommandBuffer, _level: i32, _x: u32, _y: u32, _w: u32, _h: u32, _bh: u32, _rl: u32, _buf: VkBuffer, _off: u32) {}
    pub fn Update(&mut self, _r: GSVector4i, _data: *const c_void, _pitch: i32, _layer: i32) -> bool { false }
    pub fn Map(&mut self, _m: &mut GSMap, _r: *const GSVector4i, _layer: i32) -> bool { false }
    pub fn Unmap(&mut self) {}
    pub fn GenerateMipmap(&mut self) {}
    pub fn AllocateUploadStagingBuffer(&self, _data: *const c_void, _p: u32, _up: u32, _h: u32) -> VkBuffer { ptr::null_mut() }
    pub fn CopyTextureDataForUpload(&self, _dst: *mut c_void, _src: *const c_void, _pitch: u32, _up: u32, _h: u32) {}
}

pub type GSTextureVKLayout = i32;
pub mod GSTextureVK_Layout {
    use super::GSTextureVKLayout;
    pub const Undefined: GSTextureVKLayout = 0;
    pub const Preinitialized: GSTextureVKLayout = 1;
    pub const ColorAttachment: GSTextureVKLayout = 2;
    pub const DepthStencilAttachment: GSTextureVKLayout = 3;
    pub const ShaderReadOnly: GSTextureVKLayout = 4;
    pub const ClearDst: GSTextureVKLayout = 5;
    pub const TransferSrc: GSTextureVKLayout = 6;
    pub const TransferDst: GSTextureVKLayout = 7;
    pub const TransferSelf: GSTextureVKLayout = 8;
    pub const PresentSrc: GSTextureVKLayout = 9;
    pub const FeedbackLoop: GSTextureVKLayout = 10;
    pub const ReadWriteImage: GSTextureVKLayout = 11;
    pub const ComputeReadWriteImage: GSTextureVKLayout = 12;
    pub const General: GSTextureVKLayout = 13;
    pub const Count: GSTextureVKLayout = 14;
}

pub fn CreateNullFramebuffer() -> VkFramebuffer { ptr::null_mut() }

pub struct GSDownloadTexture {
    pub m_width: u32, pub m_height: u32, pub m_format: GSTextureFormatEnum,
    pub m_current_pitch: u32, pub m_buffer_size: u32, pub m_map_pointer: *const u8,
    pub m_copy_fence_counter: u64, pub m_needs_cache_invalidate: bool, pub m_needs_flush: bool,
}
impl GSDownloadTexture {
    pub fn GetBufferSize(w: u32, h: u32, _fmt: GSTextureFormatEnum, _align: u32) -> u32 { w * h * 4 }
    pub fn GetTransferPitch(w: u32, _align: u32) -> u32 { w * 4 }
    pub fn GetTransferSize(_r: GSVector4i, off: &mut u32, sz: &mut u32, rows: &mut u32) { *off = 0; *sz = 0; *rows = 0; }
}

pub struct GSDownloadTextureVK {
    pub base: GSDownloadTexture,
    pub m_allocation: VmaAllocation,
    pub m_buffer: VkBuffer,
}
impl GSDownloadTextureVK {
    pub fn Create(_w: u32, _h: u32, _fmt: GSTextureFormatEnum) -> Option<Box<Self>> { None }
    pub fn CopyFromTexture(&mut self, _drc: GSVector4i, _stex: *mut GSTexture, _src: GSVector4i, _lvl: u32, _utp: bool) {}
    pub fn Map(&mut self, _read_rc: GSVector4i) -> bool { false }
    pub fn Unmap(&mut self) {}
    pub fn Flush(&mut self) {}
    pub fn SetDebugName(&mut self, _name: &str) {}
}

// =====================================================================
//  Section 12.  GSHWDrawConfig (stub struct - the real one is huge).
// =====================================================================

pub mod GSHWDrawConfig {
    use super::*;
    #[derive(Clone, Default, Debug, PartialEq, Eq)]
    pub struct PSSelector { pub key_hi: u64, pub key_lo: u64 }
    impl PSSelector {
        pub fn Hash(_: &PSSelector) -> u32 { 0 }
        pub fn HasColorROV(&self) -> bool { false }
        pub fn HasDepthROV(&self) -> bool { false }
        pub fn HasColorOutput(&self) -> bool { true }
        pub fn HasDepthROVWrite(&self) -> bool { false }
    }
    #[derive(Clone, Default, Debug, PartialEq, Eq)]
    pub struct VSSelector { pub key: u32, pub tme: i32, pub fst: i32, pub iip: i32, pub point_size: i32, pub expand: u8 }
    #[derive(Clone, Default, Debug, PartialEq, Eq)]
    pub struct BlendState { pub enable: bool, pub constant_enable: bool, pub constant: u8, pub src_factor: u32, pub dst_factor: u32, pub op: u32, pub src_factor_alpha: u32, pub dst_factor_alpha: u32, pub key: u32 }
    #[derive(Clone, Default, Debug, PartialEq, Eq)]
    pub struct DepthStencilSelector { pub date: bool, pub date_one: bool, pub ztst: u32, pub zwe: bool, pub key: u32 }
    #[derive(Clone, Default, Debug, PartialEq, Eq)]
    pub struct ColorMaskSelector { pub wrgba: u32, pub key: u32 }
    pub struct VSConstantBuffer;
    pub struct VSPushConstants { pub base_vertex: u32, pub base_index: u32 }
    pub struct SamplerSelector { pub key: u32, pub tau: bool, pub tav: bool, pub lodclamp: bool }
    impl SamplerSelector {
        pub fn Point() -> Self { Self { key: 0, tau: false, tav: false, lodclamp: false } }
        pub fn Linear() -> Self { Self { key: 1, tau: false, tav: false, lodclamp: false } }
        pub fn IsMagFilterLinear(&self) -> bool { false }
        pub fn IsMinFilterLinear(&self) -> bool { false }
        pub fn IsMipFilterLinear(&self) -> bool { false }
        pub fn UseMipmapFiltering(&self) -> bool { false }
    }
    pub struct Topo { _priv: () }
    pub mod Topology { pub use super::Topo as T; pub type Point = super::Topo; pub type Line = super::Topo; pub type Triangle = super::Topo; }
    pub enum VSExpand { None, _Count }
    impl VSSelector { pub fn UseVSExpandIndexBuffer(&self) -> bool { false } }
    pub struct alpha_second_pass_t { pub enable: bool, pub no_color1: bool, pub blend_hw: bool, pub dither: bool, pub ps: PSSelector, pub colormask: ColorMaskSelector, pub depth: DepthStencilSelector, pub ps_aref: f32, pub require_one_barrier: bool, pub require_full_barrier: bool }
    pub struct blend_multi_pass_t { pub enable: bool, pub no_color1: bool, pub blend_hw: bool, pub dither: bool, pub blend: BlendState }
    pub enum DestinationAlphaMode { Off, Full, PrimIDTracking, StencilOne, Stencil }
    pub enum ColClipMode { None, EarlyResolve, ConvertOnly, ResolveOnly, ConvertAndResolve }
    pub struct GSHWDrawConfig {
        pub cb_vs: VSConstantBuffer, pub cb_ps: PSConstantBuffer, pub vs: VSSelector, pub ps: PSSelector,
        pub tex: *mut GSTexture, pub pal: *mut GSTexture, pub rt: *mut GSTexture, pub ds: *mut GSTexture,
        pub sampler: SamplerSelector, pub blend: BlendState, pub topology: Topo,
        pub scissor: GSVector4i, pub drawarea: GSVector4i, pub samplearea: GSVector4i,
        pub colclip_mode: ColClipMode, pub colclip_update_area: GSVector4i, pub destination_alpha: DestinationAlphaMode,
        pub datm: u32, pub require_one_barrier: bool, pub require_full_barrier: bool,
        pub tex_hazard: u32, pub line_expand: bool, pub alpha_second_pass: alpha_second_pass_t,
        pub blend_multi_pass: blend_multi_pass_t,
    }
    pub fn GetExpansionFactor(_e: VSExpand) -> u32 { 1 }
    pub struct GSVertex { pub _data: [u8; 32] }
    pub struct PSConstantBuffer { pub FogColor_AREF: PSConstantBufferF }
    pub struct PSConstantBufferF { pub a: f32, pub _pad: [f32; 3] }
    impl PSConstantBuffer { pub fn ScaleFactor(&self) -> GSVector4 { GSVector4::ONE } }
}
pub fn IsDATEModePrimIDInit(flag: u32) -> bool { flag == 1 || flag == 2 }

// =====================================================================
//  Section 13.  GSDevice trait.
// =====================================================================

pub trait GSDevice {
    fn Create(&mut self, vsync: GSVsyncModeKind, allow_present_throttle: bool) -> bool { false }
    fn Destroy(&mut self) {}
    fn UpdateWindow(&mut self) -> bool { false }
    fn ResizeWindow(&mut self, _w: u32, _h: u32, _scale: f32) {}
    fn SupportsExclusiveFullscreen(&self) -> bool { false }
    fn DestroySurface(&mut self) {}
    fn GetDriverInfo(&self) -> String { String::new() }
    fn SetVSyncMode(&mut self, _mode: GSVsyncModeKind, _allow: bool) {}
    fn BeginPresent(&mut self, _frame_skip: bool) -> PresentResult { PresentResult::OK }
    fn EndPresent(&mut self) {}
    fn IsPresenting(&self) -> bool { false }
    fn SetGPUTimingEnabled(&mut self, _e: bool) -> bool { false }
    fn GetAndResetAccumulatedGPUTime(&mut self) -> f32 { 0.0 }
    fn PushDebugGroup(&mut self, _group: &str) {}
    fn PopDebugGroup(&mut self) {}
    fn InsertDebugMessage(&mut self, _cat: DebugMessageCategory, _msg: &str) {}
    fn RenderImGui(&mut self) {}
    fn CreateSurface(&mut self, _t: GSTextureType, _w: i32, _h: i32, _levels: i32, _fmt: GSTextureFormatEnum) -> *mut GSTexture { ptr::null_mut() }
    fn CreateDownloadTexture(&mut self, _w: u32, _h: u32, _fmt: GSTextureFormatEnum) -> Option<Box<GSDownloadTextureVK>> { None }
    fn CreateRenderTarget(&mut self, _w: i32, _h: i32, _fmt: GSTextureFormatEnum, _clear: bool) -> *mut GSTexture { ptr::null_mut() }
    fn CreateTexture(&mut self, _w: i32, _h: i32, _levels: i32, _fmt: GSTextureFormatEnum, _mipmap: bool) -> *mut GSTexture { ptr::null_mut() }
    fn CreateDepthStencil(_w: i32, _h: i32, _fmt: GSTextureFormatEnum, _clear: bool) -> *mut GSTexture { ptr::null_mut() }
    fn ClearSamplerCache(&mut self) {}
    fn RenderHW(&mut self, _config: &mut GSHWDrawConfig::GSHWDrawConfig) {}
    fn GetRenderAPI(&self) -> RenderAPI;
    fn HasSurface(&self) -> bool { false }
    fn GetFeatures(&self) -> FeatureSupport;
    fn GetWindowWidth(&self) -> i32 { 0 }
    fn GetWindowHeight(&self) -> i32 { 0 }
    fn SetColorClipTexture(&mut self, _t: *mut GSTexture) {}
    fn GetColorClipTexture(&self) -> *mut GSTexture { ptr::null_mut() }
    fn Recycle(&mut self, _t: *mut GSTexture) {}
    fn PurgePool(&mut self) {}
    fn AcquireWindow(&mut self, _b: bool) -> bool { true }
    fn ProcessClearsBeforeCopy(&mut self, _s: *mut GSTexture, _d: *mut GSTexture, _full: bool) -> bool { false }
    fn ProcessCopyArea(_a: GSVector4i, _b: GSVector4i) -> GSVector4i { GSVector4i::ZERO }
    fn GenerateExpansionIndexBuffer(_ptr: *mut c_void) {}
    fn ReadShaderSource(_path: &str) -> Option<String> { None }
    fn GetCASShaderSource(_src: &mut String) -> bool { false }
    fn ShortSpin() {}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSVsyncModeKind { Disabled, FIFO, Mailbox }
pub const g_gs_device: *mut GSDeviceVK = ptr::null_mut();
pub struct Host;
impl Host {
    pub fn ReportErrorAsync(_a: &str, _b: &str) {}
    pub fn ReportFormattedErrorAsync(_a: &str, _b: &str, _c: std::fmt::Arguments) {}
    pub fn AddKeyedOSDMessage(_a: &str, _b: &str, _c: f32) {}
    pub const OSD_WARNING_DURATION: f32 = 5.0;
}
pub struct Pcsx2Config;
impl Pcsx2Config {
    pub fn TriStateToOptionalBoolean(_b: bool) -> Option<bool> { None }
}
pub struct GSConfig;
impl GSConfig {
    pub fn UseDebugDevice() -> bool { false }
    pub fn HWSpinCPUForReadbacks() -> bool { false }
    pub fn HWSpinGPUForReadbacks() -> bool { false }
    pub fn DisableFramebufferFetch() -> bool { false }
    pub fn OverrideTextureBarriers() -> i32 { 0 }
    pub fn DisableVertexShaderExpand() -> bool { false }
    pub fn HWAA1() -> bool { false }
    pub fn DisableShaderCache() -> bool { false }
    pub fn HWROVBarriersVK() -> bool { false }
    pub fn Adapter() -> String { String::new() }
    pub fn UpscaleMultiplier() -> f32 { 1.0 }
    pub fn UserHacks_NativeScaling() -> GSNativeScaling { GSNativeScaling::Normal }
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GSNativeScaling { Normal, Aggressive }

pub struct GSPerfMon;
impl GSPerfMon {
    pub fn Put(&mut self, _k: GSPerfMonKey, _v: i32) {}
}
pub enum GSPerfMonKey { DrawCalls, TextureCopies, TextureUploads, Readbacks, RenderPasses, Barriers, BarriersROV }
pub fn g_perfmon() -> GSPerfMon { GSPerfMon }
pub fn g_gs_device_get() -> *mut GSDeviceVK { ptr::null_mut() }

pub fn Error_AddPrefix(_e: *mut c_void, _s: &str) {}
pub fn TRANSLATE_SV<'a>(_a: &'a str, _b: &'a str) -> &'a str { "" }
pub fn TRANSLATE_STR<'a>(_a: &'a str, _b: &'a str) -> &'a str { "" }
pub fn INFO_LOG(_fmt: &str) {}
pub fn WARNING_LOG(_fmt: std::fmt::Arguments) {}
pub fn ERROR_LOG(_fmt: std::fmt::Arguments) {}
pub fn DEV_LOG(_fmt: std::fmt::Arguments) {}
pub struct DevCon;
impl DevCon {
    pub fn WriteLn(&mut self, _a: std::fmt::Arguments) {}
}
pub fn GetDefaultAdapter() -> String { String::new() }
pub const Color_StrongOrange: u32 = 0xffaa00;
pub const Color_StrongGreen: u32 = 0x00ff00;
pub struct GSGL;
impl GSGL { pub fn INS(_a: std::fmt::Arguments) {} pub fn PUSH(_a: std::fmt::Arguments) {} pub fn POP() {} pub fn PUSH_(_a: std::fmt::Arguments) {} }
pub fn GL_INS(_a: std::fmt::Arguments) {}
pub fn GL_PUSH(_a: std::fmt::Arguments) {}
pub fn GL_POP() {}
pub fn GL_PUSH_(_a: std::fmt::Arguments) {}

pub struct GSDownloadTextureVKHandle;

// =====================================================================
//  Section 14.  ReadbackSpinManager and friends.
// =====================================================================

pub struct ReadbackSpinManager;
impl ReadbackSpinManager {
    pub fn DrawSubmitted(&mut self, _rp: u32) -> ReadbackSpinResult { ReadbackSpinResult { id: 0, recommended_spin: 0 } }
    pub fn DrawCompleted(&mut self, _id: i32, _t1: u64, _t2: u64) {}
    pub fn SpinCompleted(&mut self, _cycles: u32, _t1: u64, _t2: u64) {}
    pub fn NextFrame(&mut self) {}
    pub fn ReadbackRequested(&mut self) {}
    pub fn SpinsPerUnitTime(&self) -> u32 { 1 }
}
pub struct ReadbackSpinResult { pub id: i32, pub recommended_spin: u32 }

// =====================================================================
//  Section 15.  VMA + Vulkan loader modules (mocked).
// =====================================================================

pub mod Vulkan {
    use super::*;
    pub fn ResetVulkanLibraryFunctionPointers() {}
    pub fn IsVulkanLibraryLoaded() -> bool { false }
    pub fn LoadVulkanLibrary(_err: *mut c_void) -> bool { false }
    pub fn UnloadVulkanLibrary() {}
    pub fn LoadVulkanInstanceFunctions(_i: VkInstance) -> bool { false }
    pub fn LoadVulkanDeviceFunctions(_d: VkDevice) -> bool { false }
    pub fn AddPointerToChain(_head: *mut c_void, _ptr: *const c_void) {}
    pub fn VkResultToString(_r: VkResult) -> &'static str { "VK_RESULT" }
    pub fn LogVulkanResult(_f: &str, _r: VkResult, _m: &str) {}

    // ---- Builder pattern types from VKBuilders.h ----
    pub const MAX_BINDINGS: u32 = 16;
    pub const MAX_SETS: u32 = 8;
    pub const MAX_PUSH_CONSTANTS: u32 = 1;
    pub const MAX_SHADER_STAGES: u32 = 3;
    pub const MAX_VERTEX_ATTRIBUTES: u32 = 16;
    pub const MAX_VERTEX_BUFFERS: u32 = 8;
    pub const MAX_ATTACHMENTS: u32 = 2;
    pub const MAX_DYNAMIC_STATE: u32 = 8;
    pub const SPECIALIZATION_CONSTANT_SIZE: u32 = 4;
    pub const MAX_SPECIALIZATION_CONSTANTS: u32 = 4;
    pub const MAX_WRITES: u32 = 16;
    pub const MAX_IMAGE_INFOS: u32 = 8;
    pub const MAX_BUFFER_INFOS: u32 = 4;
    pub const MAX_VIEWS: u32 = 4;
    pub const MAX_ATTACHMENT_REFERENCES: u32 = 2;
    pub const MAX_SUBPASSES: u32 = 1;

    pub struct DescriptorSetLayoutBuilder {
        pub m_ci: VkDescriptorSetLayoutCreateInfo,
        pub m_bindings: [VkDescriptorSetLayoutBinding; 16],
        pub m_binding_count: u32,
    }
    impl DescriptorSetLayoutBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn SetPushFlag(&mut self) { self.m_ci.flags |= VK_DESCRIPTOR_SET_LAYOUT_CREATE_PUSH_DESCRIPTOR_BIT_KHR }
        pub fn Create(&mut self, _d: VkDevice) -> VkDescriptorSetLayout { ptr::null_mut() }
        pub fn AddBinding(&mut self, _b: u32, _t: VkDescriptorType, _c: u32, _s: VkShaderStageFlags) {}
    }

    pub struct PipelineLayoutBuilder {
        pub m_ci: VkPipelineLayoutCreateInfo,
        pub m_sets: [VkDescriptorSetLayout; 8],
        pub m_push_constants: [VkPushConstantRange; 1],
        pub m_set_count: u32, pub m_push_count: u32,
    }
    impl PipelineLayoutBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice) -> VkPipelineLayout { ptr::null_mut() }
        pub fn AddDescriptorSet(&mut self, _l: VkDescriptorSetLayout) {}
        pub fn AddPushConstants(&mut self, _s: VkShaderStageFlags, _o: u32, _sz: u32) {}
    }

    pub struct GraphicsPipelineBuilder {
        pub m_ci: VkGraphicsPipelineCreateInfo,
        pub m_shader_stages: [VkPipelineShaderStageCreateInfo; 3],
        pub m_vertex_input_state: VkPipelineVertexInputStateCreateInfo,
        pub m_vertex_buffers: [VkVertexInputBindingDescription; 8],
        pub m_vertex_attributes: [VkVertexInputAttributeDescription; 16],
        pub m_input_assembly: VkPipelineInputAssemblyStateCreateInfo,
        pub m_rasterization_state: VkPipelineRasterizationStateCreateInfo,
        pub m_depth_state: VkPipelineDepthStencilStateCreateInfo,
        pub m_blend_state: VkPipelineColorBlendStateCreateInfo,
        pub m_blend_attachments: [VkPipelineColorBlendAttachmentState; 2],
        pub m_viewport_state: VkPipelineViewportStateCreateInfo,
        pub m_viewport: VkViewport,
        pub m_scissor: VkRect2D,
        pub m_dynamic_state: VkPipelineDynamicStateCreateInfo,
        pub m_dynamic_state_values: [VkDynamicState; 8],
        pub m_multisample_state: VkPipelineMultisampleStateCreateInfo,
        pub m_provoking_vertex: VkPipelineRasterizationProvokingVertexStateCreateInfoEXT,
        pub m_line_rasterization_state: VkPipelineRasterizationLineStateCreateInfoEXT,
    }
    impl GraphicsPipelineBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice, _c: VkPipelineCache, _clear: bool) -> VkPipeline { ptr::null_mut() }
        pub fn SetShaderStage(&mut self, _s: VkShaderStageFlagBits, _m: VkShaderModule, _e: &str) {}
        pub fn SetVertexShader(&mut self, m: VkShaderModule) { self.SetShaderStage(VK_SHADER_STAGE_VERTEX, m, "main") }
        pub fn SetGeometryShader(&mut self, m: VkShaderModule) { self.SetShaderStage(VK_SHADER_STAGE_GEOMETRY, m, "main") }
        pub fn SetFragmentShader(&mut self, m: VkShaderModule) { self.SetShaderStage(VK_SHADER_STAGE_FRAGMENT, m, "main") }
        pub fn AddVertexBuffer(&mut self, _b: u32, _s: u32, _r: VkVertexInputRate) {}
        pub fn AddVertexAttribute(&mut self, _l: u32, _b: u32, _f: VkFormat, _o: u32) {}
        pub fn SetPrimitiveTopology(&mut self, _t: VkPrimitiveTopology, _restart: bool) {}
        pub fn SetRasterizationState(&mut self, _p: VkPolygonMode, _c: VkCullModeFlags, _f: VkFrontFace) {}
        pub fn SetLineWidth(&mut self, _w: f32) {}
        pub fn SetLineRasterizationMode(&mut self, _m: VkLineRasterizationModeEXT) {}
        pub fn SetMultisamples(&mut self, _s: u32, _p: bool) {}
        pub fn SetNoCullRasterizationState(&mut self) { self.SetRasterizationState(VK_POLYGON_MODE_FILL, VK_CULL_MODE_NONE, VK_FRONT_FACE_CLOCKWISE) }
        pub fn SetDepthState(&mut self, _t: bool, _w: bool, _o: VkCompareOp) {}
        pub fn SetStencilState(&mut self, _t: bool, _f: VkStencilOpState, _b: VkStencilOpState) {}
        pub fn SetNoDepthTestState(&mut self) { self.SetDepthState(false, false, VK_COMPARE_OP_ALWAYS) }
        pub fn SetNoStencilState(&mut self) { self.m_depth_state.stencilTestEnable = 0 }
        pub fn AddBlendAttachment(&mut self, _e: bool, _a: VkBlendFactor, _b: VkBlendFactor, _o: VkBlendOp, _c: VkBlendFactor, _d: VkBlendFactor, _f: VkBlendOp, _m: VkColorComponentFlags) {}
        pub fn SetBlendAttachment(&mut self, _i: u32, _e: bool, _a: VkBlendFactor, _b: VkBlendFactor, _o: VkBlendOp, _c: VkBlendFactor, _d: VkBlendFactor, _f: VkBlendOp, _m: VkColorComponentFlags) {}
        pub fn SetColorWriteMask(&mut self, _a: u32, _m: VkColorComponentFlags) {}
        pub fn AddBlendFlags(&mut self, _f: u32) {}
        pub fn ClearBlendAttachments(&mut self) {}
        pub fn SetBlendConstants(&mut self, _r: f32, _g: f32, _b: f32, _a: f32) {}
        pub fn SetNoBlendingState(&mut self) { self.ClearBlendAttachments() }
        pub fn AddDynamicState(&mut self, _s: VkDynamicState) {}
        pub fn SetDynamicViewportAndScissorState(&mut self) { self.AddDynamicState(VK_DYNAMIC_STATE_VIEWPORT); self.AddDynamicState(VK_DYNAMIC_STATE_SCISSOR) }
        pub fn SetViewport(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _min: f32, _max: f32) {}
        pub fn SetScissorRect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) {}
        pub fn SetMultisamples2(&mut self, _s: VkSampleCountFlagBits) {}
        pub fn SetPipelineLayout(&mut self, _l: VkPipelineLayout) {}
        pub fn SetRenderPass(&mut self, _r: VkRenderPass, _s: u32) {}
        pub fn SetProvokingVertex(&mut self, _m: VkProvokingVertexModeEXT) {}
    }

    pub struct ComputePipelineBuilder {
        pub m_ci: VkComputePipelineCreateInfo,
        pub m_si: VkSpecializationInfo,
        pub m_smap_entries: [VkSpecializationMapEntry; 4],
        pub m_smap_constants: [u8; 16],
    }
    impl ComputePipelineBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice, _c: VkPipelineCache, _cl: bool) -> VkPipeline { ptr::null_mut() }
        pub fn SetShader(&mut self, _m: VkShaderModule, _e: &str) {}
        pub fn SetPipelineLayout(&mut self, _l: VkPipelineLayout) {}
        pub fn SetSpecializationBool(&mut self, _i: u32, _v: bool) {}
    }

    pub struct SamplerBuilder { pub m_ci: VkSamplerCreateInfo }
    impl SamplerBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice, _cl: bool) -> VkSampler { ptr::null_mut() }
        pub fn SetFilter(&mut self, _a: VkFilter, _b: VkFilter, _c: VkSamplerMipmapMode) {}
        pub fn SetAddressMode(&mut self, _a: VkSamplerAddressMode, _b: VkSamplerAddressMode, _c: VkSamplerAddressMode) {}
        pub fn SetPointSampler(&mut self, _a: VkSamplerAddressMode) {}
        pub fn SetLinearSampler(&mut self, _m: bool, _a: VkSamplerAddressMode) {}
    }

    pub struct DescriptorSetUpdateBuilder {
        pub m_writes: [VkWriteDescriptorSet; 16],
        pub m_num_writes: u32,
        pub m_buffer_infos: [VkDescriptorBufferInfo; 4],
        pub m_image_infos: [VkDescriptorImageInfo; 8],
        pub m_views: [VkBufferView; 4],
        pub m_num_buffer_infos: u32, pub m_num_image_infos: u32, pub m_num_views: u32,
    }
    impl DescriptorSetUpdateBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Update(&mut self, _d: VkDevice, _cl: bool) {}
        pub fn PushUpdate(&mut self, _c: VkCommandBuffer, _b: VkPipelineBindPoint, _l: VkPipelineLayout, _s: u32, _cl: bool) {}
        pub fn AddImageDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _v: VkImageView, _l: VkImageLayout, _si: bool) {}
        pub fn AddSamplerDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _sa: VkSampler) {}
        pub fn AddSamplerDescriptorWrites(&mut self, _s: VkDescriptorSet, _b: u32, _sa: *const VkSampler, _n: u32) {}
        pub fn AddCombinedImageSamplerDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _v: VkImageView, _sa: VkSampler, _l: VkImageLayout) {}
        pub fn AddCombinedImageSamplerDescriptorWrites(&mut self, _s: VkDescriptorSet, _b: u32, _v: *const VkImageView, _sa: *const VkSampler, _n: u32, _l: VkImageLayout) {}
        pub fn AddBufferDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _t: VkDescriptorType, _buf: VkBuffer, _o: u32, _sz: u32) {}
        pub fn AddBufferViewDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _t: VkDescriptorType, _v: VkBufferView) {}
        pub fn AddInputAttachmentDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _v: VkImageView, _l: VkImageLayout) {}
        pub fn AddStorageImageDescriptorWrite(&mut self, _s: VkDescriptorSet, _b: u32, _v: VkImageView, _l: VkImageLayout) {}
    }

    pub struct FramebufferBuilder { pub m_ci: VkFramebufferCreateInfo, pub m_images: [VkImageView; 2] }
    impl FramebufferBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice, _cl: bool) -> VkFramebuffer { ptr::null_mut() }
        pub fn AddAttachment(&mut self, _i: VkImageView) {}
        pub fn SetSize(&mut self, _w: u32, _h: u32, _l: u32) {}
        pub fn SetRenderPass(&mut self, _r: VkRenderPass) {}
    }

    pub struct RenderPassBuilder {
        pub m_ci: VkRenderPassCreateInfo,
        pub m_attachments: [VkAttachmentDescription; 2],
        pub m_attachment_references: [VkAttachmentReference; 2],
        pub m_num_attachment_references: u32,
        pub m_subpasses: [VkSubpassDescription; 1],
    }
    impl RenderPassBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice, _cl: bool) -> VkRenderPass { ptr::null_mut() }
        pub fn AddAttachment(&mut self, _f: VkFormat, _s: VkSampleCountFlagBits, _l: VkAttachmentLoadOp, _st: VkAttachmentStoreOp, _il: VkImageLayout, _fl: VkImageLayout) -> u32 { 0 }
        pub fn AddSubpass(&mut self) -> u32 { 0 }
        pub fn AddSubpassColorAttachment(&mut self, _sp: u32, _a: u32, _l: VkImageLayout) {}
        pub fn AddSubpassDepthAttachment(&mut self, _sp: u32, _a: u32, _l: VkImageLayout) {}
    }

    pub struct BufferViewBuilder { pub m_ci: VkBufferViewCreateInfo }
    impl BufferViewBuilder {
        pub fn new() -> Self { unsafe { zeroed() } }
        pub fn Clear(&mut self) { *self = Self::new() }
        pub fn Create(&mut self, _d: VkDevice, _cl: bool) -> VkBufferView { ptr::null_mut() }
        pub fn Set(&mut self, _b: VkBuffer, _f: VkFormat, _o: u32, _sz: u32) {}
    }

    // Debug object naming
    pub fn SetObjectName<T>(_device: VkDevice, _handle: T, _format: &str) {}
}

// =====================================================================
//  Section 16.  VKStreamBuffer
// =====================================================================

#[derive(Default)]
pub struct VKStreamBuffer {
    pub m_size: u32,
    pub m_current_offset: u32,
    pub m_current_space: u32,
    pub m_current_gpu_position: u32,
    pub m_allocation: VmaAllocation,
    pub m_buffer: VkBuffer,
    pub m_host_pointer: *mut u8,
    pub m_tracked_fences: Vec<(u64, u32)>,
}
impl VKStreamBuffer {
    pub fn IsValid(&self) -> bool { !self.m_buffer.is_null() }
    pub fn GetCurrentOffset(&self) -> u32 { self.m_current_offset }
    pub fn GetCurrentSize(&self) -> u32 { self.m_size }
    pub fn GetCurrentHostPointer(&self) -> *mut u8 { self.m_host_pointer }
    pub fn GetBuffer(&self) -> VkBuffer { self.m_buffer }
    pub fn GetBufferPtr(&self) -> *const VkBuffer { &self.m_buffer }
    pub fn Create(&mut self, _usage: u32, _size: u32) -> bool { self.m_size = _size; true }
    pub fn Destroy(&mut self, _defer: bool) { self.m_buffer = ptr::null_mut(); self.m_allocation = ptr::null_mut(); self.m_host_pointer = ptr::null_mut(); self.m_size = 0; }
    pub fn ReserveMemory(&mut self, _n: u32, _a: u32) -> bool { true }
    pub fn CommitMemory(&mut self, _n: u32) {}
    pub fn UpdateCurrentFencePosition(&mut self) {}
    pub fn UpdateGPUPosition(&mut self) {}
    pub fn WaitForClearSpace(&mut self, _n: u32) -> bool { true }
}

// =====================================================================
//  Section 17.  VKSwapChain
// =====================================================================

pub struct VKSwapChain {
    pub m_window_info: WindowInfo,
    pub m_surface: VkSurfaceKHR,
    pub m_present_mode: VkPresentModeKHR,
    pub m_exclusive_fullscreen_control: Option<bool>,
    pub m_swap_chain: VkSwapchainKHR,
    pub m_images: Vec<Box<GSTextureVK>>,
    pub m_current_image: u32,
    pub m_current_semaphore: u32,
    pub m_semaphores: [ImageSemaphores; 4],
    pub m_image_acquire_result: Option<VkResult>,
}
#[derive(Copy, Clone)]
pub struct ImageSemaphores { pub available_semaphore: VkSemaphore, pub rendering_finished_semaphore: VkSemaphore }
impl VKSwapChain {
    pub const NUM_SEMAPHORES: usize = 4;
    pub fn new(wi: WindowInfo, surface: VkSurfaceKHR, pm: VkPresentModeKHR, e: Option<bool>) -> Self { Self { m_window_info: wi, m_surface: surface, m_present_mode: pm, m_exclusive_fullscreen_control: e, m_swap_chain: ptr::null_mut(), m_images: Vec::new(), m_current_image: 0, m_current_semaphore: 0, m_semaphores: [ImageSemaphores { available_semaphore: ptr::null_mut(), rendering_finished_semaphore: ptr::null_mut() }; 4], m_image_acquire_result: None } }
    pub fn CreateVulkanSurface(_i: VkInstance, _p: VkPhysicalDevice, _w: *mut WindowInfo) -> VkSurfaceKHR { ptr::null_mut() }
    pub fn DestroyVulkanSurface(_i: VkInstance, _w: *mut WindowInfo, _s: VkSurfaceKHR) {}
    pub fn Create(_w: WindowInfo, _s: VkSurfaceKHR, _p: VkPresentModeKHR, _e: Option<bool>) -> Option<Box<Self>> { None }
    pub fn SelectPresentMode(_s: VkSurfaceKHR, _m: *mut GSVsyncModeKind, _o: *mut VkPresentModeKHR) -> bool { false }
    pub fn SelectSurfaceFormat(_s: VkSurfaceKHR) -> Option<VkSurfaceFormatKHR> { None }
    pub fn CreateSwapChain(&mut self) -> bool { false }
    pub fn DestroySwapChain(&mut self) {}
    pub fn DestroySwapChainImages(&mut self) {}
    pub fn DestroySurface(&mut self) {}
    pub fn GetTextureFormat(&self) -> VkFormat { VK_FORMAT_UNDEFINED }
    pub fn AcquireNextImage(&mut self) -> VkResult { VK_SUCCESS }
    pub fn ReleaseCurrentImage(&mut self) {}
    pub fn ResetImageAcquireResult(&mut self) { self.m_image_acquire_result = None }
    pub fn ResizeSwapChain(&mut self, _w: u32, _h: u32, _sc: f32) -> bool { false }
    pub fn SetPresentMode(&mut self, _m: VkPresentModeKHR) -> bool { false }
    pub fn RecreateSurface(&mut self, _w: WindowInfo) -> bool { false }
    pub fn GetCurrentTexture(&mut self) -> &mut GSTextureVK { unimplemented!() }
    pub fn GetWidth(&self) -> u32 { self.m_window_info.surface_width }
    pub fn GetHeight(&self) -> u32 { self.m_window_info.surface_height }
    pub fn GetWindowInfo(&self) -> WindowInfo { self.m_window_info.clone() }
    pub fn GetSurface(&self) -> VkSurfaceKHR { self.m_surface }
    pub fn GetSwapChainPtr(&self) -> *const VkSwapchainKHR { &self.m_swap_chain }
    pub fn GetCurrentImageIndexPtr(&self) -> *const u32 { &self.m_current_image }
    pub fn GetImageAvailableSemaphore(&self) -> VkSemaphore { self.m_semaphores[self.m_current_semaphore as usize].available_semaphore }
    pub fn GetImageAvailableSemaphorePtr(&self) -> *const VkSemaphore { &self.m_semaphores[self.m_current_semaphore as usize].available_semaphore }
    pub fn GetRenderingFinishedSemaphore(&self) -> VkSemaphore { self.m_semaphores[self.m_current_semaphore as usize].rendering_finished_semaphore }
    pub fn GetRenderingFinishedSemaphorePtr(&self) -> *const VkSemaphore { &self.m_semaphores[self.m_current_semaphore as usize].rendering_finished_semaphore }
}

#[derive(Default, Debug, Clone)]
pub struct WindowInfo { pub surface_width: u32, pub surface_height: u32, pub surface_scale: f32, pub window_handle: u64, pub display_connection: *mut c_void, pub surface_handle: *mut c_void, pub type_: WindowInfoType }
impl WindowInfo {
    pub fn type_is_surfaceless(&self) -> bool { matches!(self.type_, WindowInfoType::Surfaceless) }
}
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum WindowInfoType { #[default] Surfaceless, Win32, X11, Wayland, MacOS }

// =====================================================================
//  Section 18.  VKShaderCache
// =====================================================================

pub struct VKShaderCache {
    pub m_pipeline_cache: VkPipelineCache,
    pub m_pipeline_cache_dirty: bool,
    pub m_pipeline_cache_filename: String,
    pub m_index_file: Option<File>,
    pub m_blob_file: Option<File>,
    pub m_index: HashMap<CacheIndexKey, CacheIndexData>,
}
impl VKShaderCache {
    pub fn new() -> Self { Self { m_pipeline_cache: ptr::null_mut(), m_pipeline_cache_dirty: false, m_pipeline_cache_filename: String::new(), m_index_file: None, m_blob_file: None, m_index: HashMap::new() } }
    pub fn Create() {}
    pub fn Destroy() {}
    pub fn Open(&mut self) {}
    pub fn GetPipelineCache(&mut self, _dirty: bool) -> VkPipelineCache { self.m_pipeline_cache }
    pub fn GetVertexShader(&mut self, _c: &str) -> VkShaderModule { ptr::null_mut() }
    pub fn GetFragmentShader(&mut self, _c: &str) -> VkShaderModule { ptr::null_mut() }
    pub fn GetComputeShader(&mut self, _c: &str) -> VkShaderModule { ptr::null_mut() }
    pub fn CompileShaderToSPV(_st: u32, _c: &str, _d: bool) -> Option<Vec<u32>> { None }
    pub fn GetShaderSPV(&mut self, _t: u32, _c: &str) -> Option<Vec<u32>> { None }
    pub fn CompileAndAddShaderSPV(&mut self, _k: CacheIndexKey, _c: &str) -> Option<Vec<u32>> { None }
    pub fn GetCacheKey(_t: u32, _c: &str) -> CacheIndexKey { CacheIndexKey { source_hash_low: 0, source_hash_high: 0, source_length: 0, shader_type: _t } }
    pub fn ReadExistingShaderCache(&mut self, _i: &str, _b: &str) -> bool { false }
    pub fn CreateNewShaderCache(&mut self, _i: &str, _b: &str) -> bool { false }
    pub fn CloseShaderCache(&mut self) {}
    pub fn CreateNewPipelineCache(&mut self) -> bool { false }
    pub fn ReadExistingPipelineCache(&mut self) -> bool { false }
    pub fn FlushPipelineCache(&mut self) -> bool { false }
    pub fn ClosePipelineCache(&mut self) {}
    pub fn GetShaderCacheBaseFileName(_d: bool) -> String { String::new() }
    pub fn GetPipelineCacheBaseFileName(_d: bool) -> String { String::new() }
}
pub type SPIRVCodeType = u32;
pub type SPIRVCodeVector = Vec<u32>;
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq, Hash)]
pub struct CacheIndexKey { pub source_hash_low: u64, pub source_hash_high: u64, pub source_length: u32, pub shader_type: u32 }
#[derive(Copy, Clone, Default, Debug)]
pub struct CacheIndexData { pub file_offset: u32, pub blob_size: u32 }
pub fn g_vulkan_shader_cache() -> *mut VKShaderCache { ptr::null_mut() }

// =====================================================================
//  Section 19.  GSDeviceVK - the main struct
// =====================================================================

pub struct GSDeviceVK {
    pub instance: VkInstance,
    pub physical_device: VkPhysicalDevice,
    pub device: VkDevice,
    pub allocator: VmaAllocator,
    pub current_command_buffer: VkCommandBuffer,
    pub global_descriptor_pool: VkDescriptorPool,
    pub graphics_queue: VkQueue,
    pub present_queue: VkQueue,
    pub graphics_queue_family_index: u32,
    pub present_queue_family_index: u32,

    pub spin_manager: ReadbackSpinManager,
    pub spin_queue: VkQueue,
    pub spin_descriptor_set_layout: VkDescriptorSetLayout,
    pub spin_pipeline_layout: VkPipelineLayout,
    pub spin_pipeline: VkPipeline,
    pub spin_buffer: VkBuffer,
    pub spin_buffer_allocation: VmaAllocation,
    pub spin_descriptor_set: VkDescriptorSet,
    pub spin_resources: [SpinResources; 3],
    pub queryperfcounter_to_ns: f64,
    pub spin_timestamp_scale: f64,
    pub spin_timestamp_offset: f64,
    pub spin_queue_family_index: u32,
    pub command_buffer_render_passes: u32,
    pub spin_timer: u32,
    pub spinning_supported: bool,
    pub spin_queue_is_graphics_queue: bool,
    pub spin_buffer_initialized: bool,

    pub timestamp_query_pool: VkQueryPool,
    pub accumulated_gpu_time: f32,
    pub gpu_timing_enabled: bool,
    pub gpu_timing_supported: bool,
    pub wants_new_timestamp_calibration: bool,
    pub calibrated_timestamp_type: VkTimeDomainEXT,

    pub frame_resources: [FrameResources; 3],
    pub next_fence_counter: u64,
    pub completed_fence_counter: u64,
    pub current_frame: u32,

    pub last_submit_failed: bool,

    pub render_pass_cache: BTreeMap<u32, VkRenderPass>,

    pub debug_messenger_callback: VkDebugUtilsMessengerEXT,

    pub device_features: VkPhysicalDeviceFeatures,
    pub device_properties: VkPhysicalDeviceProperties,
    pub device_driver_properties: VkPhysicalDeviceDriverProperties,
    pub optional_extensions: OptionalExtensions,

    pub swap_chain: Option<Box<VKSwapChain>>,
    pub resize_requested: bool,
    pub is_presenting: bool,
    pub vblank_wait_supported: bool,
    pub vblank_wait: bool,
    pub vblank_skipped: bool,
    pub vblank_realtime: bool,
    pub vsync_mode: GSVsyncModeKind,
    pub allow_present_throttle: bool,

    pub utility_ds_layout: VkDescriptorSetLayout,
    pub utility_pipeline_layout: VkPipelineLayout,
    pub tfx_ubo_ds_layout: VkDescriptorSetLayout,
    pub tfx_texture_ds_layout: VkDescriptorSetLayout,
    pub tfx_pipeline_layout: VkPipelineLayout,

    pub vertex_stream_buffer: VKStreamBuffer,
    pub index_stream_buffer: VKStreamBuffer,
    pub expand_index_stream_buffer: VKStreamBuffer,
    pub vertex_uniform_stream_buffer: VKStreamBuffer,
    pub fragment_uniform_stream_buffer: VKStreamBuffer,
    pub texture_stream_buffer: VKStreamBuffer,
    pub expand_index_buffer: VkBuffer,
    pub expand_index_buffer_allocation: VmaAllocation,

    pub point_sampler: VkSampler,
    pub linear_sampler: VkSampler,
    pub samplers: HashMap<u32, VkSampler>,

    pub convert: Vec<VkPipeline>,
    pub present: [VkPipeline; 2],
    pub merge: [VkPipeline; 2],
    pub interlace: [VkPipeline; NUM_INTERLACE_SHADERS],
    pub colclip_setup_pipelines: [[VkPipeline; 2]; 2],
    pub colclip_finish_pipelines: [[VkPipeline; 2]; 2],
    pub primid_image_setup_render_passes: [[VkRenderPass; 2]; 2],
    pub primid_image_setup_pipelines: [[VkPipeline; 4]; 2],
    pub fxaa_pipeline: VkPipeline,
    pub shadeboost_pipeline: VkPipeline,

    pub tfx_vertex_shaders: HashMap<u32, VkShaderModule>,
    pub tfx_fragment_shaders: HashMap<(u64, u64), VkShaderModule>,
    pub tfx_pipelines: HashMap<PipelineSelector, VkPipeline>,

    pub utility_color_render_pass_load: VkRenderPass,
    pub utility_color_render_pass_clear: VkRenderPass,
    pub utility_color_render_pass_discard: VkRenderPass,
    pub utility_depth_render_pass_load: VkRenderPass,
    pub utility_depth_render_pass_clear: VkRenderPass,
    pub utility_depth_render_pass_discard: VkRenderPass,
    pub date_setup_render_pass: VkRenderPass,
    pub swap_chain_render_pass: VkRenderPass,
    pub tfx_render_pass: [[[[[[[[VkRenderPass; 3]; 3]; 2]; 2]; 2]; 2]; 2]; 2],

    pub cas_ds_layout: VkDescriptorSetLayout,
    pub cas_pipeline_layout: VkPipelineLayout,
    pub cas_pipelines: [VkPipeline; 2],
    pub imgui_pipeline: VkPipeline,

    pub vs_cb_cache: GSHWDrawConfig::VSConstantBuffer,
    pub ps_cb_cache: GSHWDrawConfig::PSConstantBuffer,
    pub vs_pc_cache: GSHWDrawConfig::VSPushConstants,
    pub tfx_source: String,

    pub features: FeatureSupport,

    pub dirty_flags: u32,
    pub current_framebuffer_feedback_loop: FeedbackLoopFlag,
    pub warned_slow_spin: bool,

    pub index_buffer: VkBuffer,
    pub current_render_target: *mut GSTextureVK,
    pub current_depth_target: *mut GSTextureVK,
    pub current_framebuffer: VkFramebuffer,
    pub current_render_pass: VkRenderPass,
    pub current_render_pass_area: GSVector4i,
    pub scissor: GSVector4i,
    pub viewport: VkViewport,
    pub current_line_width: f32,
    pub blend_constant_color: u8,

    pub tfx_textures: [*mut GSTextureVK; 7],
    pub tfx_sampler: VkSampler,
    pub tfx_sampler_sel: u32,
    pub tfx_ubo_descriptor_set: VkDescriptorSet,
    pub tfx_texture_descriptor_set: VkDescriptorSet,
    pub tfx_rt_descriptor_set: VkDescriptorSet,
    pub tfx_dynamic_offsets: [u32; 2],

    pub utility_texture: *const GSTextureVK,
    pub utility_sampler: VkSampler,
    pub utility_descriptor_set: VkDescriptorSet,

    pub current_pipeline_layout: PipelineLayout,
    pub current_pipeline: VkPipeline,

    pub null_texture: Option<Box<GSTextureVK>>,
    pub null_framebuffer: VkFramebuffer,

    pub pipeline_selector: PipelineSelector,

    pub shader_cache: VKShaderCache,
    pub console: Console,
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct OptionalExtensions {
    pub vk_ext_provoking_vertex: bool,
    pub vk_ext_memory_budget: bool,
    pub vk_ext_calibrated_timestamps: bool,
    pub vk_ext_rasterization_order_attachment_access: bool,
    pub vk_ext_full_screen_exclusive: bool,
    pub vk_ext_line_rasterization: bool,
    pub vk_swapchain_maintenance1: bool,
    pub vk_swapchain_maintenance1_is_khr: bool,
    pub vk_khr_driver_properties: bool,
    pub vk_khr_shader_non_semantic_info: bool,
    pub vk_ext_attachment_feedback_loop_layout: bool,
    pub vk_ext_fragment_shader_interlock: bool,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct SpinResources {
    pub command_pool: VkCommandPool,
    pub command_buffer: VkCommandBuffer,
    pub semaphore: VkSemaphore,
    pub fence: VkFence,
    pub cycles: u32,
    pub in_progress: bool,
}

#[derive(Default)]
pub struct FrameResources {
    pub command_pool: VkCommandPool,
    pub command_buffers: [VkCommandBuffer; 2],
    pub fence: VkFence,
    pub fence_counter: u64,
    pub spin_id: i32,
    pub submit_timestamp: u32,
    pub init_buffer_used: bool,
    pub needs_fence_wait: bool,
    pub timestamp_written: bool,
    pub cleanup_resources: Vec<Box<dyn FnOnce()>>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct PipelineSelector {
    pub ps: GSHWDrawConfig::PSSelector,
    pub topology: u8,
    pub rt: u8,
    pub ds: u8,
    pub line_width: u8,
    pub feedback_loop_flags: u8,
    pub bs: GSHWDrawConfig::BlendState,
    pub vs: GSHWDrawConfig::VSSelector,
    pub dss: GSHWDrawConfig::DepthStencilSelector,
    pub cms: GSHWDrawConfig::ColorMaskSelector,
    pub pad: u8,
}
impl PipelineSelector {
    pub const fn IsRTFeedbackLoop(&self) -> bool { (self.feedback_loop_flags & 1) != 0 }
    pub const fn IsDepthFeedbackLoop(&self) -> bool { (self.feedback_loop_flags & 4) != 0 }
    pub const fn IsTestingAndSamplingDepth(&self) -> bool { (self.feedback_loop_flags & 6) != 0 }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FeedbackLoopFlag { None = 0, ReadAndWriteRT = 1, ReadDepth = 2, ReadAndWriteDepth = 4 }
pub type PipelineLayout = i32;

// =====================================================================
//  Section 20.  GSDeviceVK implementation
// =====================================================================

impl GSDeviceVK {
    pub const NUM_COMMAND_BUFFERS: usize = 3;
    pub const NUM_TFX_DYNAMIC_OFFSETS: u32 = 2;
    pub const NUM_UTILITY_SAMPLERS: u32 = 1;
    pub const CONVERT_PUSH_CONSTANTS_SIZE: u32 = 96;
    pub const NUM_CAS_PIPELINES: usize = 2;

    pub fn new() -> Self { Self::default() }
    pub fn GetInstance() -> *mut GSDeviceVK { g_gs_device_get() }
    pub fn GetVulkanInstance(&self) -> VkInstance { self.instance }
    pub fn GetPhysicalDevice(&self) -> VkPhysicalDevice { self.physical_device }
    pub fn GetDevice(&self) -> VkDevice { self.device }
    pub fn GetAllocator(&self) -> VmaAllocator { self.allocator }
    pub fn GetGraphicsQueueFamilyIndex(&self) -> u32 { self.graphics_queue_family_index }
    pub fn GetPresentQueueFamilyIndex(&self) -> u32 { self.present_queue_family_index }
    pub fn GetDeviceProperties(&self) -> &VkPhysicalDeviceProperties { &self.device_properties }
    pub fn GetOptionalExtensions(&self) -> &OptionalExtensions { &self.optional_extensions }
    pub fn UseFeedbackLoopLayout(&self) -> bool {
        self.optional_extensions.vk_ext_attachment_feedback_loop_layout && !self.optional_extensions.vk_ext_rasterization_order_attachment_access
    }
    pub fn IsDeviceNVIDIA(&self) -> bool { self.device_properties.vendorID == 0x10DE }
    pub fn IsDeviceAMD(&self) -> bool { self.device_properties.vendorID == 0x1002 }
    pub fn GetBufferCopyOffsetAlignment(&self) -> u32 { self.device_properties.limits.optimalBufferCopyOffsetAlignment as u32 }
    pub fn GetBufferCopyRowPitchAlignment(&self) -> u32 { self.device_properties.limits.optimalBufferCopyRowPitchAlignment as u32 }
    pub fn GetCurrentCommandBuffer(&self) -> VkCommandBuffer { self.current_command_buffer }
    pub fn GetTextureUploadBuffer(&mut self) -> &mut VKStreamBuffer { &mut self.texture_stream_buffer }
    pub fn GetCurrentCommandBufferFence(&self) -> VkFence { self.frame_resources[self.current_frame as usize].fence }
    pub fn GetCurrentFenceCounter(&self) -> u64 { self.frame_resources[self.current_frame as usize].fence_counter }
    pub fn GetCompletedFenceCounter(&self) -> u64 { self.completed_fence_counter }

    pub fn GetRenderPass(&mut self, _cf: VkFormat, _df: VkFormat, _clo: VkAttachmentLoadOp, _cst: VkAttachmentStoreOp, _dlo: VkAttachmentLoadOp, _dst: VkAttachmentStoreOp, _slo: VkAttachmentLoadOp, _sst: VkAttachmentStoreOp, _fbl: bool, _dsp: bool) -> VkRenderPass { ptr::null_mut() }
    pub fn GetRenderPassForRestarting(&mut self, _p: VkRenderPass) -> VkRenderPass { ptr::null_mut() }
    pub fn GetCurrentInitCommandBuffer(&mut self) -> VkCommandBuffer { self.frame_resources[self.current_frame as usize].command_buffers[0] }
    pub fn AllocatePersistentDescriptorSet(&mut self, _l: VkDescriptorSetLayout) -> VkDescriptorSet { ptr::null_mut() }
    pub fn FreePersistentDescriptorSet(&mut self, _s: VkDescriptorSet) {}
    pub fn DeferBufferDestruction(&mut self, _b: VkBuffer, _a: VmaAllocation) {}
    pub fn DeferFramebufferDestruction(&mut self, _f: VkFramebuffer) {}
    pub fn DeferImageDestruction(&mut self, _i: VkImage, _a: VmaAllocation) {}
    pub fn DeferImageViewDestruction(&mut self, _v: VkImageView) {}
    pub fn WaitForFenceCounter(&mut self, _c: u64) {}
    pub fn WaitForGPUIdle(&mut self) {}

    pub fn EnumerateGPUs(_i: VkInstance) -> Vec<(VkPhysicalDevice, GSAdapterInfo)> { Vec::new() }
    pub fn EnumerateGPUs_static() -> Vec<(VkPhysicalDevice, GSAdapterInfo)> { Vec::new() }
    pub fn GetAdapterInfo() -> Vec<GSAdapterInfo> { Vec::new() }
    pub fn IsSuitableDefaultRenderer() -> bool { false }

    pub fn CreateVulkanInstance(_wi: WindowInfo, _oe: &mut OptionalExtensions, _du: bool, _vl: bool) -> VkInstance { ptr::null_mut() }
    pub fn SelectInstanceExtensions(_l: &mut Vec<String>, _wi: WindowInfo, _oe: &mut OptionalExtensions, _du: bool) -> bool { false }
    pub fn SelectDeviceExtensions(&mut self, _l: &mut Vec<String>, _s: bool) -> bool { false }
    pub fn SelectDeviceFeatures(&mut self) -> bool { false }
    pub fn CreateDevice(&mut self, _s: VkSurfaceKHR, _vl: bool) -> bool { false }
    pub fn ProcessDeviceExtensions(&mut self) -> bool { false }
    pub fn CreateAllocator(&mut self) -> bool { false }
    pub fn CreateCommandBuffers(&mut self) -> bool { false }
    pub fn CreateGlobalDescriptorPool(&mut self) -> bool { false }
    pub fn CreateCachedRenderPass(&mut self, _k: u32) -> VkRenderPass { ptr::null_mut() }
    pub fn CommandBufferCompleted(&mut self, _i: u32) {}
    pub fn ActivateCommandBuffer(&mut self, _i: u32) {}
    pub fn ScanForCommandBufferCompletion(&mut self) {}
    pub fn WaitForCommandBufferCompletion(&mut self, _i: u32) {}
    pub fn InitSpinResources(&mut self) -> bool { false }
    pub fn DestroySpinResources(&mut self) {}
    pub fn WaitForSpinCompletion(&mut self, _i: u32) {}
    pub fn SpinCommandCompleted(&mut self, _i: u32) {}
    pub fn SubmitSpinCommand(&mut self, _i: u32, _c: u32) {}
    pub fn CalibrateSpinTimestamp(&mut self) {}
    pub fn GetCPUTimestamp(&self) -> u64 { 0 }
    pub fn AllocatePreinitializedGPUBuffer(&mut self, _s: u32, _b: *mut VkBuffer, _a: *mut VmaAllocation, _u: u32, _f: Box<dyn FnOnce(*mut c_void)>) -> bool { false }
    pub fn SubmitCommandBuffer(&mut self, _s: *mut VKSwapChain) {}
    pub fn MoveToNextCommandBuffer(&mut self) {}
    pub fn EnableDebugUtils(&mut self) -> bool { false }
    pub fn DisableDebugUtils(&mut self) {}
    pub fn DebugMessengerCallback(_s: u32, _t: u32, _d: *const VkDebugUtilsMessengerCallbackDataEXT, _u: *mut c_void) -> u32 { 0 }
    pub fn GetWaitType(_w: bool, _s: bool) -> i32 { 0 }
    pub fn ExecuteCommandBuffer_wait(&mut self, _w: i32) {}
    pub fn ExecuteCommandBuffer(&mut self, _w: bool) {}
    pub fn ExecuteCommandBufferVa(&mut self, _w: bool, _r: &str) {}
    pub fn ExecuteCommandBufferAndRestartRenderPass(&mut self, _w: bool, _r: &str) {}
    pub fn ExecuteCommandBufferAndRestartPresent(&mut self, _w: bool, _r: &str) {}
    pub fn ExecuteCommandBufferForReadback(&mut self) {}
    pub fn InvalidateCachedState(&mut self) {}
    pub fn SetIndexBuffer(&mut self, _b: VkBuffer) {}
    pub fn SetBlendConstants(&mut self, _c: u8) {}
    pub fn SetLineWidth(&mut self, _w: f32) {}
    pub fn PSSetUnorderedAccess(&mut self, _rt: *mut GSTexture, _ds: *mut GSTexture, _wrt: bool, _wds: bool) {}
    pub fn PSSetShaderResource(&mut self, _i: i32, _sr: *mut GSTexture, _cs: bool, _t: ResourceType) {}
    pub fn PSSetSampler(&mut self, _sel: GSHWDrawConfig::SamplerSelector) {}
    pub fn SetUtilityTexture(&mut self, _t: *mut GSTexture, _s: VkSampler) {}
    pub fn SetUtilityPushConstants(&mut self, _d: *const c_void, _s: u32) {}
    pub fn UnbindTexture(&mut self, _t: *mut GSTextureVK) {}
    pub fn InRenderPass(&self) -> bool { !self.current_render_pass.is_null() }
    pub fn BeginRenderPass(&mut self, _rp: VkRenderPass, _r: GSVector4i) {}
    pub fn BeginClearRenderPass(&mut self, _rp: VkRenderPass, _r: GSVector4i, _cv: *const VkClearValue, _cnt: u32) {}
    pub fn BeginClearRenderPassColor(&mut self, _rp: VkRenderPass, _r: GSVector4i, _c: u32) {}
    pub fn BeginClearRenderPassDepth(&mut self, _rp: VkRenderPass, _r: GSVector4i, _d: f32, _s: u8) {}
    pub fn EndRenderPass(&mut self) {}
    pub fn SetViewport(&mut self, _v: VkViewport) {}
    pub fn SetScissor(&mut self, _s: GSVector4i) {}
    pub fn SetPipeline(&mut self, _p: VkPipeline) {}
    pub fn SetInitialState(&mut self, _c: VkCommandBuffer) {}
    pub fn ApplyBaseState(&mut self, _f: u32, _c: VkCommandBuffer) {}
    pub fn ApplyTFXState(&mut self, _a: bool) -> bool { false }
    pub fn ApplyUtilityState(&mut self, _a: bool) -> bool { false }
    pub fn SetVSConstantBuffer(&mut self, _cb: GSHWDrawConfig::VSConstantBuffer) {}
    pub fn SetPSConstantBuffer(&mut self, _cb: GSHWDrawConfig::PSConstantBuffer) {}
    pub fn SetVSPushConstants(&mut self, _bv: u32, _bi: u32, _f: bool) {}
    pub fn SetupDATE(&mut self, _rt: *mut GSTexture, _ds: *mut GSTexture, _d: SetDATM, _b: GSVector4i) {}
    pub fn SetupPrimitiveTrackingDATE(&mut self, _c: &mut GSHWDrawConfig::GSHWDrawConfig) -> *mut GSTextureVK { ptr::null_mut() }
    pub fn RenderHW(&mut self, _c: &mut GSHWDrawConfig::GSHWDrawConfig) {}
    pub fn UpdateHWPipelineSelector(&mut self, _c: &GSHWDrawConfig::GSHWDrawConfig, _p: &mut PipelineSelector) {}
    pub fn UploadHWDrawVerticesAndIndices(&mut self, _c: &GSHWDrawConfig::GSHWDrawConfig) {}
    pub fn GetColorBufferFeedbackBarrier(&self, _rt: *mut GSTextureVK) -> VkImageMemoryBarrier { unsafe { zeroed() } }
    pub fn GetDepthStencilBufferFeedbackBarrier(&self, _ds: *mut GSTextureVK) -> VkImageMemoryBarrier { unsafe { zeroed() } }
    pub fn GetFeedbackBarrierDependencyFlags(&self) -> VkDependencyFlags { 0 }
    pub fn SendHWDraw(&mut self, _c: &GSHWDrawConfig::GSHWDrawConfig, _rt: *mut GSTextureVK, _ds: *mut GSTextureVK, _o: bool, _f: bool) {}
    pub fn GetTFXRenderPass(&self, rt: bool, ds: bool, _cc: bool, _st: bool, _fbl: bool, _dsp: bool, _opa: u32, _opb: u32) -> VkRenderPass { self.tfx_render_pass[rt as usize][ds as usize][0][0][0][0][0][0] }
    pub fn GetPointSampler(&self) -> VkSampler { self.point_sampler }
    pub fn GetLinearSampler(&self) -> VkSampler { self.linear_sampler }
    pub fn GetConvertPipeline(&self, _s: u32) -> VkPipeline { ptr::null_mut() }
    pub fn GetTFXVertexShader(&mut self, _s: GSHWDrawConfig::VSSelector) -> VkShaderModule { ptr::null_mut() }
    pub fn GetTFXFragmentShader(&mut self, _s: GSHWDrawConfig::PSSelector) -> VkShaderModule { ptr::null_mut() }
    pub fn CreateTFXPipeline(&mut self, _p: PipelineSelector) -> VkPipeline { ptr::null_mut() }
    pub fn GetTFXPipeline(&mut self, _p: PipelineSelector) -> VkPipeline { ptr::null_mut() }
    pub fn GetUtilityVertexShader(&self, _s: &str, _m: Option<&str>) -> VkShaderModule { ptr::null_mut() }
    pub fn GetUtilityFragmentShader(&self, _s: &str, _m: Option<&str>) -> VkShaderModule { ptr::null_mut() }
    pub fn CreateDeviceAndSwapChain(&mut self) -> bool { false }
    pub fn CheckFeatures(&mut self) -> bool { false }
    pub fn CreateNullTexture(&mut self) -> bool { false }
    pub fn CreateBuffers(&mut self) -> bool { false }
    pub fn CreatePipelineLayouts(&mut self) -> bool { false }
    pub fn CreateRenderPasses(&mut self) -> bool { false }
    pub fn CompileConvertPipelines(&mut self) -> bool { false }
    pub fn CompilePresentPipelines(&mut self) -> bool { false }
    pub fn CompileInterlacePipelines(&mut self) -> bool { false }
    pub fn CompileMergePipelines(&mut self) -> bool { false }
    pub fn CompilePostProcessingPipelines(&mut self) -> bool { false }
    pub fn CompileCASPipelines(&mut self) -> bool { false }
    pub fn CompileImGuiPipeline(&mut self) -> bool { false }
    pub fn RenderImGui2(&mut self) {}
    pub fn RenderBlankFrame(&mut self) {}
    pub fn DoCAS(&mut self, _s: *mut GSTexture, _d: *mut GSTexture, _so: bool, _c: [u32; NUM_CAS_CONSTANTS]) -> bool { false }
    pub fn DestroyResources(&mut self) {}
    pub fn InitializeState(&mut self) {}
    pub fn CreatePersistentDescriptorSets(&mut self) -> bool { false }
    pub fn BindDrawPipeline(&mut self, _p: PipelineSelector) -> bool { false }
    pub fn ClearSamplerCache(&mut self) {}
    pub fn LookupNativeFormat(&self, f: GSTextureFormatEnum) -> VkFormat { match f { GSTextureFormatEnum::Color => VK_FORMAT_R8G8B8A8_UNORM, _ => VK_FORMAT_UNDEFINED } }
    pub fn UpdateImGuiTextures(&mut self) {}

    // Direct vertex/index uploads
    pub fn IASetVertexBuffer(&mut self, _v: *const c_void, _stride: usize, _count: usize, _align: usize) {}
    pub fn IASetIndexBuffer(&mut self, _i: *const c_void, _count: usize) {}
    pub fn VSSetIndexBuffer(&mut self, _i: *const c_void, _count: usize) {}
    pub fn UploadIndices(&mut self, _b: &mut VKStreamBuffer, _i: *const c_void, _count: usize) {}

    // Stretch rect and blits
    pub fn CopyRect(&mut self, _s: *mut GSTexture, _d: *mut GSTexture, _r: GSVector4i, _x: u32, _y: u32) {}
    pub fn PresentRect(&mut self, _s: *mut GSTexture, _sr: GSVector4, _d: *mut GSTexture, _dr: GSVector4, _sh: PresentShader, _t: f32, _f: Filter) {}
    pub fn DrawMultiStretchRects(&mut self, _rects: *const MultiStretchRect, _n: u32, _d: *mut GSTexture, _sh: ShaderConvertSelector) {}
    pub fn DoMultiStretchRects(&mut self, _rects: *const MultiStretchRect, _n: u32, _d: *mut GSTextureVK, _sh: ShaderConvertSelector) {}
    pub fn BeginRenderPassForStretchRect(&mut self, _d: *mut GSTextureVK, _a: GSVector4i, _b: GSVector4i, _al: bool) {}
    pub fn DoStretchRect(&mut self, _s: *mut GSTexture, _sr: GSVector4, _d: *mut GSTexture, _dr: GSVector4, _sh: ShaderConvertSelector, _f: Filter) {}
    pub fn DoStretchRect2(&mut self, _s: *mut GSTexture, _sr: GSVector4, _dr: GSVector4, _sh: PresentShader, _f: Filter) {}
    pub fn DoStretchRect3(&mut self, _s: *mut GSTextureVK, _sr: GSVector4, _d: *mut GSTextureVK, _dr: GSVector4, _p: VkPipeline, _f: Filter, _al: bool) {}
    pub fn DrawStretchRect(&mut self, _s: GSVector4, _d: GSVector4, _ds: GSVector2i) {}
    pub fn BlitRect(&mut self, _s: *mut GSTexture, _sr: GSVector4i, _sl: u32, _d: *mut GSTexture, _dr: GSVector4i, _dl: u32, _f: Filter) {}
    pub fn UpdateCLUTTexture(&mut self, _s: *mut GSTexture, _sc: f32, _ox: u32, _oy: u32, _d: *mut GSTexture, _do: u32, _ds: u32) {}
    pub fn ConvertToIndexedTexture(&mut self, _s: *mut GSTexture, _sc: f32, _ox: u32, _oy: u32, _sbw: u32, _spsm: u32, _d: *mut GSTexture, _dbw: u32, _dpsm: u32) {}
    pub fn FilteredDownsampleTexture(&mut self, _s: *mut GSTexture, _d: *mut GSTexture, _f: u32, _cm: GSVector2i, _dr: GSVector4) {}
    pub fn DoMerge(&mut self, _s: [*mut GSTexture; 3], _sr: *mut GSVector4, _d: *mut GSTexture, _dr: *mut GSVector4, _p: GSRegPMODE, _e: GSRegEXTBUF, _c: u32, _f: Filter) {}
    pub fn DoInterlace(&mut self, _s: *mut GSTexture, _sr: GSVector4, _d: *mut GSTexture, _dr: GSVector4, _sh: ShaderInterlace, _f: Filter, _cb: InterlaceConstantBuffer) {}
    pub fn DoShadeBoost(&mut self, _s: *mut GSTexture, _d: *mut GSTexture, _p: [f32; 4]) {}
    pub fn DoFXAA(&mut self, _s: *mut GSTexture, _d: *mut GSTexture) {}

    pub fn OMSetRenderTargets(&mut self, _rt: *mut GSTexture, _ds: *mut GSTexture, _s: GSVector4i, _fbl: FeedbackLoopFlag, _vs: GSVector2i) {}
    pub fn GetSampler(&mut self, _sel: GSHWDrawConfig::SamplerSelector) -> VkSampler { ptr::null_mut() }

    pub fn DrawPrimitive(&mut self) {}
    pub fn DrawIndexedPrimitive(&mut self) {}
    pub fn DrawIndexedPrimitive_off(&mut self, _o: i32, _c: i32) {}
    pub fn DrawIndexedPrimitiveVSExpand(&mut self, _o: i32, _c: i32, _vi: bool, _ve: i32) {}
    pub fn Draw1(&mut self, _c: &GSHWDrawConfig::GSHWDrawConfig, _o: i32, _cnt: i32) {}
    pub fn Draw2(&mut self, _c: &GSHWDrawConfig::GSHWDrawConfig) {}
    pub fn GetColorBufferFeedbackBarrier_impl(&self, _rt: *mut GSTextureVK) -> VkImageMemoryBarrier { unsafe { zeroed() } }
    pub fn GetDepthStencilBufferFeedbackBarrier_impl(&self, _ds: *mut GSTextureVK) -> VkImageMemoryBarrier { unsafe { zeroed() } }
    pub fn SendHWDraw_impl(&mut self, _c: &GSHWDrawConfig::GSHWDrawConfig, _rt: *mut GSTextureVK, _ds: *mut GSTextureVK, _o: bool, _f: bool) {}

    // Frame primitives
    pub fn DrawPrimitive_cmd(&mut self) {}
    pub fn DrawIndexedPrimitive_cmd(&mut self) {}
    pub fn GetResourceLayout(_t: ResourceType) -> i32 { 0 }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ResourceType { SRV, UAV }
pub const TFX_DESCRIPTOR_SET_UBO: u32 = 0;
pub const TFX_DESCRIPTOR_SET_TEXTURES: u32 = 1;
pub const NUM_TFX_DESCRIPTOR_SETS: u32 = 2;
pub const TFX_TEXTURE_TEXTURE: u32 = 0;
pub const TFX_TEXTURE_PALETTE: u32 = 1;
pub const TFX_TEXTURE_RT: u32 = 2;
pub const TFX_TEXTURE_PRIMID: u32 = 3;
pub const TFX_TEXTURE_DEPTH: u32 = 4;
pub const TFX_TEXTURE_RT_ROV: u32 = 5;
pub const TFX_TEXTURE_DEPTH_ROV: u32 = 6;
pub const NUM_TFX_TEXTURES: usize = 7;
pub const EXPAND_BUFFER_SIZE: u32 = 16 * 1024 * 1024;
pub const VERTEX_BUFFER_SIZE: u32 = 32 * 1024 * 1024;
pub const INDEX_BUFFER_SIZE: u32 = 16 * 1024 * 1024;
pub const VERTEX_UNIFORM_BUFFER_SIZE: u32 = 8 * 1024 * 1024;
pub const FRAGMENT_UNIFORM_BUFFER_SIZE: u32 = 8 * 1024 * 1024;
pub const TEXTURE_BUFFER_SIZE: u32 = 64 * 1024 * 1024;

pub const DIRTY_FLAG_TFX_TEXTURE_0: u32 = 1 << 0;
pub const DIRTY_FLAG_TFX_UBO: u32 = 1 << 7;
pub const DIRTY_FLAG_UTILITY_TEXTURE: u32 = 1 << 8;
pub const DIRTY_FLAG_BLEND_CONSTANTS: u32 = 1 << 9;
pub const DIRTY_FLAG_LINE_WIDTH: u32 = 1 << 10;
pub const DIRTY_FLAG_INDEX_BUFFER: u32 = 1 << 11;
pub const DIRTY_FLAG_VIEWPORT: u32 = 1 << 12;
pub const DIRTY_FLAG_SCISSOR: u32 = 1 << 13;
pub const DIRTY_FLAG_PIPELINE: u32 = 1 << 14;
pub const DIRTY_FLAG_VS_CONSTANT_BUFFER: u32 = 1 << 15;
pub const DIRTY_FLAG_PS_CONSTANT_BUFFER: u32 = 1 << 16;
pub const DIRTY_FLAG_VS_PUSH_CONSTANTS: u32 = 1 << 17;
pub const DIRTY_BASE_STATE: u32 = DIRTY_FLAG_INDEX_BUFFER | DIRTY_FLAG_PIPELINE | DIRTY_FLAG_VIEWPORT | DIRTY_FLAG_SCISSOR | DIRTY_FLAG_BLEND_CONSTANTS | DIRTY_FLAG_LINE_WIDTH;
pub const DIRTY_TFX_STATE: u32 = DIRTY_BASE_STATE | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 7);
pub const DIRTY_UTILITY_STATE: u32 = DIRTY_BASE_STATE | DIRTY_FLAG_UTILITY_TEXTURE;
pub const DIRTY_CONSTANT_BUFFER_STATE: u32 = DIRTY_FLAG_VS_CONSTANT_BUFFER | DIRTY_FLAG_PS_CONSTANT_BUFFER | DIRTY_FLAG_VS_PUSH_CONSTANTS;
pub const ALL_DIRTY_STATE: u32 = DIRTY_BASE_STATE | DIRTY_TFX_STATE | DIRTY_UTILITY_STATE | DIRTY_CONSTANT_BUFFER_STATE;

pub const DIRTY_FLAG_TFX_TEXTURE_TEX: u32 = DIRTY_FLAG_TFX_TEXTURE_0;
pub const DIRTY_FLAG_TFX_TEXTURE_PALETTE: u32 = DIRTY_FLAG_TFX_TEXTURE_0 << 1;
pub const DIRTY_FLAG_TFX_TEXTURE_RT: u32 = DIRTY_FLAG_TFX_TEXTURE_0 << 2;
pub const DIRTY_FLAG_TFX_TEXTURE_PRIMID: u32 = DIRTY_FLAG_TFX_TEXTURE_0 << 3;
pub const DIRTY_FLAG_TFX_TEXTURE_DEPTH: u32 = DIRTY_FLAG_TFX_TEXTURE_0 << 4;
pub const DIRTY_FLAG_TFX_TEXTURE_RT_ROV: u32 = DIRTY_FLAG_TFX_TEXTURE_0 << 5;
pub const DIRTY_FLAG_TFX_TEXTURE_DEPTH_ROV: u32 = DIRTY_FLAG_TFX_TEXTURE_0 << 6;
pub const DIRTY_FLAG_TFX_TEXTURES: u32 = DIRTY_FLAG_TFX_TEXTURE_TEX | DIRTY_FLAG_TFX_TEXTURE_PALETTE | DIRTY_FLAG_TFX_TEXTURE_RT | DIRTY_FLAG_TFX_TEXTURE_PRIMID | DIRTY_FLAG_TFX_TEXTURE_DEPTH | DIRTY_FLAG_TFX_TEXTURE_RT_ROV | DIRTY_FLAG_TFX_TEXTURE_DEPTH_ROV;

pub const FeedbackLoopFlag_None: u8 = 0;
pub const FeedbackLoopFlag_ReadAndWriteRT: u8 = 1;
pub const FeedbackLoopFlag_ReadDepth: u8 = 2;
pub const FeedbackLoopFlag_ReadAndWriteDepth: u8 = 4;

pub const PipelineLayout_Undefined: i32 = 0;
pub const PipelineLayout_TFX: i32 = 1;
pub const PipelineLayout_Utility: i32 = 2;

pub fn GetLoadOpForTexture(_t: *mut GSTextureVK) -> u32 { 0 }

#[derive(Clone, Default, Debug)]
pub struct GSAdapterInfo { pub name: String, pub max_texture_size: u32, pub max_upscale_multiplier: f32 }

// =====================================================================
//  Section 21.  GSDevice trait implementation for GSDeviceVK.
// =====================================================================

impl GSDevice for GSDeviceVK {
    fn GetRenderAPI(&self) -> RenderAPI { RenderAPI::Vulkan }
    fn HasSurface(&self) -> bool { self.swap_chain.is_some() }
    fn GetFeatures(&self) -> FeatureSupport { self.features }
    fn GetWindowWidth(&self) -> i32 { self.swap_chain.as_ref().map(|s| s.GetWidth() as i32).unwrap_or(0) }
    fn GetWindowHeight(&self) -> i32 { self.swap_chain.as_ref().map(|s| s.GetHeight() as i32).unwrap_or(0) }
}

// =====================================================================
//  Section 22.  Helper used to populate the C++ test interface.
// =====================================================================

impl Default for GSDeviceVK {
    fn default() -> Self {
        Self {
            instance: ptr::null_mut(),
            physical_device: ptr::null_mut(),
            device: ptr::null_mut(),
            allocator: ptr::null_mut(),
            current_command_buffer: ptr::null_mut(),
            global_descriptor_pool: ptr::null_mut(),
            graphics_queue: ptr::null_mut(),
            present_queue: ptr::null_mut(),
            graphics_queue_family_index: 0,
            present_queue_family_index: 0,
            spin_manager: ReadbackSpinManager,
            spin_queue: ptr::null_mut(),
            spin_descriptor_set_layout: ptr::null_mut(),
            spin_pipeline_layout: ptr::null_mut(),
            spin_pipeline: ptr::null_mut(),
            spin_buffer: ptr::null_mut(),
            spin_buffer_allocation: ptr::null_mut(),
            spin_descriptor_set: ptr::null_mut(),
            spin_resources: unsafe { zeroed() },
            queryperfcounter_to_ns: 0.0,
            spin_timestamp_scale: 0.0,
            spin_timestamp_offset: 0.0,
            spin_queue_family_index: 0,
            command_buffer_render_passes: 0,
            spin_timer: 0,
            spinning_supported: false,
            spin_queue_is_graphics_queue: false,
            spin_buffer_initialized: false,
            timestamp_query_pool: ptr::null_mut(),
            accumulated_gpu_time: 0.0,
            gpu_timing_enabled: false,
            gpu_timing_supported: false,
            wants_new_timestamp_calibration: false,
            calibrated_timestamp_type: VK_TIME_DOMAIN_DEVICE_EXT,
            frame_resources: unsafe { zeroed() },
            next_fence_counter: 1,
            completed_fence_counter: 0,
            current_frame: 0,
            last_submit_failed: false,
            render_pass_cache: BTreeMap::new(),
            debug_messenger_callback: ptr::null_mut(),
            device_features: unsafe { zeroed() },
            device_properties: unsafe { zeroed() },
            device_driver_properties: unsafe { zeroed() },
            optional_extensions: OptionalExtensions::default(),
            swap_chain: None,
            resize_requested: false,
            is_presenting: false,
            vblank_wait_supported: false,
            vblank_wait: false,
            vblank_skipped: false,
            vblank_realtime: false,
            vsync_mode: GSVsyncModeKind::FIFO,
            allow_present_throttle: false,
            utility_ds_layout: ptr::null_mut(),
            utility_pipeline_layout: ptr::null_mut(),
            tfx_ubo_ds_layout: ptr::null_mut(),
            tfx_texture_ds_layout: ptr::null_mut(),
            tfx_pipeline_layout: ptr::null_mut(),
            vertex_stream_buffer: VKStreamBuffer::default(),
            index_stream_buffer: VKStreamBuffer::default(),
            expand_index_stream_buffer: VKStreamBuffer::default(),
            vertex_uniform_stream_buffer: VKStreamBuffer::default(),
            fragment_uniform_stream_buffer: VKStreamBuffer::default(),
            texture_stream_buffer: VKStreamBuffer::default(),
            expand_index_buffer: ptr::null_mut(),
            expand_index_buffer_allocation: ptr::null_mut(),
            point_sampler: ptr::null_mut(),
            linear_sampler: ptr::null_mut(),
            samplers: HashMap::new(),
            convert: Vec::new(),
            present: [ptr::null_mut(); 2],
            merge: [ptr::null_mut(); 2],
            interlace: [ptr::null_mut(); NUM_INTERLACE_SHADERS],
            colclip_setup_pipelines: [[ptr::null_mut(); 2]; 2],
            colclip_finish_pipelines: [[ptr::null_mut(); 2]; 2],
            primid_image_setup_render_passes: [[ptr::null_mut(); 2]; 2],
            primid_image_setup_pipelines: [[ptr::null_mut(); 4]; 2],
            fxaa_pipeline: ptr::null_mut(),
            shadeboost_pipeline: ptr::null_mut(),
            tfx_vertex_shaders: HashMap::new(),
            tfx_fragment_shaders: HashMap::new(),
            tfx_pipelines: HashMap::new(),
            utility_color_render_pass_load: ptr::null_mut(),
            utility_color_render_pass_clear: ptr::null_mut(),
            utility_color_render_pass_discard: ptr::null_mut(),
            utility_depth_render_pass_load: ptr::null_mut(),
            utility_depth_render_pass_clear: ptr::null_mut(),
            utility_depth_render_pass_discard: ptr::null_mut(),
            date_setup_render_pass: ptr::null_mut(),
            swap_chain_render_pass: ptr::null_mut(),
            tfx_render_pass: unsafe { zeroed() },
            cas_ds_layout: ptr::null_mut(),
            cas_pipeline_layout: ptr::null_mut(),
            cas_pipelines: [ptr::null_mut(); 2],
            imgui_pipeline: ptr::null_mut(),
            vs_cb_cache: GSHWDrawConfig::VSConstantBuffer,
            ps_cb_cache: GSHWDrawConfig::PSConstantBuffer { FogColor_AREF: GSHWDrawConfig::PSConstantBufferF { a: 0.0, _pad: [0.0; 3] } },
            vs_pc_cache: GSHWDrawConfig::VSPushConstants { base_vertex: 0, base_index: 0 },
            tfx_source: String::new(),
            features: FeatureSupport::default(),
            dirty_flags: 0,
            current_framebuffer_feedback_loop: FeedbackLoopFlag::None,
            warned_slow_spin: false,
            index_buffer: ptr::null_mut(),
            current_render_target: ptr::null_mut(),
            current_depth_target: ptr::null_mut(),
            current_framebuffer: ptr::null_mut(),
            current_render_pass: ptr::null_mut(),
            current_render_pass_area: GSVector4i::ZERO,
            scissor: GSVector4i::ZERO,
            viewport: VkViewport { x: 0.0, y: 0.0, width: 1.0, height: 1.0, minDepth: 0.0, maxDepth: 1.0 },
            current_line_width: 1.0,
            blend_constant_color: 0,
            tfx_textures: [ptr::null_mut(); 7],
            tfx_sampler: ptr::null_mut(),
            tfx_sampler_sel: 0,
            tfx_ubo_descriptor_set: ptr::null_mut(),
            tfx_texture_descriptor_set: ptr::null_mut(),
            tfx_rt_descriptor_set: ptr::null_mut(),
            tfx_dynamic_offsets: [0; 2],
            utility_texture: ptr::null(),
            utility_sampler: ptr::null_mut(),
            utility_descriptor_set: ptr::null_mut(),
            current_pipeline_layout: PipelineLayout_Undefined,
            current_pipeline: ptr::null_mut(),
            null_texture: None,
            null_framebuffer: ptr::null_mut(),
            pipeline_selector: PipelineSelector::default(),
            shader_cache: VKShaderCache::new(),
            console: Console::new(),
        }
    }
}
