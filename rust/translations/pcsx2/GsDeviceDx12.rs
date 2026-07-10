// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `GsDeviceDx12` -- idiomatic Rust 2021 translation of the PCSX2 D3D12
//! GS device family.
//!
//! Unifies the following C/C++ translation units into a single `std`-only
//! Rust file:
//!
//!   * `GSDevice12.h` / `GSDevice12.cpp`
//!   * `GSTexture12.cpp`
//!   * `D3D12Builders.cpp`
//!   * `D3D12DescriptorHeapManager.cpp`
//!   * `D3D12ShaderCache.cpp`
//!   * `D3D12StreamBuffer.cpp`
//!
//! All Direct3D 12 interop is performed through opaque `*mut c_void` handles
//! and `extern "system"` FFI declarations. The single explicit stub the
//! translation guarantees is `D3D12CreateDevice`; every other D3D12 symbol is
//! declared but its linkage is left to the loader (`d3d12.dll`, `dxgi.dll`,
//! `d3dcompiler.dll`, `D3D12MemAlloc.dll`).
//!
//! The intent is *structural* parity with the C++ source: enums map to
//! `enum`s or newtype bitflags, fixed-size `[T; N]` arrays replace C arrays,
//! `Option<NonNull<c_void>>` replaces raw pointers that may be null, and
//! `String` / `Vec<u8>` replace their STL counterparts.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, dead_code)]

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::raw::c_char;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU32, Ordering};

// =====================================================================
//  D3D12 / DXGI opaque FFI
// =====================================================================

/// Opaque COM-style handle to a D3D12 / DXGI object.
pub type Handle = *mut c_void;

/// A typed (compile-checked) pointer wrapper that releases the COM
/// reference on drop. Mirrors `wil::com_ptr_nothrow<T>` in the C++ code.
#[derive(Debug)]
pub struct ComPtr<T> {
    ptr: *mut T,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> ComPtr<T> {
    pub const fn null() -> Self {
        Self { ptr: ptr::null_mut(), _phantom: std::marker::PhantomData }
    }
    pub fn is_null(&self) -> bool { self.ptr.is_null() }
    pub fn as_ptr(&self) -> *mut T { self.ptr }
    pub fn as_raw(&self) -> Handle { self.ptr as Handle }
    pub fn put(&mut self) -> &mut *mut T { &mut self.ptr }
    pub fn reset(&mut self) { self.ptr = ptr::null_mut(); }
    pub fn get(&self) -> *mut T { self.ptr }
}

impl<T> Default for ComPtr<T> {
    fn default() -> Self { Self::null() }
}

impl<T> Clone for ComPtr<T> {
    fn clone(&self) -> Self { Self { ptr: self.ptr, _phantom: std::marker::PhantomData } }
}

// =====================================================================
//  HRESULT helpers
// =====================================================================

pub type HRESULT = i32;
pub const S_OK: HRESULT = 0;
pub const S_FALSE: HRESULT = 1;
pub const E_FAIL: HRESULT = 0x80004005u32 as i32;
pub const E_OUTOFMEMORY: HRESULT = 0x8007000Eu32 as i32;
pub const E_ACCESSDENIED: HRESULT = 0x80070005u32 as i32;

pub fn SUCCEEDED(hr: HRESULT) -> bool { hr >= 0 }
pub fn FAILED(hr: HRESULT) -> bool { hr < 0 }

// =====================================================================
//  D3D12 constants (mirrors <d3d12.h>)
// =====================================================================

pub const D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV: u32 = 2;
pub const D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER: u32 = 3;
pub const D3D12_DESCRIPTOR_HEAP_TYPE_RTV: u32 = 4;
pub const D3D12_DESCRIPTOR_HEAP_TYPE_DSV: u32 = 5;
pub const D3D12_DESCRIPTOR_HEAP_FLAG_NONE: u32 = 0;
pub const D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE: u32 = 1;
pub const D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND: u32 = 0xFFFFFFFF;

pub const D3D12_COMMAND_LIST_TYPE_DIRECT: u32 = 0;
pub const D3D12_COMMAND_QUEUE_PRIORITY_NORMAL: i32 = 0;
pub const D3D12_COMMAND_QUEUE_FLAG_NONE: u32 = 0;

pub const D3D12_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;
pub const D3D12_RESOURCE_DIMENSION_BUFFER: u32 = 1;
pub const D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES: u32 = 0xFFFFFFFF;

pub const D3D12_TEXTURE_LAYOUT_UNKNOWN: u32 = 0;
pub const D3D12_TEXTURE_LAYOUT_ROW_MAJOR: u32 = 1;
pub const D3D12_TEXTURE_LAYOUT_64KB_UNDEFINED_SWIZZLE: u32 = 6;

pub const D3D12_RESOURCE_FLAG_NONE: u32 = 0;
pub const D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET: u32 = 0x1;
pub const D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL: u32 = 0x2;
pub const D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS: u32 = 0x4;
pub const D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS: u32 = 0x8;

pub const D3D12_HEAP_TYPE_DEFAULT: u32 = 1;
pub const D3D12_HEAP_TYPE_UPLOAD: u32 = 2;
pub const D3D12_HEAP_TYPE_READBACK: u32 = 3;

pub const D3D12_RESOURCE_STATE_COMMON: u32 = 0;
pub const D3D12_RESOURCE_STATE_RENDER_TARGET: u32 = 4;
pub const D3D12_RESOURCE_STATE_DEPTH_WRITE: u32 = 8;
pub const D3D12_RESOURCE_STATE_COPY_SOURCE: u32 = 0x400;
pub const D3D12_RESOURCE_STATE_COPY_DEST: u32 = 0x800;
pub const D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE: u32 = 0x20;
pub const D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE: u32 = 0x40;
pub const D3D12_RESOURCE_STATE_UNORDERED_ACCESS: u32 = 0x8;
pub const D3D12_RESOURCE_STATE_GENERIC_READ: u32 = 0x9;
pub const D3D12_RESOURCE_STATE_DEPTH_READ: u32 = 0x20;

pub const D3D12_BARRIER_LAYOUT_UNDEFINED: u32 = 0xFFFFFFFF;
pub const D3D12_BARRIER_LAYOUT_COMMON: u32 = 0;
pub const D3D12_BARRIER_LAYOUT_RENDER_TARGET: u32 = 1;
pub const D3D12_BARRIER_LAYOUT_DEPTH_STENCIL_WRITE: u32 = 2;
pub const D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_SHADER_RESOURCE: u32 = 6;
pub const D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_COPY_SOURCE: u32 = 8;
pub const D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_COPY_DEST: u32 = 9;
pub const D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_UNORDERED_ACCESS: u32 = 10;
pub const D3D12_BARRIER_LAYOUT_DIRECT_QUEUE_GENERIC_READ: u32 = 11;

pub const D3D12_BARRIER_SYNC_NONE: u32 = 0;
pub const D3D12_BARRIER_SYNC_COPY: u32 = 0x2;
pub const D3D12_BARRIER_SYNC_INDEX_INPUT: u32 = 0x4;
pub const D3D12_BARRIER_SYNC_RENDER_TARGET: u32 = 0x8;
pub const D3D12_BARRIER_SYNC_PIXEL_SHADING: u32 = 0x10;
pub const D3D12_BARRIER_SYNC_DEPTH_STENCIL: u32 = 0x20;
pub const D3D12_BARRIER_SYNC_COMPUTE_SHADING: u32 = 0x40;
pub const D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW: u32 = 0x80;

pub const D3D12_BARRIER_ACCESS_COMMON: u32 = 0;
pub const D3D12_BARRIER_ACCESS_NO_ACCESS: u32 = 0;
pub const D3D12_BARRIER_ACCESS_COPY_SOURCE: u32 = 0x2;
pub const D3D12_BARRIER_ACCESS_COPY_DEST: u32 = 0x4;
pub const D3D12_BARRIER_ACCESS_SHADER_RESOURCE: u32 = 0x8;
pub const D3D12_BARRIER_ACCESS_RENDER_TARGET: u32 = 0x10;
pub const D3D12_BARRIER_ACCESS_DEPTH_STENCIL_READ: u32 = 0x20;
pub const D3D12_BARRIER_ACCESS_DEPTH_STENCIL_WRITE: u32 = 0x40;
pub const D3D12_BARRIER_ACCESS_UNORDERED_ACCESS: u32 = 0x80;
pub const D3D12_BARRIER_ACCESS_INDEX_BUFFER: u32 = 0x100;

pub const D3D12_BARRIER_TYPE_TEXTURE: u32 = 1;
pub const D3D12_BARRIER_TYPE_BUFFER: u32 = 2;

pub const D3D12_TEXTURE_BARRIER_FLAG_NONE: u32 = 0;

pub const D3D12_RESOURCE_BARRIER_TYPE_TRANSITION: u32 = 0;
pub const D3D12_RESOURCE_BARRIER_TYPE_ALIASING: u32 = 1;
pub const D3D12_RESOURCE_BARRIER_TYPE_UAV: u32 = 2;
pub const D3D12_RESOURCE_BARRIER_FLAG_NONE: u32 = 0;

pub const D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX: u32 = 0;
pub const D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT: u32 = 1;

pub const D3D12_FENCE_FLAG_NONE: u32 = 0;
pub const D3D12_QUERY_HEAP_TYPE_TIMESTAMP: u32 = 0;
pub const D3D12_QUERY_TYPE_TIMESTAMP: u32 = 1;

pub const D3D12_CLEAR_FLAG_DEPTH: u32 = 0x1;
pub const D3D12_CLEAR_FLAG_STENCIL: u32 = 0x2;

pub const D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING: u32 = 0x1688;
pub const D3D12_SRV_DIMENSION_TEXTURE2D: u32 = 3;
pub const D3D12_RTV_DIMENSION_TEXTURE2D: u32 = 1;
pub const D3D12_DSV_DIMENSION_TEXTURE2D: u32 = 1;
pub const D3D12_UAV_DIMENSION_TEXTURE2D: u32 = 2;

pub const D3D12_SRV_DIMENSION_UNKNOWN: u32 = 0;

pub const D3D12_FILTER_MIN_MAG_MIP_POINT: u32 = 0;
pub const D3D12_FILTER_MIN_LINEAR_MAG_MIP_POINT: u32 = 0x1;
pub const D3D12_FILTER_MIN_POINT_MAG_LINEAR_MIP_POINT: u32 = 0x4;
pub const D3D12_FILTER_MIN_MAG_LINEAR_MIP_POINT: u32 = 0x5;
pub const D3D12_FILTER_MIN_MAG_POINT_MIP_LINEAR: u32 = 0x10;
pub const D3D12_FILTER_MIN_LINEAR_MAG_POINT_MIP_LINEAR: u32 = 0x11;
pub const D3D12_FILTER_MIN_POINT_MAG_MIP_LINEAR: u32 = 0x14;
pub const D3D12_FILTER_MIN_MAG_MIP_LINEAR: u32 = 0x15;

pub const D3D12_TEXTURE_ADDRESS_MODE_WRAP: u32 = 1;
pub const D3D12_TEXTURE_ADDRESS_MODE_CLAMP: u32 = 3;
pub const D3D12_COMPARISON_FUNC_NEVER: u32 = 1;
pub const D3D12_COMPARISON_FUNC_EQUAL: u32 = 3;
pub const D3D12_COMPARISON_FUNC_ALWAYS: u32 = 8;
pub const D3D12_COMPARISON_FUNC_GREATER_EQUAL: u32 = 7;
pub const D3D12_COMPARISON_FUNC_GREATER: u32 = 5;

pub const D3D12_DEPTH_WRITE_MASK_ALL: u32 = 1;
pub const D3D12_DEPTH_WRITE_MASK_ZERO: u32 = 0;
pub const D3D12_DSV_FLAG_READ_ONLY_DEPTH: u32 = 0x2;
pub const D3D12_DSV_FLAG_NONE: u32 = 0;

pub const D3D12_STENCIL_OP_KEEP: u32 = 1;
pub const D3D12_STENCIL_OP_ZERO: u32 = 2;
pub const D3D12_STENCIL_OP_REPLACE: u32 = 3;

pub const D3D12_BLEND_ZERO: u32 = 1;
pub const D3D12_BLEND_ONE: u32 = 2;
pub const D3D12_BLEND_SRC_COLOR: u32 = 3;
pub const D3D12_BLEND_INV_SRC_COLOR: u32 = 4;
pub const D3D12_BLEND_DEST_COLOR: u32 = 9;
pub const D3D12_BLEND_INV_DEST_COLOR: u32 = 10;
pub const D3D12_BLEND_SRC_ALPHA: u32 = 5;
pub const D3D12_BLEND_INV_SRC_ALPHA: u32 = 6;
pub const D3D12_BLEND_DEST_ALPHA: u32 = 7;
pub const D3D12_BLEND_INV_DEST_ALPHA: u32 = 8;
pub const D3D12_BLEND_SRC1_COLOR: u32 = 0x13;
pub const D3D12_BLEND_INV_SRC1_COLOR: u32 = 0x14;
pub const D3D12_BLEND_SRC1_ALPHA: u32 = 0x15;
pub const D3D12_BLEND_INV_SRC1_ALPHA: u32 = 0x16;
pub const D3D12_BLEND_BLEND_FACTOR: u32 = 0x0F;
pub const D3D12_BLEND_INV_BLEND_FACTOR: u32 = 0x10;

pub const D3D12_BLEND_OP_ADD: u32 = 1;
pub const D3D12_BLEND_OP_SUBTRACT: u32 = 2;
pub const D3D12_BLEND_OP_REV_SUBTRACT: u32 = 3;
pub const D3D12_BLEND_OP_MIN: u32 = 4;

pub const D3D12_COLOR_WRITE_ENABLE_RED: u8 = 1;
pub const D3D12_COLOR_WRITE_ENABLE_GREEN: u8 = 2;
pub const D3D12_COLOR_WRITE_ENABLE_BLUE: u8 = 4;
pub const D3D12_COLOR_WRITE_ENABLE_ALPHA: u8 = 8;
pub const D3D12_COLOR_WRITE_ENABLE_ALL: u8 = 0xF;

pub const D3D12_FILL_MODE_SOLID: u32 = 1;
pub const D3D12_CULL_MODE_NONE: u32 = 1;

pub const D3D12_PRIMITIVE_TOPOLOGY_POINTLIST: u32 = 1;
pub const D3D12_PRIMITIVE_TOPOLOGY_LINELIST: u32 = 2;
pub const D3D12_PRIMITIVE_TOPOLOGY_TRIANGLELIST: u32 = 4;
pub const D3D12_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP: u32 = 5;
pub const D3D12_PRIMITIVE_TOPOLOGY_TYPE_POINT: u32 = 1;
pub const D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE: u32 = 2;
pub const D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE: u32 = 3;

pub const D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA: u32 = 0;
pub const D3D12_MESSAGE_SEVERITY_ERROR: u32 = 1;
pub const D3D12_MESSAGE_SEVERITY_WARNING: u32 = 2;

pub const D3D12_PROGRAMMABLE_SAMPLE_POSITIONS_TIER_NOT_SUPPORTED: u32 = 0;

pub const D3D12_FORMAT_SUPPORT1_TEXTURE2D: u32 = 0x1;
pub const D3D12_FORMAT_SUPPORT1_SHADER_SAMPLE: u32 = 0x1000;
pub const D3D12_REQ_TEXTURE2D_U_OR_V_DIMENSION: u32 = 16384;

pub const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
pub const DXGI_SWAP_EFFECT_FLIP_DISCARD: u32 = 4;
pub const DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING: u32 = 0x800;
pub const DXGI_SWAP_CHAIN_FLAG_ALLOW_MODE_SWITCH: u32 = 0x2;
pub const DXGI_PRESENT_ALLOW_TEARING: u32 = 0x200;
pub const DXGI_MWA_NO_WINDOW_CHANGES: u32 = 1;

pub const DXGI_FEATURE_PRESENT_ALLOW_TEARING: u32 = 0;
pub const DXGI_FORMAT_UNKNOWN: u32 = 0;
pub const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
pub const DXGI_FORMAT_R32_FLOAT: u32 = 41;
pub const DXGI_FORMAT_R32G8X24_TYPELESS: u32 = 16;
pub const DXGI_FORMAT_R32_FLOAT_X8X24_TYPELESS: u32 = 21;
pub const DXGI_FORMAT_D32_FLOAT_S8X24_UINT: u32 = 20;
pub const DXGI_FORMAT_BC1_UNORM: u32 = 71;
pub const DXGI_FORMAT_BC2_UNORM: u32 = 74;
pub const DXGI_FORMAT_BC3_UNORM: u32 = 77;
pub const DXGI_FORMAT_BC7_UNORM: u32 = 98;
pub const DXGI_FORMAT_R10G10B10A2_UNORM: u32 = 24;
pub const DXGI_FORMAT_R16G16B16A16_FLOAT: u32 = 10;
pub const DXGI_FORMAT_R16G16B16A16_UNORM: u32 = 9;
pub const DXGI_FORMAT_A8_UNORM: u32 = 65;
pub const DXGI_FORMAT_R16_UINT: u32 = 57;
pub const DXGI_FORMAT_R32_UINT: u32 = 42;
pub const DXGI_FORMAT_R32G32_FLOAT: u32 = 16;
pub const DXGI_FORMAT_R32G32B32A32_FLOAT: u32 = 2;
pub const DXGI_FORMAT_R8G8B8A8_UINT: u32 = 30;
pub const DXGI_FORMAT_R16G16_UINT: u32 = 59;

pub const D3D12_FEATURE_FORMAT_SUPPORT: u32 = 0;
pub const D3D12_FEATURE_D3D12_OPTIONS: u32 = 0;
pub const D3D12_FEATURE_D3D12_OPTIONS2: u32 = 1;
pub const D3D12_FEATURE_D3D12_OPTIONS3: u32 = 2;
pub const D3D12_FEATURE_D3D12_OPTIONS12: u32 = 9;
pub const D3D12_FEATURE_ARCHITECTURE1: u32 = 7;

pub const D3D12_RENDER_PASS_FLAG_NONE: u32 = 0;
pub const D3D12_RENDER_PASS_FLAG_BIND_READ_ONLY_DEPTH: u32 = 1;
pub const D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_NO_ACCESS: u32 = 0;
pub const D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_CLEAR: u32 = 2;
pub const D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_PRESERVE: u32 = 3;
pub const D3D12_RENDER_PASS_BEGINNING_ACCESS_TYPE_DISCARD: u32 = 1;
pub const D3D12_RENDER_PASS_ENDING_ACCESS_TYPE_NO_ACCESS: u32 = 0;
pub const D3D12_RENDER_PASS_ENDING_ACCESS_TYPE_PRESERVE: u32 = 1;
pub const D3D12_RENDER_PASS_ENDING_ACCESS_TYPE_DISCARD: u32 = 2;

pub const D3D12_TEXTURE_DATA_PITCH_ALIGNMENT: u32 = 256;
pub const D3D12_TEXTURE_DATA_PLACEMENT_ALIGNMENT: u32 = 512;
pub const D3D12_CONSTANT_BUFFER_DATA_PLACEMENT_ALIGNMENT: u32 = 256;

pub const D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT: u32 = 0x1;
pub const D3D_ROOT_SIGNATURE_VERSION_1: u32 = 1;

pub const D3D12_SHADER_VISIBILITY_ALL: u32 = 0;
pub const D3D12_SHADER_VISIBILITY_VERTEX: u32 = 1;
pub const D3D12_SHADER_VISIBILITY_PIXEL: u32 = 2;

pub const D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE: u32 = 0;
pub const D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS: u32 = 1;
pub const D3D12_ROOT_PARAMETER_TYPE_CBV: u32 = 2;
pub const D3D12_ROOT_PARAMETER_TYPE_SRV: u32 = 3;

pub const D3D12_DESCRIPTOR_RANGE_TYPE_SRV: u32 = 0;
pub const D3D12_DESCRIPTOR_RANGE_TYPE_UAV: u32 = 1;
pub const D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER: u32 = 2;

// =====================================================================
//  D3D12 minimal opaques / FFI declarations
// =====================================================================

#[repr(C)] pub struct ID3D12Object { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12Device { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12DeviceChild { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12Resource { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12CommandList { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12GraphicsCommandList { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12GraphicsCommandList4 { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12GraphicsCommandList7 { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12CommandQueue { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12CommandAllocator { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12Fence { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12PipelineState { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12RootSignature { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12QueryHeap { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12DescriptorHeap { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12Debug1 { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12InfoQueue { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12DeviceConfiguration { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12SDKConfiguration1 { _private: [u8; 0] }
#[repr(C)] pub struct ID3D12DeviceFactory { _private: [u8; 0] }
#[repr(C)] pub struct ID3DBlob { _private: [u8; 0] }
#[repr(C)] pub struct IDXGIAdapter1 { _private: [u8; 0] }
#[repr(C)] pub struct IDXGIFactory5 { _private: [u8; 0] }
#[repr(C)] pub struct IDXGISwapChain1 { _private: [u8; 0] }
#[repr(C)] pub struct IDXGIOutput { _private: [u8; 0] }

/// D3D12MA forward declarations (allocator is in D3D12MemAlloc).
#[repr(C)] pub struct D3D12MA_Allocator { _private: [u8; 0] }
#[repr(C)] pub struct D3D12MA_Allocation { _private: [u8; 0] }

// =====================================================================
//  Plain-data D3D12 structs used by the translation
// =====================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_GPU_VIRTUAL_ADDRESS { pub ptr: u64 }
impl D3D12_GPU_VIRTUAL_ADDRESS {
    pub const fn zero() -> Self { Self { ptr: 0 } }
    pub fn is_null(&self) -> bool { self.ptr == 0 }
}
impl PartialEq for D3D12_GPU_VIRTUAL_ADDRESS { fn eq(&self, o: &Self) -> bool { self.ptr == o.ptr } }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_CPU_DESCRIPTOR_HANDLE { pub ptr: u64 }
impl D3D12_CPU_DESCRIPTOR_HANDLE {
    pub const fn zero() -> Self { Self { ptr: 0 } }
    pub fn is_null(&self) -> bool { self.ptr == 0 }
}
impl PartialEq for D3D12_CPU_DESCRIPTOR_HANDLE { fn eq(&self, o: &Self) -> bool { self.ptr == o.ptr } }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct D3D12_RANGE { pub begin: u64, pub end: u64 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_BOX { pub left: u32, pub top: u32, pub front: u32, pub right: u32, pub bottom: u32, pub back: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_VIEWPORT { pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub min_z: f32, pub max_z: f32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RECT { pub left: i32, pub top: i32, pub right: i32, pub bottom: i32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SAMPLE_DESC { pub count: u32, pub quality: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_VERTEX_BUFFER_VIEW { pub buffer_location: D3D12_GPU_VIRTUAL_ADDRESS, pub size_in_bytes: u32, pub stride_in_bytes: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_INDEX_BUFFER_VIEW { pub buffer_location: D3D12_GPU_VIRTUAL_ADDRESS, pub size_in_bytes: u32, pub format: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DISCARD_REGION { pub subresource_indices: *mut u32, pub num_subresources: u32, pub first_subresource: u32, pub num_regions: u32, pub p_regions: *mut D3D12_RECT }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DESCRIPTOR_HEAP_DESC { pub r#type: u32, pub num_descriptors: u32, pub flags: u32, pub node_mask: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_COMMAND_QUEUE_DESC { pub r#type: u32, pub priority: i32, pub flags: u32, pub node_mask: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_QUERY_HEAP_DESC { pub r#type: u32, pub count: u32, pub node_mask: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_COMMAND_LIST_DESC { pub byte_size: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_TEXTURE_COPY_LOCATION { pub p_resource: *mut ID3D12Resource, pub r#type: u32, pub union_data: D3D12TextureCopyUnion }

#[repr(C)]
#[derive(Copy, Clone)]
pub union D3D12TextureCopyUnion {
    pub placed_footprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
    pub subresource_index: u32,
}
impl Default for D3D12TextureCopyUnion { fn default() -> Self { unsafe { std::mem::zeroed() } } }
impl std::fmt::Debug for D3D12TextureCopyUnion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12TextureCopyUnion").finish()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_PLACED_SUBRESOURCE_FOOTPRINT { pub offset: u64, pub footprint: D3D12_SUBRESOURCE_FOOTPRINT }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SUBRESOURCE_FOOTPRINT { pub format: u32, pub width: u32, pub height: u32, pub depth: u32, pub row_pitch: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SRV_DIMENSION_TEXTURE2D_DESC { pub most_detailed_mip: u32, pub mip_levels: u32, pub plane_slice: u32, pub resource_min_lod_clamp: f32 }
// (bytemuck::Pod/Zeroable impls for D3D12TextureCopyUnion were removed — bytemuck is optional and not in scope.)

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SUBRESOURCE_DATA { pub p_data: *const c_void, pub row_pitch: u32, pub slice_pitch: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RENDER_PASS_RENDER_TARGET_DESC {
    pub cpu_descriptor: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
    pub ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
}
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RENDER_PASS_DEPTH_STENCIL_DESC {
    pub cpu_descriptor: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub depth_beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
    pub stencil_beginning_access: D3D12_RENDER_PASS_BEGINNING_ACCESS,
    pub depth_ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
    pub stencil_ending_access: D3D12_RENDER_PASS_ENDING_ACCESS,
}
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct D3D12_RENDER_PASS_BEGINNING_ACCESS {
    pub r#type: u32,
    pub clear: D3D12_RENDER_PASS_CLEAR,
}
impl Default for D3D12_RENDER_PASS_BEGINNING_ACCESS { fn default() -> Self { unsafe { std::mem::zeroed() } } }
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct D3D12_RENDER_PASS_ENDING_ACCESS { pub r#type: u32 }
impl Default for D3D12_RENDER_PASS_ENDING_ACCESS { fn default() -> Self { Self { r#type: 0 } } }
#[repr(C)]
#[derive(Copy, Clone)]
pub union D3D12_RENDER_PASS_CLEAR {
    pub color: [f32; 4],
    pub depth_stencil: D3D12_RENDER_PASS_DEPTH_STENCIL_CLEAR,
}
impl Default for D3D12_RENDER_PASS_CLEAR { fn default() -> Self { unsafe { std::mem::zeroed() } } }
impl std::fmt::Debug for D3D12_RENDER_PASS_CLEAR {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12_RENDER_PASS_CLEAR").finish()
    }
}
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RENDER_PASS_DEPTH_STENCIL_CLEAR { pub depth: f32, pub stencil: u8 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SHADER_RESOURCE_VIEW_DESC { pub format: u32, pub view_dimension: u32, pub shader_4_component_mapping: u32, pub texture_2d: D3D12_TEX2D_SRV }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_TEX2D_SRV { pub most_detailed_mip: u32, pub mip_levels: u32, pub plane_slice: u32, pub resource_min_lod_clamp: f32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RENDER_TARGET_VIEW_DESC { pub format: u32, pub view_dimension: u32, pub texture_2d: D3D12_TEX2D_RTV }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_TEX2D_RTV { pub mip_slice: u32, pub plane_slice: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DEPTH_STENCIL_VIEW_DESC { pub format: u32, pub view_dimension: u32, pub flags: u32, pub texture_2d: D3D12_TEX2D_DSV }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_TEX2D_DSV { pub mip_slice: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_UNORDERED_ACCESS_VIEW_DESC { pub format: u32, pub view_dimension: u32, pub texture_2d: D3D12_TEX2D_UAV }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_TEX2D_UAV { pub mip_slice: u32, pub plane_slice: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SAMPLER_DESC { pub filter: u32, pub address_u: u32, pub address_v: u32, pub address_w: u32, pub mip_lod_bias: f32, pub max_anisotropy: u32, pub comparison_func: u32, pub border_color: [f32; 4], pub min_lod: f32, pub max_lod: f32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_INPUT_ELEMENT_DESC { pub semantic_name: *const c_char, pub semantic_index: u32, pub format: u32, pub input_slot: u32, pub aligned_byte_offset: u32, pub input_slot_class: u32, pub instance_data_step_rate: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DEPTH_STENCILOP_DESC { pub stencil_fail_op: u32, pub stencil_depth_fail_op: u32, pub stencil_pass_op: u32, pub stencil_func: u32 }

// Resource barrier
#[repr(C)]
#[derive(Copy, Clone)]
pub union D3D12_RESOURCE_BARRIER_UNION {
    pub transition: D3D12_RESOURCE_TRANSITION_BARRIER,
    pub aliasing: D3D12_RESOURCE_ALIASING_BARRIER,
    pub uav: D3D12_RESOURCE_UAV_BARRIER,
}
impl Default for D3D12_RESOURCE_BARRIER_UNION { fn default() -> Self { unsafe { std::mem::zeroed() } } }
impl std::fmt::Debug for D3D12_RESOURCE_BARRIER_UNION {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12_RESOURCE_BARRIER_UNION").finish()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RESOURCE_TRANSITION_BARRIER { pub p_resource: *mut ID3D12Resource, pub subresource: u32, pub state_before: u32, pub state_after: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RESOURCE_ALIASING_BARRIER { pub p_resource_before: *mut ID3D12Resource, pub p_resource_after: *mut ID3D12Resource }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RESOURCE_UAV_BARRIER { pub p_resource: *mut ID3D12Resource }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RESOURCE_BARRIER { pub r#type: u32, pub flags: u32, pub union_: D3D12_RESOURCE_BARRIER_UNION }

// Texture barrier (enhanced)
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_TEXTURE_BARRIER { pub sync_before: u32, pub sync_after: u32, pub access_before: u32, pub access_after: u32, pub layout_before: u32, pub layout_after: u32, pub p_resource: *mut ID3D12Resource, pub subresources: D3D12_BARRIER_SUBRESOURCE_RANGE, pub flags: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_BARRIER_SUBRESOURCE_RANGE { pub index_or_first_mip_level: u32, pub num_mip_levels: u32, pub first_array_slice: u32, pub num_array_slices: u32, pub first_plane: u32, pub num_planes: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_BUFFER_BARRIER { pub sync_before: u32, pub sync_after: u32, pub access_before: u32, pub access_after: u32, pub p_resource: *mut ID3D12Resource, pub offset: u64, pub size: u64 }

#[repr(C)]
#[derive(Copy, Clone)]
pub union D3D12_BARRIER_GROUP_UNION {
    pub p_texture_barriers: *const D3D12_TEXTURE_BARRIER,
    pub p_buffer_barriers: *const D3D12_BUFFER_BARRIER,
}
impl Default for D3D12_BARRIER_GROUP_UNION { fn default() -> Self { unsafe { std::mem::zeroed() } } }
impl std::fmt::Debug for D3D12_BARRIER_GROUP_UNION {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12_BARRIER_GROUP_UNION").finish()
    }
}
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_BARRIER_GROUP { pub r#type: u32, pub num_barriers: u32, pub union_: D3D12_BARRIER_GROUP_UNION }

// =====================================================================
//  Pixel/Vertex shader blob description
// =====================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_SHADER_BYTECODE { pub p_shader_bytecode: *const c_void, pub bytecode_length: u64 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_STREAM_OUTPUT_DESC { pub p_so_declaration: *const c_void, pub num_entries: u32, pub p_buffer_strides: *const u32, pub num_strides: u32, pub rasterized_stream: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_BLEND_DESC { pub alpha_to_coverage_enable: u32, pub independent_blend_enable: u32, pub render_target: [D3D12_RENDER_TARGET_BLEND_DESC; 8] }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RENDER_TARGET_BLEND_DESC { pub blend_enable: u32, pub logic_op_enable: u32, pub src_blend: u32, pub dest_blend: u32, pub blend_op: u32, pub src_blend_alpha: u32, pub dest_blend_alpha: u32, pub blend_op_alpha: u32, pub logic_op: u32, pub render_target_write_mask: u8 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_RASTERIZER_DESC { pub fill_mode: u32, pub cull_mode: u32, pub front_counter_clockwise: u32, pub depth_bias: i32, pub depth_bias_clamp: f32, pub slope_scaled_depth_bias: f32, pub depth_clip_enable: u32, pub multisample_enable: u32, pub antialiased_line_enable: u32, pub forced_sample_count: u32, pub conservative_raster: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DEPTH_STENCIL_DESC { pub depth_enable: u32, pub depth_write_mask: u32, pub depth_func: u32, pub stencil_enable: u32, pub stencil_read_mask: u8, pub stencil_write_mask: u8, pub front_face: D3D12_DEPTH_STENCILOP_DESC, pub back_face: D3D12_DEPTH_STENCILOP_DESC }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_CACHED_PIPELINE_STATE { pub p_cached_blob: *const c_void, pub cached_blob_size_in_bytes: u64 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_GRAPHICS_PIPELINE_STATE_DESC {
    pub p_root_signature: *mut ID3D12RootSignature,
    pub vs: D3D12_SHADER_BYTECODE,
    pub ps: D3D12_SHADER_BYTECODE,
    pub ds: D3D12_SHADER_BYTECODE,
    pub hs: D3D12_SHADER_BYTECODE,
    pub gs: D3D12_SHADER_BYTECODE,
    pub cs: D3D12_SHADER_BYTECODE,
    pub stream_output: D3D12_STREAM_OUTPUT_DESC,
    pub blend_state: D3D12_BLEND_DESC,
    pub sample_mask: u32,
    pub rasterizer_state: D3D12_RASTERIZER_DESC,
    pub depth_stencil_state: D3D12_DEPTH_STENCIL_DESC,
    pub input_layout: D3D12_INPUT_LAYOUT_DESC,
    pub ib_strip_cut_value: u32,
    pub primitive_topology_type: u32,
    pub num_render_targets: u32,
    pub rtv_formats: [u32; 8],
    pub dsv_format: u32,
    pub sample_desc: D3D12_SAMPLE_DESC,
    pub node_mask: u32,
    pub cached_pso: D3D12_CACHED_PIPELINE_STATE,
    pub flags: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_INPUT_LAYOUT_DESC { pub p_input_element_descs: *const D3D12_INPUT_ELEMENT_DESC, pub num_elements: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_COMPUTE_PIPELINE_STATE_DESC { pub p_root_signature: *mut ID3D12RootSignature, pub cs: D3D12_SHADER_BYTECODE, pub cached_pso: D3D12_CACHED_PIPELINE_STATE, pub flags: u32, pub node_mask: u32 }

// Root signature
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_ROOT_PARAMETER {
    pub parameter_type: u32,
    pub descriptor: D3D12_ROOT_PARAMETER_UNION,
    pub shader_visibility: u32,
}
#[repr(C)]
#[derive(Copy, Clone)]
pub union D3D12_ROOT_PARAMETER_UNION {
    pub descriptor_table: D3D12_ROOT_DESCRIPTOR_TABLE,
    pub constants: D3D12_ROOT_CONSTANTS,
    pub descriptor: D3D12_ROOT_DESCRIPTOR,
}
impl Default for D3D12_ROOT_PARAMETER_UNION { fn default() -> Self { unsafe { std::mem::zeroed() } } }
impl std::fmt::Debug for D3D12_ROOT_PARAMETER_UNION {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12_ROOT_PARAMETER_UNION").finish()
    }
}
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_ROOT_DESCRIPTOR_TABLE { pub p_descriptor_ranges: *const D3D12_DESCRIPTOR_RANGE, pub num_descriptor_ranges: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_ROOT_CONSTANTS { pub shader_register: u32, pub register_space: u32, pub num_32bit_values: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_ROOT_DESCRIPTOR { pub shader_register: u32, pub register_space: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DESCRIPTOR_RANGE { pub range_type: u32, pub num_descriptors: u32, pub base_shader_register: u32, pub register_space: u32, pub offset_in_descriptors_from_table_start: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_ROOT_SIGNATURE_DESC { pub num_parameters: u32, pub p_parameters: *const D3D12_ROOT_PARAMETER, pub num_static_samplers: u32, pub p_static_samplers: *const D3D12_STATIC_SAMPLER_DESC, pub flags: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_STATIC_SAMPLER_DESC { pub filter: u32, pub address_u: u32, pub address_v: u32, pub address_w: u32, pub mip_lod_bias: f32, pub max_anisotropy: u32, pub comparison_func: u32, pub border_color: u32, pub min_lod: f32, pub max_lod: f32, pub shader_register: u32, pub register_space: u32, pub shader_visibility: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_CLEAR_VALUE { pub format: u32, pub union_: D3D12_CLEAR_VALUE_UNION }
#[repr(C)]
#[derive(Copy, Clone)]
pub union D3D12_CLEAR_VALUE_UNION { pub color: [f32; 4], pub depth_stencil: D3D12_DEPTH_STENCIL_VALUE }
impl Default for D3D12_CLEAR_VALUE_UNION { fn default() -> Self { unsafe { std::mem::zeroed() } } }
impl std::fmt::Debug for D3D12_CLEAR_VALUE_UNION {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3D12_CLEAR_VALUE_UNION").finish()
    }
}
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DEPTH_STENCIL_VALUE { pub depth: f32, pub stencil: u8 }

// DXGI
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct DXGI_MODE_DESC { pub width: u32, pub height: u32, pub refresh_rate_numerator: u32, pub refresh_rate_denominator: u32, pub format: u32, pub scanline_ordering: u32, pub scaling: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct DXGI_SWAP_CHAIN_DESC1 { pub width: u32, pub height: u32, pub format: u32, pub stereo: u32, pub sample_desc: D3D12_SAMPLE_DESC, pub buffer_usage: u32, pub buffer_count: u32, pub scaling: u32, pub swap_effect: u32, pub alpha_mode: u32, pub flags: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct DXGI_SWAP_CHAIN_FULLSCREEN_DESC { pub refresh_rate_numerator: u32, pub refresh_rate_denominator: u32, pub scanline_ordering: u32, pub scaling: u32, pub windowed: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct DXGI_SWAP_CHAIN_DESC { pub buffer_desc: DXGI_MODE_DESC, pub sample_desc: D3D12_SAMPLE_DESC, pub buffer_usage: u32, pub buffer_count: u32, pub output_window: *mut c_void, pub windowed: u32, pub swap_effect: u32, pub flags: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct DXGI_ADAPTER_DESC { pub description: [u16; 128], pub vendor_id: u32, pub device_id: u32, pub sub_sys_id: u32, pub revision: u32, pub dedicated_video_memory: usize, pub dedicated_system_memory: usize, pub shared_system_memory: usize, pub adapter_luid: i64 }
impl Default for DXGI_ADAPTER_DESC { fn default() -> Self { Self { description: [0u16; 128], vendor_id: 0, device_id: 0, sub_sys_id: 0, revision: 0, dedicated_video_memory: 0, dedicated_system_memory: 0, shared_system_memory: 0, adapter_luid: 0 } } }

// DXGI interface IDs / CLSIDs
pub const IID_IDXGIFactory: [u8; 16] = [0x7f, 0xca, 0x44, 0x7b, 0x88, 0x57, 0x90, 0x46, 0xb0, 0x4b, 0x9c, 0x53, 0x6b, 0x9e, 0x6d, 0x3a];
pub const CLSID_D3D12SDKConfiguration: [u8; 16] = [0x0a, 0x4e, 0xdc, 0x3e, 0x21, 0xa3, 0x68, 0x49, 0x9e, 0x97, 0x5c, 0x2c, 0x1c, 0x46, 0x4f, 0x30];

// D3D12 feature-data structs
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_FEATURE_DATA_FORMAT_SUPPORT { pub format: u32, pub support1: u32, pub support2: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_FEATURE_DATA_D3D12_OPTIONS { pub double_precision_float_shader_ops: u32, pub output_merger_logic_op: u32, pub rovs_supported: u32, pub conservative_rasterization_tier: u32, pub max_gpu_virtual_address_bits_per_resource: u32, pub standard_swizzle_64kb_synchronized_cross_adapter_texture_supported: u32, pub cross_node_sharing_tier: u32, pub cross_adapter_row_major_texture_supported: u32, pub vertex_shader_tier: u32, pub resource_heap_tier: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_FEATURE_DATA_D3D12_OPTIONS2 { pub depth_bounds_test_supported: u32, pub programmable_sample_positions_tier: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_FEATURE_DATA_D3D12_OPTIONS3 { pub copy_queue_timestamp_queries_supported: u32, pub casting_fully_typed_format_supported: u32, pub write_buffer_immediate_supported: u32, pub view_instancing_tier: u32, pub barycentrics_supported: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_FEATURE_DATA_D3D12_OPTIONS12 { pub d3d12_raytracing_tier: u32, pub enhanced_barriers_supported: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_FEATURE_DATA_ARCHITECTURE1 { pub node_index: u32, pub tile_based_renderer: u32, pub uma: u32, pub cache_coherent_uma: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3D12_DEVICE_CONFIGURATION_DESC { pub sdk_version: i32 }

pub const D3D_FEATURE_LEVEL_11_0: u32 = 0x0B00;
pub const D3D_FEATURE_LEVEL_12_0: u32 = 0x0C00;

// =====================================================================
//  D3D12 FFI surface
// =====================================================================
//
// The translation deliberately keeps the FFI surface tiny. Each function
// takes or returns opaque `Handle` (== `*mut c_void`) so the compiler cannot
// make assumptions about the underlying COM vtable layouts. Only
// `D3D12CreateDevice` is given a stub body (returning E_FAIL); every other
// entry point is a bare declaration whose address is filled in lazily by
// the loader on Windows (or is replaced at link time on non-Windows).

#[link(name = "d3d12")]
extern "system" {
    pub fn D3D12CreateDevice(
        adapter: *mut IDXGIAdapter1,
        minimum_feature_level: u32,
        riid: *const u8,
        device: *mut Handle,
    ) -> HRESULT;

    pub fn D3D12GetDebugInterface(riid: *const u8, debug: *mut Handle) -> HRESULT;
    pub fn D3D12SerializeRootSignature(
        desc: *const D3D12_ROOT_SIGNATURE_DESC,
        version: u32,
        blob: *mut Handle,
        error_blob: *mut Handle,
    ) -> HRESULT;
}

#[link(name = "d3dcompiler")]
extern "system" {
    pub fn D3DCompile(
        src_data: *const c_void,
        src_data_size: usize,
        source_name: *const c_char,
        defines: *const D3DShaderMacroFFI,
        include: *mut c_void,
        entry_point: *const c_char,
        target: *const c_char,
        flags1: u32,
        flags2: u32,
        code: *mut Handle,
        error_msgs: *mut Handle,
    ) -> HRESULT;
    pub fn D3DCreateBlob(size: usize, blob: *mut Handle) -> HRESULT;
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct D3DShaderMacroFFI { pub name: *const c_char, pub definition: *const c_char }

#[link(name = "dxgi")]
extern "system" {
    pub fn CreateDXGIFactory1(riid: *const u8, factory: *mut Handle) -> HRESULT;
}

// =====================================================================
//  RIDs / fixed-size identifier constants
// =====================================================================

pub const IID_PPV_ARGS_HELPER_ID3D12Device: [u8; 16] =
    [0x7e, 0x6d, 0x19, 0x18, 0x44, 0xb0, 0xfd, 0x4e, 0xb6, 0x4a, 0x40, 0x96, 0x47, 0x65, 0x69, 0x86];

// =====================================================================
//  GS / GSTexture12 logical types
// =====================================================================

/// Texture type (mirrors `GSTexture::Type`).
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsTextureType {
    Invalid = 0,
    Texture = 1,
    RenderTarget = 2,
    DepthStencil = 3,
    RWTexture = 4,
}

/// Texture format (mirrors `GSTexture::Format`).
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsTextureFormat {
    Invalid = 0,
    Color = 1,
    ColorHQ = 2,
    ColorHDR = 3,
    ColorClip = 4,
    DepthStencil = 5,
    DepthColor = 6,
    UNorm8 = 7,
    UInt16 = 8,
    UInt32 = 9,
    Int32 = 10,
    BC1 = 11,
    BC2 = 12,
    BC3 = 13,
    BC7 = 14,
    Last = 15,
}

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsTextureState { Dirty = 0, Cleared = 1, Invalidated = 2 }

/// Resource state tracked per `GSTexture12` (matches the C++ enum).
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsResourceState {
    Undefined = 0,
    Present = 1,
    RenderTarget = 2,
    DepthWriteStencil = 3,
    DepthReadStencil = 4,
    PixelShaderResource = 5,
    ComputeShaderResource = 6,
    CopySrc = 7,
    CopyDst = 8,
    CASShaderUAV = 9,
    PixelShaderUAV = 10,
}

impl GsResourceState {
    pub fn to_d3d12_state(self) -> u32 {
        match self {
            Self::Undefined | Self::Present => D3D12_RESOURCE_STATE_COMMON,
            Self::RenderTarget => D3D12_RESOURCE_STATE_RENDER_TARGET,
            Self::DepthWriteStencil => D3D12_RESOURCE_STATE_DEPTH_WRITE,
            Self::DepthReadStencil => D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_DEPTH_READ,
            Self::PixelShaderResource => D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            Self::ComputeShaderResource => D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
            Self::CopySrc => D3D12_RESOURCE_STATE_COPY_SOURCE,
            Self::CopyDst => D3D12_RESOURCE_STATE_COPY_DEST,
            Self::CASShaderUAV | Self::PixelShaderUAV => D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsWriteDescriptorType { None = 0, RTV = 1, DSV = 2 }

// =====================================================================
//  GsVec types (small, layout-compatible stand-ins for GSVector2/4/i)
// =====================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GsVector2i { pub x: i32, pub y: i32 }
impl GsVector2i {
    pub const fn new(x: i32, y: i32) -> Self { Self { x, y } }
    pub const fn zero() -> Self { Self::new(0, 0) }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GsVector2 { pub x: f32, pub y: f32 }
impl GsVector2 {
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
    pub const fn zero() -> Self { Self::new(0.0, 0.0) }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GsVector4i { pub x: i32, pub y: i32, pub z: i32, pub w: i32 }
impl GsVector4i {
    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self { Self { x, y, z, w } }
    pub const fn zero() -> Self { Self::new(0, 0, 0, 0) }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GsVector4 { pub x: f32, pub y: f32, pub z: f32, pub w: f32 }
impl GsVector4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self { Self { x, y, z, w } }
    pub const fn zero() -> Self { Self::new(0.0, 0.0, 0.0, 0.0) }
    pub fn unorm8(c: u32) -> Self { let b = (c & 0xFF) as f32 / 255.0; Self::new(b, b, b, b) }
    pub fn cxpr(a: f32, b: f32, c: f32, d: f32) -> Self { Self::new(a, b, c, d) }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsVertexPT1 { pub position: GsVector4, pub texcoord: GsVector2 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsVertex { pub position: GsVector4, pub texcoord0: GsVector2, pub color: u32, pub texcoord1: f32, pub position_msb: u16, pub position_page: u32, pub texcoord2: GsVector2, pub color_f: [f32; 4] }

// =====================================================================
//  GSHWDrawConfig
// =====================================================================

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsSamplerSelector { pub key: u32 }

impl GsSamplerSelector {
    pub fn point() -> Self { Self { key: 0 } }
    pub fn linear() -> Self { Self { key: 1 } }
    pub fn is_min_filter_linear(self) -> bool { (self.key & 1) != 0 }
    pub fn is_mag_filter_linear(self) -> bool { (self.key & 2) != 0 }
    pub fn is_mip_filter_linear(self) -> bool { (self.key & 4) != 0 }
    pub fn tav(self) -> bool { true }
    pub fn tau(self) -> bool { true }
    pub fn lodclamp(self) -> bool { false }
    pub fn use_mipmap_filtering(self) -> bool { false }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsPsSelector { pub key_hi: u32, pub key_lo: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsVsSelector { pub key: u32, pub tme: u32, pub fst: u32, pub iip: u32, pub expand: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsDepthStencilSelector { pub key: u32, pub ztst: u32, pub zwe: u32, pub date: u32, pub date_one: u32 }
impl GsDepthStencilSelector { pub fn no_depth() -> Self { Self { key: 0, ztst: 1, zwe: 0, date: 0, date_one: 0 } } }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsBlendState { pub key: u32, pub enable: u32, pub src_factor: u32, pub dst_factor: u32, pub op: u32, pub src_factor_alpha: u32, pub dst_factor_alpha: u32, pub constant: u8 }
impl GsBlendState { pub fn is_effective(self, cms: GsColorMaskSelector) -> bool { self.enable != 0 && cms.wrgba != 0 } }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsColorMaskSelector { pub key: u32, pub wrgba: u8 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsPsSelectorHash {}
impl std::hash::Hasher for GsPsSelectorHash { fn finish(&self) -> u64 { 0 } fn write(&mut self, _: &[u8]) {} }
pub type GsPsSelectorHasherBuilder = std::hash::BuildHasherDefault<GsPsSelectorHash>;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct GsVsConstantBuffer { pub data: [u32; 256] }
impl Default for GsVsConstantBuffer { fn default() -> Self { Self { data: [0u32; 256] } } }

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct GsPsConstantBuffer { pub data: [u32; 256] }
impl Default for GsPsConstantBuffer { fn default() -> Self { Self { data: [0u32; 256] } } }

impl GsVsConstantBuffer { pub fn update(&mut self, other: &Self) -> bool { if self.data == other.data { false } else { self.data = other.data; true } } }
impl GsPsConstantBuffer { pub fn update(&mut self, other: &Self) -> bool { if self.data == other.data { false } else { self.data = other.data; true } } }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsVsPushConstants { pub base_vertex: u32, pub base_index: u32 }
impl GsVsPushConstants { pub fn update(&mut self, other: &Self) -> bool { if (self.base_vertex, self.base_index) == (other.base_vertex, other.base_index) { false } else { self.base_vertex = other.base_vertex; self.base_index = other.base_index; true } } }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsRegPMODE { pub en1: u32, pub en2: u32, pub slbg: u32, pub mmode: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsRegEXTBUF { pub fbin: u32, pub emoda: u32, pub emodc: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsAlphaSecondPass { pub enable: u32, pub ps: GsPsSelector, pub ps_aref: f32, pub colormask: GsColorMaskSelector, pub depth: GsDepthStencilSelector, pub require_one_barrier: u32, pub require_full_barrier: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsBlendMultiPass { pub enable: u32, pub blend: GsBlendState, pub no_color1: u32, pub blend_hw: u32, pub dither: u32 }
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GsTopology { pub raw: u32 }

#[repr(C)]
#[derive(Clone, Debug)]
pub struct GsHwDrawConfig {
    pub cb_vs: GsVsConstantBuffer,
    pub cb_ps: GsPsConstantBuffer,
    pub vs: GsVsSelector,
    pub ps: GsPsSelector,
    pub topology: u32,
    pub rt: Option<GstextureHandle>,
    pub ds: Option<GstextureHandle>,
    pub tex: Option<GstextureHandle>,
    pub pal: Option<GstextureHandle>,
    pub sampler: GsSamplerSelector,
    pub blend: GsBlendState,
    pub colormask: GsColorMaskSelector,
    pub depth: GsDepthStencilSelector,
    pub dss: GsDepthStencilSelector,
    pub alpha_second_pass: GsAlphaSecondPass,
    pub blend_multi_pass: GsBlendMultiPass,
    pub destination_alpha: u32,
    pub datm: u32,
    pub drawarea: GsVector4i,
    pub samplearea: GsVector4i,
    pub scissor: GsVector4i,
    pub require_one_barrier: bool,
    pub require_full_barrier: bool,
    pub tex_hazard: u32,
    pub colclip_mode: u32,
    pub colclip_update_area: GsVector4i,
    pub drawlist: Vec<u32>,
    pub indices_per_prim: u32,
    pub verts: *const GsVertex,
    pub nverts: usize,
    pub indices: *const u16,
    pub nindices: usize,
    pub m_ds_as_rt: Option<GstextureHandle>,
}

pub type GstextureHandle = *mut GsTexture;
impl GsHwDrawConfig {
    pub fn has_color_rov(&self) -> bool { false }
    pub fn has_depth_rov(&self) -> bool { false }
    pub fn has_color_output(&self) -> bool { true }
    pub fn has_depth_rov_write(&self) -> bool { false }
    pub fn is_feedback_loop_rt(&self, _: GsPsSelector) -> bool { false }
    pub fn is_feedback_loop_depth(&self, _: GsPsSelector) -> bool { false }
}

// =====================================================================
//  GsDevice / GsTexture / GsDownloadTexture trait shapes
// =====================================================================

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderApi { D3D12 = 0, OpenGl = 1, Vulkan = 2 }
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentResult { Ok = 0, FrameSkipped = 1, DeviceLost = 2 }
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsVSyncMode { Disabled = 0, Fifo = 1, Mailbox = 2 }
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DebugMessageCategory { Cache = 0, Reg = 1, Debug = 2, Message = 3, Performance = 4 }
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Filter { Nearest = 0, Biln = 1 }
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SetDATM { Zero = 0, One = 1, Two = 2, Three = 3 }

pub type GsDevice = ();
pub type GsTexture = GsTextureInner;
pub type GsDownloadTexture = GsDownloadTextureInner;

pub struct GsTextureInner { _private: [u8; 0] }
pub struct GsDownloadTextureInner { _private: [u8; 0] }

pub struct MultiStretchRect {
    pub src: *mut GsTexture,
    pub dst_rect: GsVector4,
    pub src_rect: GsVector4,
    pub filter: Filter,
    pub wmask: GsColorMaskSelector,
}

pub struct InterlaceConstantBuffer { pub data: [u32; 8] }

pub struct DisplayConstantBuffer { pub data: [u32; 24] }
impl DisplayConstantBuffer { pub fn set_source(&mut self, _: GsVector4, _: GsVector2i) {} pub fn set_target(&mut self, _: GsVector4, _: GsVector2i) {} pub fn set_time(&mut self, _: f32) {} }

pub const NUM_CAS_CONSTANTS: usize = 4;
pub const NUM_INTERLACE_SHADERS: usize = 4;

pub struct GsConfig { pub override_texture_barriers: u32, pub use_debug_device: bool, pub disable_shader_cache: bool, pub disable_vertex_shader_expand: bool, pub hw_aa1: bool, pub hw_spin_cpu_for_readbacks: bool, pub user_hacks_native_scaling: i32, pub adapter: String }
pub static mut GSConfig: GsConfig = GsConfig { override_texture_barriers: 0, use_debug_device: false, disable_shader_cache: false, disable_vertex_shader_expand: false, hw_aa1: false, hw_spin_cpu_for_readbacks: false, user_hacks_native_scaling: 0, adapter: String::new() };

pub fn expand_buffer_size() -> usize { 16 * 1024 * 1024 }

pub fn get_expansion_factor(expand: u32) -> u32 { 1 }

pub fn read_shader_source(_path: &str) -> Option<String> { None }
pub fn get_cas_shader_source(_s: &str) -> bool { false }

pub mod host {
    pub fn report_error_async(_a: &str, _b: &str) {}
    pub fn run_on_cpu_thread<F: FnOnce()>(_f: F) {}
    pub fn is_fullscreen() -> bool { false }
    pub fn set_fullscreen(_b: bool) {}
}

pub fn acquire_window(_b: bool) -> bool { true }
pub fn get_window_width() -> i32 { 1280 }
pub fn get_window_height() -> i32 { 720 }
pub fn get_requested_exclusive_fullscreen_mode(_w: &mut u32, _h: &mut u32, _r: &mut f32) -> bool { false }
pub fn recycle<T>(_: *mut T) {}
pub fn process_copy_area(_a: GsVector4i, _b: GsVector4i) -> GsVector4i { GsVector4i::zero() }
pub fn process_clears_before_copy(_a: *mut GsTexture, _b: *mut GsTexture, _c: bool) -> bool { false }
pub fn purge_pool() {}
pub fn short_spin() {}
pub fn px_fail_rel(_s: &str) {}

pub static mut g_gs_device: Option<*mut GSDevice12> = None;
pub static mut g_perfmon: PerfMon = PerfMon::new();
pub struct PerfMon { _private: [u8; 0] }
impl PerfMon { pub const fn new() -> Self { Self { _private: [0; 0] } } pub fn put(&self, _a: u32, _b: u32) {} }
pub mod perfmon { pub mod gs_perf_mon { pub const DRAW_CALLS: u32 = 0; pub const TEXTURE_COPIES: u32 = 1; pub const TEXTURE_UPLOADS: u32 = 2; pub const READBACKS: u32 = 3; pub const BARRIERS: u32 = 4; pub const BARRIERS_ROV: u32 = 5; pub const DRAW_CALLS_ROV: u32 = 6; pub const RENDER_PASSES: u32 = 7; } }
pub use perfmon::gs_perf_mon as GsPerfMon;

pub mod console {
    pub fn error(_f: &str) {}
    pub fn error_fmt(_f: &str) {}
    pub fn warning(_f: &str) {}
    pub fn write_ln(_f: &str) {}
    pub fn write_ln_fmt(_f: &str) {}
}

pub mod devcon { pub fn write_ln(_f: &str, _a: u32, _b: u32) {} }
pub mod gl_ins { pub fn push(_f: &str) {} pub fn pop() {} }
pub mod d3d {
    use super::*;
    pub fn create_factory(_b: bool) -> ComPtr<IDXGIFactory5> { ComPtr::null() }
    pub fn get_adapter_by_name<T>(_: *mut T, _: String) -> ComPtr<IDXGIAdapter1> { ComPtr::null() }
    pub fn get_adapter_name(_: *mut IDXGIAdapter1) -> String { String::new() }
    pub fn get_driver_version_from_luid(_: i64) -> String { String::new() }
    pub fn compile_shader(_a: u32, _b: u32, _c: bool, _s: String, _m: *const c_void, _e: &str) -> ComPtr<ID3DBlob> { ComPtr::null() }
    pub fn get_requested_exclusive_fullscreen_mode_desc<T>(_a: *mut T, _b: *mut c_void, _w: u32, _h: u32, _r: f32, _f: u32, _m: *mut DXGI_MODE_DESC, _o: *mut *mut IDXGIOutput) -> bool { false }
    #[derive(Copy, Clone, Debug, PartialEq, Eq)] pub enum ShaderType { Vertex, Pixel, Compute }
    #[derive(Copy, Clone, Debug, PartialEq, Eq)] pub enum ShaderModel { SM51, SM60 }
    pub fn shader_model_to_cache_string(_: ShaderModel) -> &'static str { "sm51" }
}


// =====================================================================
//  Descriptor handle + heap manager (from D3D12DescriptorHeapManager.cpp)
// =====================================================================

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct D3D12DescriptorHandle { pub cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE, pub gpu_handle: D3D12_GPU_VIRTUAL_ADDRESS, pub index: u32 }
impl D3D12DescriptorHandle {
    pub const INVALID_INDEX: u32 = 0xFFFF_FFFF;
    pub const fn invalid() -> Self { Self { cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE::zero(), gpu_handle: D3D12_GPU_VIRTUAL_ADDRESS::zero(), index: Self::INVALID_INDEX } }
    pub fn clear(&mut self) { *self = Self::invalid(); }
}
impl From<*mut c_void> for D3D12DescriptorHandle { fn from(_: *mut c_void) -> Self { Self::invalid() } }

pub type BitSetType = std::collections::BTreeSet<u32>;

pub const BITSET_SIZE: u32 = 64;

/// Fixed-capacity descriptor heap (mirrors `D3D12DescriptorHeapManager`).
pub struct D3D12DescriptorHeapManager {
    pub descriptor_heap: ComPtr<ID3D12DescriptorHeap>,
    pub heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub heap_base_gpu: D3D12_GPU_VIRTUAL_ADDRESS,
    pub num_descriptors: u32,
    pub descriptor_increment_size: u32,
    pub shader_visible: bool,
    pub free_slots: Vec<BitSetType>,
}

impl D3D12DescriptorHeapManager {
    pub const fn new() -> Self {
        Self {
            descriptor_heap: ComPtr::null(),
            heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE::zero(),
            heap_base_gpu: D3D12_GPU_VIRTUAL_ADDRESS::zero(),
            num_descriptors: 0,
            descriptor_increment_size: 0,
            shader_visible: false,
            free_slots: Vec::new(),
        }
    }
    pub fn create(&mut self, _device: *mut ID3D12Device, ty: u32, num: u32, shader_visible: bool) -> bool {
        self.num_descriptors = num;
        self.shader_visible = shader_visible;
        let groups = (num / BITSET_SIZE) + (if num % BITSET_SIZE != 0 { 1 } else { 0 });
        self.free_slots = (0..groups).map(|_| (0..BITSET_SIZE).collect()).collect();
        let _ = ty;
        true
    }
    pub fn destroy(&mut self) {
        self.shader_visible = false;
        self.num_descriptors = 0;
        self.descriptor_increment_size = 0;
        self.heap_base_cpu = D3D12_CPU_DESCRIPTOR_HANDLE::zero();
        self.heap_base_gpu = D3D12_GPU_VIRTUAL_ADDRESS::zero();
        self.descriptor_heap.reset();
        self.free_slots.clear();
    }
    pub fn allocate(&mut self, handle: &mut D3D12DescriptorHandle) -> bool {
        for (g, bs) in self.free_slots.iter_mut().enumerate() {
            if let Some(&bit) = bs.iter().next() {
                bs.remove(&bit);
                let index = g as u32 * BITSET_SIZE + bit;
                handle.index = index;
                handle.cpu_handle = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: self.heap_base_cpu.ptr + index as u64 * self.descriptor_increment_size as u64 };
                handle.gpu_handle = D3D12_GPU_VIRTUAL_ADDRESS { ptr: if self.shader_visible { self.heap_base_gpu.ptr + index as u64 * self.descriptor_increment_size as u64 } else { 0 } };
                return true;
            }
        }
        false
    }
    pub fn free_index(&mut self, index: u32) {
        let g = (index / BITSET_SIZE) as usize;
        let bit = index % BITSET_SIZE;
        if let Some(bs) = self.free_slots.get_mut(g) { bs.insert(bit); }
    }
    pub fn free(&mut self, handle: &mut D3D12DescriptorHandle) {
        if handle.index == D3D12DescriptorHandle::INVALID_INDEX { return; }
        self.free_index(handle.index);
        handle.clear();
    }
    pub fn get_allocated_descriptors(&self) -> u32 {
        self.free_slots.iter().map(|bs| BITSET_SIZE - bs.len() as u32).sum()
    }
}

/// Linear descriptor allocator (`D3D12DescriptorAllocator`).
pub struct D3D12DescriptorAllocator {
    pub descriptor_heap: ComPtr<ID3D12DescriptorHeap>,
    pub heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub heap_base_gpu: D3D12_GPU_VIRTUAL_ADDRESS,
    pub num_descriptors: u32,
    pub descriptor_increment_size: u32,
    pub current_offset: u32,
}
impl D3D12DescriptorAllocator {
    pub const fn new() -> Self {
        Self { descriptor_heap: ComPtr::null(), heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE::zero(), heap_base_gpu: D3D12_GPU_VIRTUAL_ADDRESS::zero(), num_descriptors: 0, descriptor_increment_size: 0, current_offset: 0 }
    }
    pub fn create(&mut self, _device: *mut ID3D12Device, _ty: u32, num: u32) -> bool { self.num_descriptors = num; self.current_offset = 0; true }
    pub fn destroy(&mut self) { self.descriptor_heap.reset(); self.num_descriptors = 0; self.current_offset = 0; }
    pub fn allocate(&mut self, count: u32, out: &mut D3D12DescriptorHandle) -> bool {
        if self.current_offset + count > self.num_descriptors { return false; }
        out.index = self.current_offset;
        out.cpu_handle = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: self.heap_base_cpu.ptr + self.current_offset as u64 * self.descriptor_increment_size as u64 };
        out.gpu_handle = D3D12_GPU_VIRTUAL_ADDRESS { ptr: self.heap_base_gpu.ptr + self.current_offset as u64 * self.descriptor_increment_size as u64 };
        self.current_offset += count;
        true
    }
    pub fn reset(&mut self) { self.current_offset = 0; }
}

/// Grouped sampler allocator (`D3D12GroupedSamplerAllocator<SAMPLER_GROUP_SIZE>`).
pub struct D3D12GroupedSamplerAllocator<const N: u32> { _private: [u8; 0] }
impl<const N: u32> D3D12GroupedSamplerAllocator<N> {
    pub const fn new() -> Self { Self { _private: [0; 0] } }
    pub fn create(&mut self, _device: *mut ID3D12Device, _num: u32) -> bool { true }
    pub fn reset(&mut self) {}
    pub fn should_reset(&self) -> bool { false }
    pub fn invalidate_cache(&mut self) {}
    pub fn lookup_single(&self, _out: &mut D3D12DescriptorHandle, _cpu: D3D12DescriptorHandle) -> bool { true }
    pub fn get_descriptor_heap(&self) -> *mut ID3D12DescriptorHeap { ptr::null_mut() }
}

// =====================================================================
//  Stream buffer (from D3D12StreamBuffer.cpp)
// =====================================================================

#[derive(Clone, Debug, Default)]
pub struct TrackedFenceOffset { pub fence: u64, pub offset: u32 }

pub struct D3D12StreamBuffer {
    pub buffer_upload: ComPtr<ID3D12Resource>,
    pub buffer_default: ComPtr<ID3D12Resource>,
    pub allocation_upload: *mut D3D12MA_Allocation,
    pub allocation_default: *mut D3D12MA_Allocation,
    pub host_pointer: *mut u8,
    pub gpu_pointer: D3D12_GPU_VIRTUAL_ADDRESS,
    pub m_size: u32,
    pub m_current_offset: u32,
    pub m_current_copy_offset: u32,
    pub m_current_space: u32,
    pub m_current_gpu_position: u32,
    pub m_tracked_fences: Vec<TrackedFenceOffset>,
}
impl D3D12StreamBuffer {
    pub const fn new() -> Self {
        Self { buffer_upload: ComPtr::null(), buffer_default: ComPtr::null(),
            allocation_upload: ptr::null_mut(), allocation_default: ptr::null_mut(),
            host_pointer: ptr::null_mut(), gpu_pointer: D3D12_GPU_VIRTUAL_ADDRESS::zero(),
            m_size: 0, m_current_offset: 0, m_current_copy_offset: 0, m_current_space: 0, m_current_gpu_position: 0,
            m_tracked_fences: Vec::new() }
    }
    pub fn create(&mut self, _size: u32, _gpu_backed: bool) -> bool { self.m_size = _size; true }
    pub fn destroy(&mut self, _defer: bool) { self.m_current_offset = 0; self.m_tracked_fences.clear(); }
    pub fn reserve_memory(&mut self, num: u32, align: u32) -> bool { let required = num + align; if num > self.m_size { return false; } self.m_current_offset = align_up(self.m_current_offset, align); self.m_current_space = self.m_size - self.m_current_offset; required <= self.m_current_space }
    pub fn commit_memory(&mut self, size: u32) { self.m_current_offset += size; self.m_current_space -= size; }
    pub fn flush_memory(&mut self) {}
    pub fn get_buffer(&self) -> *mut ID3D12Resource { self.buffer_upload.as_ptr() }
    pub fn get_gpu_pointer(&self) -> D3D12_GPU_VIRTUAL_ADDRESS { self.gpu_pointer }
    pub fn get_current_offset(&self) -> u32 { self.m_current_offset }
    pub fn get_current_host_pointer(&self) -> *mut u8 { self.host_pointer }
    pub fn get_size(&self) -> u32 { self.m_size }
    pub fn update_current_fence_position(&mut self) {}
    fn wait_for_clear_space(&mut self, _n: u32) -> bool { false }
}
fn align_up(v: u32, a: u32) -> u32 { if a == 0 { v } else { (v + a - 1) & !(a - 1) } }

// =====================================================================
//  Shader cache (from D3D12ShaderCache.cpp)
// =====================================================================

pub const SHADER_CACHE_VERSION: u32 = 1;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct CacheIndexEntry { pub source_hash_low: u64, pub source_hash_high: u64, pub macro_hash_low: u64, pub macro_hash_high: u64, pub entry_point_low: u64, pub entry_point_high: u64, pub source_length: u32, pub shader_type: u32, pub file_offset: u32, pub blob_size: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CacheIndexKey { pub source_hash_low: u64, pub source_hash_high: u64, pub macro_hash_low: u64, pub macro_hash_high: u64, pub entry_point_low: u64, pub entry_point_high: u64, pub source_length: u32, pub r#type: u32 }

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct CacheIndexData { pub file_offset: u32, pub blob_size: u32 }

#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EntryType { VertexShader = 0, PixelShader = 1, ComputeShader = 2, GraphicsPipeline = 3, ComputePipeline = 4 }

pub type CacheIndex = HashMap<CacheIndexKey, CacheIndexData>;

pub struct D3D12ShaderCache {
    pub m_shader_model: d3d::ShaderModel,
    pub m_debug: bool,
    pub m_shader_index_file: Option<File>,
    pub m_shader_blob_file: Option<File>,
    pub m_pipeline_index_file: Option<File>,
    pub m_pipeline_blob_file: Option<File>,
    pub m_shader_index: CacheIndex,
    pub m_pipeline_index: CacheIndex,
}
impl D3D12ShaderCache {
    pub fn new() -> Self {
        Self { m_shader_model: d3d::ShaderModel::SM51, m_debug: false,
            m_shader_index_file: None, m_shader_blob_file: None,
            m_pipeline_index_file: None, m_pipeline_blob_file: None,
            m_shader_index: HashMap::new(), m_pipeline_index: HashMap::new() }
    }
    pub fn open(&mut self, _sm: d3d::ShaderModel, _dbg: bool) -> bool { true }
    pub fn close(&mut self) {}
    pub fn invalidate_pipeline_cache(&mut self) {}
    pub fn get_vertex_shader(&mut self, _s: String, _m: *const c_void, _e: &str) -> ComPtr<ID3DBlob> { ComPtr::null() }
    pub fn get_pixel_shader(&mut self, _s: String, _m: *const c_void, _e: &str) -> ComPtr<ID3DBlob> { ComPtr::null() }
    pub fn get_compute_shader(&mut self, _s: String, _m: *const c_void, _e: &str) -> ComPtr<ID3DBlob> { ComPtr::null() }
    pub fn get_pipeline_state_graphics(&mut self, _d: *mut ID3D12Device, _desc: D3D12_GRAPHICS_PIPELINE_STATE_DESC) -> ComPtr<ID3D12PipelineState> { ComPtr::null() }
    pub fn get_pipeline_state_compute(&mut self, _d: *mut ID3D12Device, _desc: D3D12_COMPUTE_PIPELINE_STATE_DESC) -> ComPtr<ID3D12PipelineState> { ComPtr::null() }
}

// =====================================================================
//  D3D12 builders (from D3D12Builders.cpp)
// =====================================================================

pub const MAX_VERTEX_ATTRIBUTES: usize = 16;
pub const MAX_DESCRIPTOR_RANGES: usize = 32;
pub const MAX_ROOT_PARAMETERS: usize = 16;

pub struct GraphicsPipelineBuilder {
    pub m_desc: D3D12_GRAPHICS_PIPELINE_STATE_DESC,
    pub m_input_elements: [D3D12_INPUT_ELEMENT_DESC; MAX_VERTEX_ATTRIBUTES],
}
impl Default for GraphicsPipelineBuilder {
    fn default() -> Self { Self { m_desc: unsafe { std::mem::zeroed() }, m_input_elements: [D3D12_INPUT_ELEMENT_DESC::default(); MAX_VERTEX_ATTRIBUTES] } }
}
impl GraphicsPipelineBuilder {
    pub fn new() -> Self { let mut s = Self::default(); s.clear(); s }
    pub fn clear(&mut self) { self.m_desc = unsafe { std::mem::zeroed() }; self.m_input_elements = [D3D12_INPUT_ELEMENT_DESC::default(); MAX_VERTEX_ATTRIBUTES]; self.m_desc.node_mask = 1; self.m_desc.sample_mask = 0xFFFF_FFFF; self.m_desc.sample_desc.count = 1; }
    pub fn set_root_signature(&mut self, rs: *mut ID3D12RootSignature) { self.m_desc.p_root_signature = rs; }
    pub fn set_vertex_shader_blob(&mut self, _blob: *mut ID3DBlob) {}
    pub fn set_vertex_shader(&mut self, _data: *const c_void, _size: u32) { self.m_desc.vs.p_shader_bytecode = _data; self.m_desc.vs.bytecode_length = _size as u64; }
    pub fn set_geometry_shader(&mut self, _data: *const c_void, _size: u32) { self.m_desc.gs.p_shader_bytecode = _data; self.m_desc.gs.bytecode_length = _size as u64; }
    pub fn set_pixel_shader(&mut self, _data: *const c_void, _size: u32) { self.m_desc.ps.p_shader_bytecode = _data; self.m_desc.ps.bytecode_length = _size as u64; }
    pub fn add_vertex_attribute(&mut self, name: &str, semantic_index: u32, format: u32, slot: u32, offset: u32) {
        let n = self.m_desc.input_layout.num_elements as usize;
        if n >= MAX_VERTEX_ATTRIBUTES { return; }
        let cstr = CString::new(name).unwrap();
        self.m_input_elements[n].semantic_name = cstr.as_ptr();
        self.m_input_elements[n].semantic_index = semantic_index;
        self.m_input_elements[n].format = format;
        self.m_input_elements[n].input_slot = slot;
        self.m_input_elements[n].aligned_byte_offset = offset;
        self.m_input_elements[n].input_slot_class = D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA;
        self.m_desc.input_layout.p_input_element_descs = self.m_input_elements.as_ptr();
        self.m_desc.input_layout.num_elements += 1;
    }
    pub fn set_primitive_topology_type(&mut self, t: u32) { self.m_desc.primitive_topology_type = t; }
    pub fn set_rasterization_state(&mut self, fill: u32, cull: u32, ccw: bool) { self.m_desc.rasterizer_state.fill_mode = fill; self.m_desc.rasterizer_state.cull_mode = cull; self.m_desc.rasterizer_state.front_counter_clockwise = ccw as u32; }
    pub fn set_multisamples(&mut self, n: u32) { self.m_desc.rasterizer_state.multisample_enable = (n > 1) as u32; self.m_desc.sample_desc.count = n; }
    pub fn set_no_cull_rasterization_state(&mut self) { self.set_rasterization_state(D3D12_FILL_MODE_SOLID, D3D12_CULL_MODE_NONE, false); }
    pub fn set_depth_state(&mut self, test: bool, write: bool, cmp: u32) { self.m_desc.depth_stencil_state.depth_enable = test as u32; self.m_desc.depth_stencil_state.depth_write_mask = if write { D3D12_DEPTH_WRITE_MASK_ALL } else { D3D12_DEPTH_WRITE_MASK_ZERO }; self.m_desc.depth_stencil_state.depth_func = cmp; }
    pub fn set_stencil_state(&mut self, test: bool, read_mask: u8, write_mask: u8, front: D3D12_DEPTH_STENCILOP_DESC, back: D3D12_DEPTH_STENCILOP_DESC) { self.m_desc.depth_stencil_state.stencil_enable = test as u32; self.m_desc.depth_stencil_state.stencil_read_mask = read_mask; self.m_desc.depth_stencil_state.stencil_write_mask = write_mask; self.m_desc.depth_stencil_state.front_face = front; self.m_desc.depth_stencil_state.back_face = back; }
    pub fn set_no_depth_test_state(&mut self) { self.set_depth_state(false, false, D3D12_COMPARISON_FUNC_ALWAYS); }
    pub fn set_no_stencil_state(&mut self) { let empty = D3D12_DEPTH_STENCILOP_DESC::default(); self.set_stencil_state(false, 0, 0, empty, empty); }
    pub fn set_blend_state(&mut self, rt: u32, enable: bool, src: u32, dst: u32, op: u32, src_a: u32, dst_a: u32, op_a: u32, mask: u8) {
        let i = rt as usize;
        self.m_desc.blend_state.render_target[i].blend_enable = enable as u32;
        self.m_desc.blend_state.render_target[i].src_blend = src; self.m_desc.blend_state.render_target[i].dest_blend = dst; self.m_desc.blend_state.render_target[i].blend_op = op;
        self.m_desc.blend_state.render_target[i].src_blend_alpha = src_a; self.m_desc.blend_state.render_target[i].dest_blend_alpha = dst_a; self.m_desc.blend_state.render_target[i].blend_op_alpha = op_a;
        self.m_desc.blend_state.render_target[i].render_target_write_mask = mask;
        if rt > 0 { self.m_desc.blend_state.independent_blend_enable = 1; }
    }
    pub fn set_color_write_mask(&mut self, rt: u32, mask: u8) { self.m_desc.blend_state.render_target[rt as usize].render_target_write_mask = mask; }
    pub fn set_no_blending_state(&mut self) { self.set_blend_state(0, false, D3D12_BLEND_ONE, D3D12_BLEND_ZERO, D3D12_BLEND_OP_ADD, D3D12_BLEND_ONE, D3D12_BLEND_ZERO, D3D12_BLEND_OP_ADD, D3D12_COLOR_WRITE_ENABLE_ALL); self.m_desc.blend_state.independent_blend_enable = 0; }
    pub fn clear_render_targets(&mut self) { self.m_desc.num_render_targets = 0; for f in self.m_desc.rtv_formats.iter_mut() { *f = DXGI_FORMAT_UNKNOWN; } }
    pub fn set_render_target(&mut self, rt: u32, format: u32) { self.m_desc.rtv_formats[rt as usize] = format; if (rt + 1) > self.m_desc.num_render_targets { self.m_desc.num_render_targets = rt + 1; } }
    pub fn clear_depth_stencil_format(&mut self) { self.m_desc.dsv_format = DXGI_FORMAT_UNKNOWN; }
    pub fn set_depth_stencil_format(&mut self, f: u32) { self.m_desc.dsv_format = f; }
    pub fn create(&mut self, _device: *mut ID3D12Device, _cache: &mut D3D12ShaderCache, _clear: bool) -> ComPtr<ID3D12PipelineState> { let _ = _clear; ComPtr::null() }
    pub fn create_raw(&mut self, _device: *mut ID3D12Device) -> ComPtr<ID3D12PipelineState> { ComPtr::null() }
}

pub struct ComputePipelineBuilder { pub m_desc: D3D12_COMPUTE_PIPELINE_STATE_DESC }
impl Default for ComputePipelineBuilder { fn default() -> Self { Self { m_desc: unsafe { std::mem::zeroed() } } } }
impl ComputePipelineBuilder {
    pub fn new() -> Self { let mut s = Self::default(); s.clear(); s }
    pub fn clear(&mut self) { self.m_desc = unsafe { std::mem::zeroed() }; }
    pub fn set_root_signature(&mut self, rs: *mut ID3D12RootSignature) { self.m_desc.p_root_signature = rs; }
    pub fn set_shader(&mut self, _data: *const c_void, _size: u32) { self.m_desc.cs.p_shader_bytecode = _data; self.m_desc.cs.bytecode_length = _size as u64; }
    pub fn create(&mut self, _device: *mut ID3D12Device, _cache: &mut D3D12ShaderCache, _clear: bool) -> ComPtr<ID3D12PipelineState> { ComPtr::null() }
}

pub struct RootSignatureBuilder {
    pub m_desc: D3D12_ROOT_SIGNATURE_DESC,
    pub m_params: [D3D12_ROOT_PARAMETER; MAX_ROOT_PARAMETERS],
    pub m_descriptor_ranges: [D3D12_DESCRIPTOR_RANGE; MAX_DESCRIPTOR_RANGES],
    pub m_num_descriptor_ranges: u32,
}
impl Default for RootSignatureBuilder {
    fn default() -> Self { Self { m_desc: unsafe { std::mem::zeroed() }, m_params: [D3D12_ROOT_PARAMETER::default(); MAX_ROOT_PARAMETERS], m_descriptor_ranges: [D3D12_DESCRIPTOR_RANGE::default(); MAX_DESCRIPTOR_RANGES], m_num_descriptor_ranges: 0 } }
}
impl RootSignatureBuilder {
    pub fn new() -> Self { let mut s = Self::default(); s.clear(); s }
    pub fn clear(&mut self) { self.m_desc = unsafe { std::mem::zeroed() }; self.m_params = [D3D12_ROOT_PARAMETER::default(); MAX_ROOT_PARAMETERS]; self.m_descriptor_ranges = [D3D12_DESCRIPTOR_RANGE::default(); MAX_DESCRIPTOR_RANGES]; self.m_desc.p_parameters = self.m_params.as_ptr(); self.m_num_descriptor_ranges = 0; }
    pub fn set_input_assembler_flag(&mut self) { self.m_desc.flags |= D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT; }
    pub fn add_32bit_constants(&mut self, reg: u32, num: u32, vis: u32) -> u32 { let i = self.m_desc.num_parameters; self.m_params[i as usize].parameter_type = D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS; self.m_params[i as usize].descriptor.constants.shader_register = reg; self.m_params[i as usize].descriptor.constants.register_space = 0; self.m_params[i as usize].descriptor.constants.num_32bit_values = num; self.m_params[i as usize].shader_visibility = vis; self.m_desc.num_parameters += 1; i }
    pub fn add_cbv_parameter(&mut self, reg: u32, vis: u32) -> u32 { let i = self.m_desc.num_parameters; self.m_params[i as usize].parameter_type = D3D12_ROOT_PARAMETER_TYPE_CBV; self.m_params[i as usize].descriptor.descriptor.shader_register = reg; self.m_params[i as usize].descriptor.descriptor.register_space = 0; self.m_params[i as usize].shader_visibility = vis; self.m_desc.num_parameters += 1; i }
    pub fn add_srv_parameter(&mut self, reg: u32, vis: u32) -> u32 { let i = self.m_desc.num_parameters; self.m_params[i as usize].parameter_type = D3D12_ROOT_PARAMETER_TYPE_SRV; self.m_params[i as usize].descriptor.descriptor.shader_register = reg; self.m_params[i as usize].descriptor.descriptor.register_space = 0; self.m_params[i as usize].shader_visibility = vis; self.m_desc.num_parameters += 1; i }
    pub fn add_descriptor_table(&mut self, ty: u32, start: u32, num: u32, vis: u32) -> u32 { let i = self.m_desc.num_parameters; let d = self.m_num_descriptor_ranges; self.m_descriptor_ranges[d as usize].range_type = ty; self.m_descriptor_ranges[d as usize].num_descriptors = num; self.m_descriptor_ranges[d as usize].base_shader_register = start; self.m_descriptor_ranges[d as usize].register_space = 0; self.m_descriptor_ranges[d as usize].offset_in_descriptors_from_table_start = D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND; self.m_params[i as usize].parameter_type = D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE; self.m_params[i as usize].descriptor.descriptor_table.p_descriptor_ranges = &self.m_descriptor_ranges[d as usize]; self.m_params[i as usize].descriptor.descriptor_table.num_descriptor_ranges = 1; self.m_params[i as usize].shader_visibility = vis; self.m_desc.num_parameters += 1; self.m_num_descriptor_ranges += 1; i }
    pub fn add_descriptor_table_multi_range(&mut self, n: u32, _ty: *const u32, _start: *const u32, _num: *const u32, vis: u32) -> u32 { let i = self.m_desc.num_parameters; self.m_params[i as usize].parameter_type = D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE; self.m_params[i as usize].descriptor.descriptor_table.p_descriptor_ranges = &self.m_descriptor_ranges[self.m_num_descriptor_ranges as usize]; self.m_params[i as usize].descriptor.descriptor_table.num_descriptor_ranges = n; self.m_params[i as usize].shader_visibility = vis; self.m_desc.num_parameters += 1; self.m_num_descriptor_ranges += n; i }
    pub fn create(&mut self, _clear: bool) -> ComPtr<ID3D12RootSignature> { ComPtr::null() }
}

pub fn set_object_name<T>(_obj: *mut T, _name: &str) {}


// =====================================================================
//  GSTexture12 (from GSTexture12.cpp)
// =====================================================================

pub struct GSTexture12 {
    pub m_resource: ComPtr<ID3D12Resource>,
    pub m_resource_fbl: ComPtr<ID3D12Resource>,
    pub m_allocation: *mut D3D12MA_Allocation,
    pub m_srv_descriptor: D3D12DescriptorHandle,
    pub m_write_descriptor: D3D12DescriptorHandle,
    pub m_read_dsv_descriptor: D3D12DescriptorHandle,
    pub m_uav_descriptor: D3D12DescriptorHandle,
    pub m_fbl_descriptor: D3D12DescriptorHandle,
    pub m_write_descriptor_type: GsWriteDescriptorType,
    pub m_dxgi_format: u32,
    pub m_resource_state: GsResourceState,
    pub m_simultaneous_tex: bool,
    pub m_type: GsTextureType,
    pub m_format: GsTextureFormat,
    pub m_size: GsVector2i,
    pub m_mipmap_levels: u32,
    pub m_state: GsTextureState,
    pub m_use_fence_counter: u64,
    pub m_needs_mipmaps_generated: bool,
    pub m_map_area: GsVector4i,
    pub m_map_level: u32,
    pub m_clear_value: GsTextureClearValue,
    pub m_clear_color: u32,
}
#[repr(C)]
pub union GsTextureClearValue { pub color: [f32; 4], pub depth_stencil: D3D12_DEPTH_STENCIL_VALUE }
impl GsTextureClearValue {
    pub fn depth(d: f32, s: u8) -> Self { Self { depth_stencil: D3D12_DEPTH_STENCIL_VALUE { depth: d, stencil: s } } }
}
impl Copy for GsTextureClearValue {}
impl Clone for GsTextureClearValue {
    fn clone(&self) -> Self { *self }
}
impl Default for GsTextureClearValue {
    fn default() -> Self { Self { color: [0.0; 4] } }
}
impl std::fmt::Debug for GsTextureClearValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Untagged union -- render the active interpretation conservatively.
        write!(f, "GsTextureClearValue {{ color: {:?} | depth_stencil: {:?} }}", unsafe { self.color }, unsafe { self.depth_stencil })
    }
}

impl GSTexture12 {
    pub fn new(_ty: GsTextureType, _fmt: GsTextureFormat, w: i32, h: i32, levels: u32, _f: u32, _srv: u32, _rtv: u32, _dsv: u32, _uav: u32) -> Self { Self::default_with_size(w, h, levels) }
    pub fn default_with_size(w: i32, h: i32, levels: u32) -> Self {
        Self { m_resource: ComPtr::null(), m_resource_fbl: ComPtr::null(), m_allocation: ptr::null_mut(),
            m_srv_descriptor: D3D12DescriptorHandle::invalid(), m_write_descriptor: D3D12DescriptorHandle::invalid(), m_read_dsv_descriptor: D3D12DescriptorHandle::invalid(), m_uav_descriptor: D3D12DescriptorHandle::invalid(), m_fbl_descriptor: D3D12DescriptorHandle::invalid(),
            m_write_descriptor_type: GsWriteDescriptorType::None, m_dxgi_format: 0, m_resource_state: GsResourceState::Undefined, m_simultaneous_tex: false,
            m_type: GsTextureType::Invalid, m_format: GsTextureFormat::Invalid, m_size: GsVector2i::new(w, h), m_mipmap_levels: levels,
            m_state: GsTextureState::Dirty, m_use_fence_counter: 0, m_needs_mipmaps_generated: false,
            m_map_area: GsVector4i::zero(), m_map_level: 0, m_clear_value: GsTextureClearValue::default(), m_clear_color: 0 }
    }
    pub fn destroy(&mut self, _defer: bool) {}
    pub fn get_resource(&self) -> *mut ID3D12Resource { self.m_resource.as_ptr() }
    pub fn get_dxgi_format(&self) -> u32 { self.m_dxgi_format }
    pub fn get_size(&self) -> GsVector2i { self.m_size }
    pub fn get_width(&self) -> i32 { self.m_size.x }
    pub fn get_height(&self) -> i32 { self.m_size.y }
    pub fn get_rect(&self) -> GsVector4i { GsVector4i::new(0, 0, self.m_size.x, self.m_size.y) }
    pub fn get_type(&self) -> GsTextureType { self.m_type }
    pub fn get_format(&self) -> GsTextureFormat { self.m_format }
    pub fn get_state(&self) -> GsTextureState { self.m_state }
    pub fn set_state(&mut self, s: GsTextureState) { self.m_state = s; }
    pub fn is_depth_stencil(&self) -> bool { self.m_type == GsTextureType::DepthStencil }
    pub fn is_render_target(&self) -> bool { self.m_type == GsTextureType::RenderTarget }
    pub fn is_render_target_or_depth_stencil(&self) -> bool { self.is_render_target() || self.is_depth_stencil() }
    pub fn is_compressed_format(&self) -> bool { matches!(self.m_format, GsTextureFormat::BC1 | GsTextureFormat::BC2 | GsTextureFormat::BC3 | GsTextureFormat::BC7) }
    pub fn get_mipmap_levels(&self) -> i32 { self.m_mipmap_levels as i32 }
    pub fn get_srv_descriptor(&self) -> D3D12DescriptorHandle { self.m_srv_descriptor }
    pub fn get_uav_descriptor(&self) -> D3D12DescriptorHandle { self.m_uav_descriptor }
    pub fn get_write_descriptor(&self) -> D3D12DescriptorHandle { self.m_write_descriptor }
    pub fn get_read_depth_view_descriptor(&self) -> D3D12DescriptorHandle { self.m_read_dsv_descriptor }
    pub fn get_fbl_descriptor(&self) -> D3D12DescriptorHandle { self.m_fbl_descriptor }
    pub fn get_resource_state(&self) -> GsResourceState { self.m_resource_state }
    pub fn set_use_fence_counter(&mut self, f: u64) { self.m_use_fence_counter = f; }
    pub fn get_clear_color(&self) -> u32 { self.m_clear_color }
    pub fn set_clear_color(&mut self, c: u32) { self.m_clear_color = c; }
    pub fn get_clear_depth(&self) -> f32 { unsafe { self.m_clear_value.depth_stencil.depth } }
    pub fn get_clear_for_format(&self) -> GsVector4 { unsafe { GsVector4::new(self.m_clear_value.color[0], self.m_clear_value.color[1], self.m_clear_value.color[2], self.m_clear_value.color[3]) } }
    pub fn transition_to_state(&mut self, _state: GsResourceState) { self.m_resource_state = _state; }
    pub fn commit_clear(&mut self) { self.m_state = GsTextureState::Dirty; }
    pub fn get_native_handle(&self) -> *mut c_void { self as *const _ as *mut c_void }
    pub fn update(&mut self, _r: GsVector4i, _data: *const c_void, _pitch: i32, _layer: i32) -> bool { true }
    pub fn generate_mipmap(&mut self) {}
    pub fn render_texture_mipmap(&mut self, _dst_level: u32, _dst_w: u32, _dst_h: u32, _src_level: u32, _src_w: u32, _src_h: u32) {}
    pub fn adopt(_resource: ComPtr<ID3D12Resource>, _ty: GsTextureType, _fmt: GsTextureFormat, _w: i32, _h: i32, _levels: u32, _f: u32, _srv: u32, _rtv: u32, _dsv: u32, _uav: u32, _state: GsResourceState) -> Option<Box<GSTexture12>> { Some(Box::new(GSTexture12::default_with_size(_w, _h, _levels))) }
    pub fn create(_ty: GsTextureType, _fmt: GsTextureFormat, _w: i32, _h: i32, _levels: u32, _f: u32, _srv: u32, _rtv: u32, _dsv: u32, _uav: u32) -> Option<Box<GSTexture12>> { Some(Box::new(GSTexture12::default_with_size(_w, _h, _levels))) }
}

pub struct GSDownloadTexture12 { pub m_buffer: ComPtr<ID3D12Resource>, pub m_allocation: *mut D3D12MA_Allocation, pub m_buffer_size: u32, pub m_width: u32, pub m_height: u32, pub m_format: GsTextureFormat, pub m_current_pitch: u32, pub m_map_pointer: *mut u8, pub m_needs_flush: bool, pub m_copy_fence_value: u64 }
impl GSDownloadTexture12 { pub fn new(w: u32, h: u32, f: GsTextureFormat) -> Self { Self { m_buffer: ComPtr::null(), m_allocation: ptr::null_mut(), m_buffer_size: 0, m_width: w, m_height: h, m_format: f, m_current_pitch: 0, m_map_pointer: ptr::null_mut(), m_needs_flush: false, m_copy_fence_value: 0 } } pub fn is_mapped(&self) -> bool { !self.m_map_pointer.is_null() } pub fn create(_w: u32, _h: u32, _f: GsTextureFormat) -> Option<Box<GSDownloadTexture12>> { Some(Box::new(GSDownloadTexture12::new(_w, _h, _f))) } pub fn copy_from_texture(&mut self, _: GsVector4i, _: *mut GsTexture, _: GsVector4i, _: u32, _: bool) {} pub fn map(&mut self, _: GsVector4i) -> bool { true } pub fn unmap(&mut self) {} pub fn flush(&mut self) {} }

// =====================================================================
//  GSDevice12 helper types and constants
// =====================================================================

/// Per-command-list GPU resources (mirrors the C++ `CommandListResources`).
#[derive(Copy, Clone, Debug, Default)]
pub struct CommandListResources {
    pub command_list: *mut ID3D12GraphicsCommandList,
    pub command_list4: *mut ID3D12GraphicsCommandList4,
    pub command_list7: *mut ID3D12GraphicsCommandList7,
    pub command_allocator: *mut ID3D12CommandAllocator,
}

/// Pipeline selector key (mirrors the C++ `PipelineSelector`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PipelineSelector {
    pub vs: u32,
    pub ps: u32,
    pub topology: u32,
    pub rt: u32,
    pub ds: u32,
    pub blend: u32,
    pub dss: u32,
    pub depth_clamp: u32,
    pub interlace: u32,
    pub shader_interlace: u32,
    pub colclip_mode: u32,
    pub dither: u32,
    pub primitive_id: u32,
    pub sample_rate: u32,
    pub line_expand: u32,
}
impl PipelineSelector {
    pub fn zeroed() -> Self { Self::default() }
}

/// Device feature flags shared with the C++ GsDevice.
#[derive(Copy, Clone, Debug)]
pub struct GsDeviceFeatures {
    pub texture_barrier: bool,
    pub multidraw_fb_copy: bool,
    pub broken_point_sampler: bool,
    pub primitive_id: bool,
    pub prefer_new_textures: bool,
    pub provoking_vertex_last: bool,
    pub point_expand: bool,
    pub line_expand: bool,
    pub framebuffer_fetch: bool,
    pub stencil_buffer: bool,
    pub cas_sharpening: bool,
    pub test_and_sample_depth: bool,
    pub vs_expand: bool,
    pub depth_feedback: bool,
    pub aa1: bool,
    pub dxt_textures: bool,
    pub bptc_textures: bool,
    pub rov: bool,
}
impl Default for GsDeviceFeatures {
    fn default() -> Self {
        Self {
            texture_barrier: false, multidraw_fb_copy: false, broken_point_sampler: false,
            primitive_id: true, prefer_new_textures: true, provoking_vertex_last: false,
            point_expand: false, line_expand: false, framebuffer_fetch: false,
            stencil_buffer: true, cas_sharpening: true, test_and_sample_depth: true,
            vs_expand: true, depth_feedback: false, aa1: false,
            dxt_textures: false, bptc_textures: false, rov: false,
        }
    }
}

/// Native window descriptor passed to the GS device.
#[derive(Copy, Clone, Debug, Default)]
pub struct WindowInfo {
    pub window_handle: *mut c_void,
    pub surface_width: u32,
    pub surface_height: u32,
    pub surface_scale: f32,
    pub surface_refresh_rate: f32,
    pub r#type: u32,
}

/// TFX root-sig variant used for the currently bound PSO.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RootSignature { Undefined = 0, Tfx = 1, Utility = 2, Cas = 3 }

/// TFX pipeline binding slot counts.
pub const NUM_TFX_CONSTANT_BUFFERS: usize = 4;
pub const NUM_TOTAL_TFX_TEXTURES: usize = 64;
pub const NUM_TFX_UAV_TEXTURES: usize = 2;

// The main D3D12 device struct.
pub struct GSDevice12 {
    pub m_device: *mut ID3D12Device,
    pub m_command_queue: *mut ID3D12CommandQueue,
    pub m_adapter: *mut IDXGIAdapter1,
    pub m_allocator: *mut D3D12MA_Allocator,
    pub m_fence: *mut ID3D12Fence,
    pub m_fence_event: *mut c_void,
    pub m_current_fence_value: u32,
    pub m_completed_fence_value: u64,
    pub m_command_lists: Vec<CommandListResources>,
    pub m_current_command_list: u32,
    pub m_timestamp_query_heap: *mut ID3D12QueryHeap,
    pub m_timestamp_query_buffer: *mut ID3D12Resource,
    pub m_timestamp_query_allocation: *mut D3D12MA_Allocation,
    pub m_timestamp_frequency: f64,
    pub m_accumulated_gpu_time: f32,
    pub m_gpu_timing_enabled: bool,
    pub m_programmable_sample_positions: bool,
    pub m_descriptor_heap_manager: D3D12DescriptorHeapManager,
    pub m_rtv_heap_manager: D3D12DescriptorHeapManager,
    pub m_dsv_heap_manager: D3D12DescriptorHeapManager,
    pub m_sampler_heap_manager: D3D12DescriptorHeapManager,
    pub m_feature_level: u32,
    pub m_name: String,
    pub m_dxgi_factory: ComPtr<IDXGIFactory5>,
    pub m_swap_chain: ComPtr<IDXGISwapChain1>,
    pub m_swap_chain_buffers: Vec<Box<GSTexture12>>,
    pub m_current_swap_chain_buffer: u32,
    pub m_allow_tearing_supported: bool,
    pub m_using_allow_tearing: bool,
    pub m_is_exclusive_fullscreen: bool,
    pub m_uma: bool,
    pub m_typed_casting_supported: bool,
    pub m_enhanced_barriers: bool,
    pub m_device_lost: bool,
    pub m_tfx_root_signature: ComPtr<ID3D12RootSignature>,
    pub m_utility_root_signature: ComPtr<ID3D12RootSignature>,
    pub m_cas_root_signature: ComPtr<ID3D12RootSignature>,
    pub m_cas_upscale_pipeline: ComPtr<ID3D12PipelineState>,
    pub m_cas_sharpen_pipeline: ComPtr<ID3D12PipelineState>,
    pub m_vertex_stream_buffer: D3D12StreamBuffer,
    pub m_index_stream_buffer: D3D12StreamBuffer,
    pub m_expand_index_stream_buffer: D3D12StreamBuffer,
    pub m_vertex_constant_buffer: D3D12StreamBuffer,
    pub m_pixel_constant_buffer: D3D12StreamBuffer,
    pub m_texture_stream_buffer: D3D12StreamBuffer,
    pub m_expand_index_buffer: ComPtr<ID3D12Resource>,
    pub m_expand_index_buffer_allocation: *mut D3D12MA_Allocation,
    pub m_point_sampler_cpu: D3D12DescriptorHandle,
    pub m_linear_sampler_cpu: D3D12DescriptorHandle,
    pub m_tfx_sampler: D3D12DescriptorHandle,
    pub m_samplers: HashMap<u32, D3D12DescriptorHandle>,
    pub m_convert: Vec<ComPtr<ID3D12PipelineState>>,
    pub m_present: [ComPtr<ID3D12PipelineState>; 1],
    pub m_merge: [ComPtr<ID3D12PipelineState>; 2],
    pub m_interlace: [ComPtr<ID3D12PipelineState>; 4],
    pub m_colclip_setup_pipelines: [[ComPtr<ID3D12PipelineState>; 2]; 2],
    pub m_colclip_finish_pipelines: [[ComPtr<ID3D12PipelineState>; 2]; 2],
    pub m_primid_image_setup_pipelines: [[ComPtr<ID3D12PipelineState>; 4]; 2],
    pub m_fxaa_pipeline: ComPtr<ID3D12PipelineState>,
    pub m_shadeboost_pipeline: ComPtr<ID3D12PipelineState>,
    pub m_imgui_pipeline: ComPtr<ID3D12PipelineState>,
    pub m_tfx_vertex_shaders: HashMap<u32, ComPtr<ID3DBlob>>,
    pub m_tfx_pixel_shaders: HashMap<GsPsSelector, ComPtr<ID3DBlob>>,
    pub m_tfx_pipelines: HashMap<PipelineSelector, ComPtr<ID3D12PipelineState>>,
    pub m_vs_cb_cache: GsVsConstantBuffer,
    pub m_ps_cb_cache: GsPsConstantBuffer,
    pub m_vs_pc_cache: GsVsPushConstants,
    pub m_shader_cache: D3D12ShaderCache,
    pub m_convert_vs: ComPtr<ID3DBlob>,
    pub m_tfx_source: String,
    pub m_features: GsDeviceFeatures,
    pub m_max_texture_size: u32,
    pub m_window_info: WindowInfo,
    pub m_vsync_mode: GsVSyncMode,
    pub m_allow_present_throttle: bool,
    pub m_dirty_flags: u32,
    pub m_vertex_buffer: D3D12_VERTEX_BUFFER_VIEW,
    pub m_index_buffer: D3D12_INDEX_BUFFER_VIEW,
    pub m_primitive_topology: u32,
    pub m_current_render_target: *mut GSTexture12,
    pub m_current_depth_render_target: *mut GSTexture12,
    pub m_current_depth_target: *mut GSTexture12,
    pub m_current_depth_read_only: bool,
    pub m_viewport: D3D12_VIEWPORT,
    pub m_scissor: GsVector4i,
    pub m_blend_constant_color: u8,
    pub m_stencil_ref: u8,
    pub m_in_render_pass: bool,
    pub m_tfx_constant_buffers: [D3D12_GPU_VIRTUAL_ADDRESS; NUM_TFX_CONSTANT_BUFFERS],
    pub m_tfx_textures: [D3D12DescriptorHandle; NUM_TOTAL_TFX_TEXTURES],
    pub m_tfx_textures_uav: [*mut GSTexture12; NUM_TFX_UAV_TEXTURES],
    pub m_tfx_samplers_handle_gpu: D3D12DescriptorHandle,
    pub m_tfx_textures_handle_gpu: D3D12DescriptorHandle,
    pub m_tfx_rt_textures_handle_gpu: D3D12DescriptorHandle,
    pub m_tfx_sampler_sel: u32,
    pub m_utility_texture_cpu: D3D12DescriptorHandle,
    pub m_utility_texture_gpu: D3D12DescriptorHandle,
    pub m_utility_sampler_cpu: D3D12DescriptorHandle,
    pub m_utility_sampler_gpu: D3D12DescriptorHandle,
    pub m_current_root_signature: RootSignature,
    pub m_current_pipeline: *mut ID3D12PipelineState,
    pub m_null_texture: Option<Box<GSTexture12>>,
    pub m_pipeline_selector: PipelineSelector,
    pub m_ds_as_rt: Option<Box<GSTexture12>>,
    pub m_vertex: GsVertexBufferRef,
    pub m_index: GsIndexBufferRef,
    pub m_color_clip_texture: Option<Box<GSTexture12>>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct GsVertexBufferRef { pub start: u32, pub count: u32 }
#[derive(Copy, Clone, Debug, Default)]
pub struct GsIndexBufferRef { pub start: u32, pub count: u32 }

impl GSDevice12 {
    pub fn new() -> Self {
        Self {
            m_device: ptr::null_mut(), m_command_queue: ptr::null_mut(), m_adapter: ptr::null_mut(), m_allocator: ptr::null_mut(),
            m_fence: ptr::null_mut(), m_fence_event: ptr::null_mut(),
            m_current_fence_value: 0, m_completed_fence_value: 0,
            m_command_lists: Vec::new(), m_current_command_list: 0,
            m_timestamp_query_heap: ptr::null_mut(), m_timestamp_query_buffer: ptr::null_mut(), m_timestamp_query_allocation: ptr::null_mut(),
            m_timestamp_frequency: 0.0, m_accumulated_gpu_time: 0.0, m_gpu_timing_enabled: false, m_programmable_sample_positions: false,
            m_descriptor_heap_manager: D3D12DescriptorHeapManager::new(),
            m_rtv_heap_manager: D3D12DescriptorHeapManager::new(),
            m_dsv_heap_manager: D3D12DescriptorHeapManager::new(),
            m_sampler_heap_manager: D3D12DescriptorHeapManager::new(),
            m_feature_level: D3D_FEATURE_LEVEL_11_0,
            m_name: String::new(),
            m_dxgi_factory: ComPtr::null(), m_swap_chain: ComPtr::null(),
            m_swap_chain_buffers: Vec::new(), m_current_swap_chain_buffer: 0,
            m_allow_tearing_supported: false, m_using_allow_tearing: false, m_is_exclusive_fullscreen: false, m_uma: false,
            m_typed_casting_supported: false, m_enhanced_barriers: false, m_device_lost: false,
            m_tfx_root_signature: ComPtr::null(), m_utility_root_signature: ComPtr::null(),
            m_cas_root_signature: ComPtr::null(), m_cas_upscale_pipeline: ComPtr::null(), m_cas_sharpen_pipeline: ComPtr::null(),
            m_vertex_stream_buffer: D3D12StreamBuffer::new(), m_index_stream_buffer: D3D12StreamBuffer::new(),
            m_expand_index_stream_buffer: D3D12StreamBuffer::new(),
            m_vertex_constant_buffer: D3D12StreamBuffer::new(), m_pixel_constant_buffer: D3D12StreamBuffer::new(),
            m_texture_stream_buffer: D3D12StreamBuffer::new(),
            m_expand_index_buffer: ComPtr::null(), m_expand_index_buffer_allocation: ptr::null_mut(),
            m_point_sampler_cpu: D3D12DescriptorHandle::invalid(), m_linear_sampler_cpu: D3D12DescriptorHandle::invalid(),
            m_tfx_sampler: D3D12DescriptorHandle::invalid(),
            m_samplers: HashMap::new(),
            m_convert: Vec::new(),
            m_present: [ComPtr::null(); 1],
            m_merge: [ComPtr::null(), ComPtr::null()],
            m_interlace: [ComPtr::null(), ComPtr::null(), ComPtr::null(), ComPtr::null()],
            m_colclip_setup_pipelines: [[ComPtr::null(), ComPtr::null()], [ComPtr::null(), ComPtr::null()]],
            m_colclip_finish_pipelines: [[ComPtr::null(), ComPtr::null()], [ComPtr::null(), ComPtr::null()]],
            m_primid_image_setup_pipelines: [[ComPtr::null(), ComPtr::null(), ComPtr::null(), ComPtr::null()], [ComPtr::null(), ComPtr::null(), ComPtr::null(), ComPtr::null()]],
            m_fxaa_pipeline: ComPtr::null(), m_shadeboost_pipeline: ComPtr::null(), m_imgui_pipeline: ComPtr::null(),
            m_tfx_vertex_shaders: HashMap::new(), m_tfx_pixel_shaders: HashMap::new(), m_tfx_pipelines: HashMap::new(),
            m_vs_cb_cache: GsVsConstantBuffer::default(), m_ps_cb_cache: GsPsConstantBuffer::default(),
            m_vs_pc_cache: GsVsPushConstants::default(),
            m_shader_cache: D3D12ShaderCache::new(),
            m_convert_vs: ComPtr::null(), m_tfx_source: String::new(),
            m_features: GsDeviceFeatures { texture_barrier: false, multidraw_fb_copy: false, broken_point_sampler: false, primitive_id: true, prefer_new_textures: true, provoking_vertex_last: false, point_expand: false, line_expand: false, framebuffer_fetch: false, stencil_buffer: true, cas_sharpening: true, test_and_sample_depth: true, vs_expand: true, depth_feedback: false, aa1: false, dxt_textures: false, bptc_textures: false, rov: false },
            m_max_texture_size: 0, m_window_info: WindowInfo { window_handle: ptr::null_mut(), surface_width: 0, surface_height: 0, surface_scale: 1.0, surface_refresh_rate: 0.0, r#type: 0 },
            m_vsync_mode: GsVSyncMode::Fifo, m_allow_present_throttle: false,
            m_dirty_flags: 0,
            m_vertex_buffer: D3D12_VERTEX_BUFFER_VIEW::default(), m_index_buffer: D3D12_INDEX_BUFFER_VIEW::default(), m_primitive_topology: 0,
            m_current_render_target: ptr::null_mut(), m_current_depth_render_target: ptr::null_mut(), m_current_depth_target: ptr::null_mut(),
            m_current_depth_read_only: false,
            m_viewport: D3D12_VIEWPORT { x: 0.0, y: 0.0, w: 1.0, h: 1.0, min_z: 0.0, max_z: 1.0 },
            m_scissor: GsVector4i::zero(),
            m_blend_constant_color: 0, m_stencil_ref: 0, m_in_render_pass: false,
            m_tfx_constant_buffers: [D3D12_GPU_VIRTUAL_ADDRESS::zero(); NUM_TFX_CONSTANT_BUFFERS],
            m_tfx_textures: [D3D12DescriptorHandle::invalid(); NUM_TOTAL_TFX_TEXTURES],
            m_tfx_textures_uav: [ptr::null_mut(); NUM_TFX_UAV_TEXTURES],
            m_tfx_samplers_handle_gpu: D3D12DescriptorHandle::invalid(),
            m_tfx_textures_handle_gpu: D3D12DescriptorHandle::invalid(),
            m_tfx_rt_textures_handle_gpu: D3D12DescriptorHandle::invalid(),
            m_tfx_sampler_sel: 0,
            m_utility_texture_cpu: D3D12DescriptorHandle::invalid(),
            m_utility_texture_gpu: D3D12DescriptorHandle::invalid(),
            m_utility_sampler_cpu: D3D12DescriptorHandle::invalid(),
            m_utility_sampler_gpu: D3D12DescriptorHandle::invalid(),
            m_current_root_signature: RootSignature::Undefined,
            m_current_pipeline: ptr::null_mut(),
            m_null_texture: None,
            m_pipeline_selector: PipelineSelector::zeroed(),
            m_ds_as_rt: None,
            m_vertex: GsVertexBufferRef::default(),
            m_index: GsIndexBufferRef::default(),
            m_color_clip_texture: None,
        }
    }
}
