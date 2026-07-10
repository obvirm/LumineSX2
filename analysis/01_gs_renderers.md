# GS Renderers — External Library Dependency Analysis

## Directory: `pcsx2/pcsx2/GS/`

---

## 1. VULKAN RENDERER (`GS/Renderers/Vulkan/`)

### Files (17 files)
`GSDeviceVK.cpp/.h`, `GSTextureVK.cpp/.h`, `vk_mem_alloc.cpp`, `VKBuilders.cpp/.h`, `VKEntryPoints.h/.inl`, `VKLoader.cpp/.h`, `VKShaderCache.cpp/.h`, `VKStreamBuffer.cpp/.h`, `VKSwapChain.cpp/.h`

### External Dependencies

#### a) `vulkan/vulkan.h` (Vulkan SDK)
**Used by:** `VKLoader.h` (entry point), all `.cpp` files via `VKLoader.h`

**C functions called (Vulkan API, 150+ entry points):**

**Module functions (6):**
| Function | Required? |
|----------|-----------|
| `vkCreateInstance` | ✅ |
| `vkGetInstanceProcAddr` | ✅ |
| `vkEnumerateInstanceExtensionProperties` | ✅ |
| `vkEnumerateInstanceLayerProperties` | ✅ |
| `vkEnumerateInstanceVersion` | ❌ |
| `vkDestroyInstance` | ✅ |

**Instance functions (40+):**
| Function | Required? |
|----------|-----------|
| `vkGetDeviceProcAddr` | ✅ |
| `vkEnumeratePhysicalDevices` | ✅ |
| `vkGetPhysicalDeviceFeatures` | ✅ |
| `vkGetPhysicalDeviceFormatProperties` | ✅ |
| `vkGetPhysicalDeviceImageFormatProperties` | ✅ |
| `vkGetPhysicalDeviceProperties` | ✅ |
| `vkGetPhysicalDeviceQueueFamilyProperties` | ✅ |
| `vkGetPhysicalDeviceMemoryProperties` | ✅ |
| `vkCreateDevice` | ✅ |
| `vkEnumerateDeviceExtensionProperties` | ✅ |
| `vkEnumerateDeviceLayerProperties` | ✅ |
| `vkGetPhysicalDeviceSparseImageFormatProperties` | ✅ |
| `vkDestroySurfaceKHR` | ❌ |
| `vkGetPhysicalDeviceSurfaceSupportKHR` | ❌ |
| `vkGetPhysicalDeviceSurfaceCapabilitiesKHR` | ❌ |
| `vkGetPhysicalDeviceSurfaceFormatsKHR` | ❌ |
| `vkGetPhysicalDeviceSurfacePresentModesKHR` | ❌ |
| `vkCreateWin32SurfaceKHR` | ❌ (Win32) |
| `vkGetPhysicalDeviceWin32PresentationSupportKHR` | ❌ (Win32) |
| `vkCreateXlibSurfaceKHR` | ❌ (X11) |
| `vkGetPhysicalDeviceXlibPresentationSupportKHR` | ❌ (X11) |
| `vkCreateWaylandSurfaceKHR` | ❌ (Wayland) |
| `vkCreateMetalSurfaceEXT` | ❌ (Apple) |
| Debug utils (12 functions) | ❌ |
| `vkGetPhysicalDeviceSurfaceCapabilities2KHR` | ❌ |
| Display functions (7) | ❌ |
| Vulkan 1.1: `vkGetPhysicalDeviceFeatures2` | ✅ |
| Vulkan 1.1: `vkGetPhysicalDeviceProperties2` | ✅ |
| Vulkan 1.1: `vkGetPhysicalDeviceMemoryProperties2` | ✅ |
| `vkGetPhysicalDeviceCalibrateableTimeDomainsEXT` | ❌ |

**Device functions (100+):**
| Category | Functions |
|----------|-----------|
| Device lifecycle | `vkDestroyDevice`, `vkGetDeviceQueue`, `vkDeviceWaitIdle` |
| Queue | `vkQueueSubmit`, `vkQueueWaitIdle`, `vkQueuePresentKHR` |
| Memory | `vkAllocateMemory`, `vkFreeMemory`, `vkMapMemory`, `vkUnmapMemory`, `vkFlushMappedMemoryRanges`, `vkInvalidateMappedMemoryRanges`, `vkGetDeviceMemoryCommitment`, `vkBindBufferMemory`, `vkBindImageMemory`, `vkGetBufferMemoryRequirements`, `vkGetImageMemoryRequirements`, `vkGetImageSparseMemoryRequirements`, `vkQueueBindSparse` |
| Fences | `vkCreateFence`, `vkDestroyFence`, `vkResetFences`, `vkGetFenceStatus`, `vkWaitForFences` |
| Semaphores | `vkCreateSemaphore`, `vkDestroySemaphore` |
| Events | `vkCreateEvent`, `vkDestroyEvent`, `vkGetEventStatus`, `vkSetEvent`, `vkResetEvent` |
| Queries | `vkCreateQueryPool`, `vkDestroyQueryPool`, `vkGetQueryPoolResults` |
| Buffers | `vkCreateBuffer`, `vkDestroyBuffer`, `vkCreateBufferView`, `vkDestroyBufferView` |
| Images | `vkCreateImage`, `vkDestroyImage`, `vkGetImageSubresourceLayout`, `vkCreateImageView`, `vkDestroyImageView` |
| Shaders | `vkCreateShaderModule`, `vkDestroyShaderModule` |
| Pipelines | `vkCreatePipelineCache`, `vkDestroyPipelineCache`, `vkGetPipelineCacheData`, `vkMergePipelineCaches`, `vkCreateGraphicsPipelines`, `vkCreateComputePipelines`, `vkDestroyPipeline`, `vkCreatePipelineLayout`, `vkDestroyPipelineLayout` |
| Samplers | `vkCreateSampler`, `vkDestroySampler` |
| Descriptors | `vkCreateDescriptorSetLayout`, `vkDestroyDescriptorSetLayout`, `vkCreateDescriptorPool`, `vkDestroyDescriptorPool`, `vkResetDescriptorPool`, `vkAllocateDescriptorSets`, `vkFreeDescriptorSets`, `vkUpdateDescriptorSets` |
| Framebuffer/RenderPass | `vkCreateFramebuffer`, `vkDestroyFramebuffer`, `vkCreateRenderPass`, `vkDestroyRenderPass`, `vkGetRenderAreaGranularity` |
| Command buffers | `vkCreateCommandPool`, `vkDestroyCommandPool`, `vkResetCommandPool`, `vkAllocateCommandBuffers`, `vkFreeCommandBuffers`, `vkBeginCommandBuffer`, `vkEndCommandBuffer`, `vkResetCommandBuffer` |
| Draw commands | `vkCmdBindPipeline`, `vkCmdSetViewport`, `vkCmdSetScissor`, `vkCmdSetLineWidth`, `vkCmdSetDepthBias`, `vkCmdSetBlendConstants`, `vkCmdSetDepthBounds`, `vkCmdSetStencilCompareMask`, `vkCmdSetStencilWriteMask`, `vkCmdSetStencilReference`, `vkCmdBindDescriptorSets`, `vkCmdBindIndexBuffer`, `vkCmdBindVertexBuffers`, `vkCmdDraw`, `vkCmdDrawIndexed`, `vkCmdDrawIndirect`, `vkCmdDrawIndexedIndirect` |
| Compute | `vkCmdDispatch`, `vkCmdDispatchIndirect` |
| Copy/Blit | `vkCmdCopyBuffer`, `vkCmdCopyImage`, `vkCmdBlitImage`, `vkCmdCopyBufferToImage`, `vkCmdCopyImageToBuffer`, `vkCmdUpdateBuffer`, `vkCmdFillBuffer`, `vkCmdClearColorImage`, `vkCmdClearDepthStencilImage`, `vkCmdClearAttachments`, `vkCmdResolveImage` |
| Sync | `vkCmdSetEvent`, `vkCmdResetEvent`, `vkCmdWaitEvents`, `vkCmdPipelineBarrier` |
| Queries (cmd) | `vkCmdBeginQuery`, `vkCmdEndQuery`, `vkCmdResetQueryPool`, `vkCmdWriteTimestamp`, `vkCmdCopyQueryPoolResults` |
| Push constants | `vkCmdPushConstants` |
| RenderPass (cmd) | `vkCmdBeginRenderPass`, `vkCmdNextSubpass`, `vkCmdEndRenderPass` |
| Secondary | `vkCmdExecuteCommands` |
| Swapchain | `vkCreateSwapchainKHR`, `vkDestroySwapchainKHR`, `vkGetSwapchainImagesKHR`, `vkAcquireNextImageKHR` |
| Vulkan 1.1 | `vkGetBufferMemoryRequirements2`, `vkGetImageMemoryRequirements2`, `vkBindBufferMemory2`, `vkBindImageMemory2` |
| Vulkan 1.3 | `vkGetDeviceBufferMemoryRequirements`, `vkGetDeviceImageMemoryRequirements` |
| Ext (Win32) | `vkAcquireFullScreenExclusiveModeEXT`, `vkReleaseFullScreenExclusiveModeEXT` |
| Ext (timestamps) | `vkGetCalibratedTimestampsEXT` |
| Ext (push descriptor) | `vkCmdPushDescriptorSetKHR` |
| Ext (swapchain maint) | `vkReleaseSwapchainImagesEXT`, `vkReleaseSwapchainImagesKHR` |

**Vulkan types/structs used:**
- VkInstance, VkPhysicalDevice, VkDevice, VkQueue, VkCommandBuffer
- VkSurfaceKHR, VkSwapchainKHR, VkRenderPass, VkFramebuffer
- VkBuffer, VkImage, VkImageView, VkSampler, VkShaderModule
- VkPipeline, VkPipelineLayout, VkPipelineCache
- VkDescriptorPool, VkDescriptorSetLayout, VkDescriptorSet
- VkFence, VkSemaphore, VkEvent, VkQueryPool
- VkCommandPool, VkCommandBuffer
- All Vk*CreateInfo structs, Vk*SubresourceLayout, VkRect2D, VkViewport, etc.
- VkAllocationCallbacks (always nullptr)
- VkFormat, VkImageLayout, VkPipelineStageFlags, VkAccessFlags, etc.

#### b) `vk_mem_alloc.h` (Vulkan Memory Allocator)
**Used by:** `VKLoader.h` (included there, used in GSDeviceVK)

**C types:** `VmaAllocator`, `VmaAllocation`, `VmaAllocationCreateInfo`, `VmaAllocatorCreateInfo`, `VmaPool`, `VmaVirtualBlock`
**C functions:** `vmaCreateAllocator`, `vmaDestroyAllocator`, `vmaAllocateMemoryPages`, `vmaFreeMemoryPages`, `vmaMapMemory`, `vmaUnmapMemory`, `vmaCreateBuffer`, `vmaDestroyBuffer`, `vmaCreateImage`, `vmaDestroyImage`, `vmaSetAllocationUserData`, `vmaGetAllocationInfo`, `vmaTouchAllocation`, `vmaGetAllocationMemoryProperties`, `vmaCreatePool`, `vmaDestroyPool`, `vmaFindMemoryTypeIndex`, `vmaFindMemoryTypeIndexForBufferInfo`, `vmaFindMemoryTypeIndexForImageInfo`, `vmaCalcMemoryTypeBudget`, etc.

**Note:** VMA is included in-tree as `3rdparty/include/vk_mem_alloc.h` and `pcsx2/GS/Renderers/Vulkan/vk_mem_alloc.cpp`

---

#### c) `shaderc/shaderc.h` (shaderc — GLSL→SPIR-V compiler)
**Used by:** `VKShaderCache.cpp`

**C functions:**
```c
shaderc_compiler_t shaderc_compiler_initialize(void);
void shaderc_compiler_release(shaderc_compiler_t);
shaderc_compile_options_t shaderc_compile_options_initialize(void);
void shaderc_compile_options_release(shaderc_compile_options_t);
void shaderc_compile_options_set_optimization_level(shaderc_compile_options_t, shaderc_optimization_level);
void shaderc_compile_options_set_target_env(shaderc_compile_options_t, shaderc_target_env, unsigned int);
shaderc_compilation_result_t shaderc_compile_into_spv(
    shaderc_compiler_t, const char*, size_t, shaderc_shader_kind,
    const char*, const char*, const shaderc_compile_options_t);
void shaderc_result_release(shaderc_compilation_result_t);
shaderc_compilation_status shaderc_result_get_status(const shaderc_compilation_result_t);
const char* shaderc_result_get_error_message(const shaderc_compilation_result_t);
const char* shaderc_result_get_bytes(const shaderc_compilation_result_t);
size_t shaderc_result_get_length(const shaderc_compilation_result_t);
```

**Types:** `shaderc_compiler_t`, `shaderc_compile_options_t`, `shaderc_compilation_result_t`, `shaderc_optimization_level`, `shaderc_shader_kind`, `shaderc_target_env`, `shaderc_compilation_status`

**Note:** This is the DLL we were missing (`shaderc_shared.dll`). Used for offline GLSL→SPIR-V compilation during shader cache building.

---

## 2. DIRECTX 11 RENDERER (`GS/Renderers/DX11/`)

### Files (8 files)
`D3D.cpp/.h`, `D3D11ShaderCache.cpp/.h`, `GSDevice11.cpp/.h`, `GSTexture11.cpp/.h`

### External Dependencies

#### a) `<d3d11_1.h>` (Direct3D 11.1)
**Used by:** All DX11 files

**COM interfaces (via `wil::com_ptr_nothrow<T>`):**
| Interface | Used In |
|-----------|---------|
| `ID3D11Device` | GSDevice11 |
| `ID3D11DeviceContext` | GSDevice11 |
| `ID3D11DeviceContext1` | GSDevice11 |
| `ID3D11VertexShader` | GSDevice11 |
| `ID3D11PixelShader` | GSDevice11 |
| `ID3D11InputLayout` | GSDevice11 |
| `ID3D11Buffer` | GSDevice11 |
| `ID3D11Texture2D` | GSTexture11, GSDevice11 |
| `ID3D11ShaderResourceView` | GSTexture11 |
| `ID3D11RenderTargetView` | GSDevice11 |
| `ID3D11DepthStencilView` | GSDevice11 |
| `ID3D11UnorderedAccessView` | GSDevice11 |
| `ID3D11SamplerState` | GSDevice11 |
| `ID3D11RasterizerState` | GSDevice11 |
| `ID3D11DepthStencilState` | GSDevice11 |
| `ID3D11BlendState` | GSDevice11 |
| `ID3D11Query` | GSDevice11 |
| `ID3D11ComputeShader` | GSDevice11 |
| `ID3D11ClassLinkage` | GSDevice11 |
| `ID3D11Blob` (from D3DCompiler) | D3D11ShaderCache |
| `ID3D11DomainShader` | GSDevice11 |
| `ID3D11HullShader` | GSDevice11 |
| `ID3D11GeometryShader` | GSDevice11 |

**C functions called:**
```cpp
D3D11CreateDevice() // via D3D.cpp
D3DCompile() / D3DCompile2() // via D3D11ShaderCache
D3DReadFileToBlob() // via D3D11ShaderCache
D3DWriteBlobToFile() // via D3D11ShaderCache
```

#### b) `<dxgi1_5.h>` (DXGI 1.5)
**Used by:** `D3D.h`, `GSDevice11.h`

**COM interfaces:**
| Interface | Used In |
|-----------|---------|
| `IDXGIFactory5` | D3D.cpp, GSDevice11 |
| `IDXGIAdapter1` | D3D.cpp, GSDevice11 |
| `IDXGIOutput` | D3D.cpp |
| `IDXGISwapChain` | GSDevice11 |
| `IDXGISwapChain1` | GSDevice11 |
| `IDXGISwapChain2` | GSDevice11 |
| `IDXGISwapChain3` | GSDevice11 |
| `IDXGIDevice` | GSDevice11 |
| `IDXGIResource` | GSDevice11 |
| `IDXGIKeyedMutex` | GSDevice11 |
| `IDXGISurface` | GSDevice11 |

#### c) `<d3dcommon.h>` (D3D Common)
**Used by:** `GSUtil.cpp`

**Types:** `D3D_FEATURE_LEVEL`, `DXGI_FORMAT`

#### d) WIL — Windows Implementation Library
**Used by:** All DX11 files via `common/RedtapeWilCom.h`

**Types:**
```cpp
wil::com_ptr_nothrow<T>  // COM smart pointer (used everywhere)
wil::unique_hmodule       // Module handle RAII
wil::unique_any           // Generic RAII wrapper
wil::result_t             // HRESULT wrapper
```

#### e) `<d3dcompiler.h>` (D3D Compiler)
**Used by:** `D3D11ShaderCache.cpp/.h`

**COM interfaces:** `ID3DBlob` (shared with D3D common)
**Functions:** `D3DCompile`, `D3DCompile2`, `D3DReadFileToBlob`, `D3DWriteBlobToFile`

---

## 3. DIRECTX 12 RENDERER (`GS/Renderers/DX12/`)

### Files (11 files)
`D3D12Builders.cpp/.h`, `D3D12DescriptorHeapManager.cpp/.h`, `D3D12ShaderCache.cpp/.h`, `D3D12StreamBuffer.cpp/.h`, `GSDevice12.cpp/.h`, `GSTexture12.cpp/.h`

### External Dependencies

#### a) `<dxgi1_5.h>` (DXGI 1.5)
**Used by:** `GSDevice12.h`

**Interfaces:** Same as DX11, plus `IDXGISwapChain4`

#### b) `<directx/d3d12.h>` (Direct3D 12)
**Used by:** All DX12 files

**COM interfaces used:**
| Interface | Used In |
|-----------|---------|
| `ID3D12Device` | GSDevice12 |
| `ID3D12GraphicsCommandList` | GSDevice12 |
| `ID3D12GraphicsCommandList4` | D3D12Builders |
| `ID3D12GraphicsCommandList7` | GSDevice12 |
| `ID3D12CommandQueue` | GSDevice12 |
| `ID3D12CommandAllocator` | GSDevice12 |
| `ID3D12Fence` | GSDevice12 |
| `ID3D12Resource` | GSTexture12, D3D12StreamBuffer |
| `ID3D12DescriptorHeap` | D3D12DescriptorHeapManager |
| `ID3D12RootSignature` | GSDevice12 |
| `ID3D12PipelineState` | GSDevice12, D3D12ShaderCache |
| `ID3D12QueryHeap` | GSDevice12 |
| `ID3D12CommandSignature` | GSDevice12 |
| `ID3D12Heap` | D3D12StreamBuffer |
| `ID3D12DeviceRemovedExtendedData` | GSDevice12 |
| `ID3D12InfoQueue` | GSDevice12 |
| `ID3D12Debug` | GSDevice12 |
| `ID3D12DebugDevice1` | GSDevice12 |
| `ID3D12SharingContract` | GSDevice12 |
| `ID3D12VersionedRootSignatureDeserializer` | GSDevice12 |

**C functions:**
```cpp
D3D12CreateDevice()
D3D12GetDebugInterface()
D3D12SerializeVersionedRootSignature()
D3D12CreateVersionedRootSignatureDeserializer()
D3DCompile() / D3DCompile2() // via D3D12ShaderCache
```

#### c) D3D12 Memory Allocator
**Used by:** `GSDevice12.cpp`, `GSTexture12.cpp`, `D3D12StreamBuffer.cpp`

**Types:** `D3D12MA::Allocator`, `D3D12MA::Allocation`
**Note:** In `3rdparty/d3d12memalloc/`

---

## 4. OPENGL RENDERER (`GS/Renderers/OpenGL/`)

### Files (22 files)
`GLContext.cpp/.h`, `GLContextEGL.cpp/.h`, `GLContextEGLWayland.cpp/.h`, `GLContextEGLX11.cpp/.h`, `GLContextWGL.cpp/.h`, `GLProgram.cpp/.h`, `GLShaderCache.cpp/.h`, `GLState.cpp/.h`, `GLStreamBuffer.cpp/.h`, `GSDeviceOGL.cpp/.h`, `GSTextureOGL.cpp/.h`

### External Dependencies

#### a) OpenGL (via `glad`)
**Used by:** All OpenGL files

**C functions called (extensive list):**
```c
// Core OpenGL 3.3+ / 4.x
glBindTexture, glGenTextures, glDeleteTextures, glTexImage2D, glTexSubImage2D
glGenBuffers, glBindBuffer, glBufferData, glBufferSubData, glDeleteBuffers
glGenFramebuffers, glBindFramebuffer, glFramebufferTexture2D, glDeleteFramebuffers
glGenRenderbuffers, glBindRenderbuffer, glRenderbufferStorage, glDeleteRenderbuffers
glCreateShader, glShaderSource, glCompileShader, glGetShaderiv, glGetShaderInfoLog, glDeleteShader
glCreateProgram, glAttachShader, glLinkProgram, glGetProgramiv, glGetProgramInfoLog, glUseProgram, glDeleteProgram
glGetUniformLocation, glUniform*, glUniformMatrix*
glGetAttribLocation, glVertexAttribPointer, glEnableVertexAttribArray, glDisableVertexAttribArray
glGenVertexArrays, glBindVertexArray, glDeleteVertexArrays
glViewport, glScissor, glClear, glClearColor, glClearDepth
glEnable, glDisable, glBlendFunc, glBlendEquation, glDepthFunc, glDepthMask
glStencilFunc, glStencilOp, glStencilMask
glDrawArrays, glDrawElements, glDrawArraysInstanced, glDrawElementsInstanced
glActiveTexture, glBindSampler, glGenSamplers, glSamplerParameter*
glReadPixels, glFlush, glFinish, glGetError
// WGL (Windows):
wglCreateContext, wglMakeCurrent, wglDeleteContext, wglGetProcAddress, wglSwapIntervalEXT
// EGL (Linux):
eglGetDisplay, eglInitialize, eglCreateContext, eglMakeCurrent, etc.
```

**Types:** `GLuint`, `GLenum`, `GLint`, `GLsizei`, `GLboolean`, `GLbitfield`, `GLfloat`, `GLclampf`

#### b) `<X11/Xlib.h>` (X11 — Linux only)
**Used by:** `GLContextEGLX11.cpp`, `VKSwapChain.cpp`

---

## 5. UTILITY FILES

### a) `GS/GSPng.cpp` (PNG I/O)
**External headers:**
- `<zlib.h>` (ZLIB compression)
- `<png.h>` (libpng)

**C functions from libpng:**
```c
png_create_write_struct(PNG_LIBPNG_VER_STRING, ...)
png_create_info_struct(png_ptr)
png_init_io(png_ptr, fp)
png_set_IHDR(png_ptr, info_ptr, width, height, bit_depth, color_type, ...)
png_set_rows(png_ptr, info_ptr, row_pointers)
png_write_png(png_ptr, info_ptr, transforms, nullptr)
png_destroy_write_struct(&png_ptr, &info_ptr)
// Reading:
png_create_read_struct(PNG_LIBPNG_VER_STRING, ...)
png_create_info_struct(png_ptr)
png_init_io(png_ptr, fp)
png_read_info(png_ptr, info_ptr)
png_get_IHDR(png_ptr, info_ptr, &width, &height, &bit_depth, &color_type, ...)
png_set_expand(png_ptr)
png_read_image(png_ptr, row_pointers)
png_destroy_read_struct(&png_ptr, &info_ptr, nullptr)
```

**C functions from zlib:**
```c
// via <zlib.h> — used indirectly through libpng
```

### b) `GS/GSLzma.cpp` (LZMA/XZ compression)
**External headers:**
- `<Alloc.h>`, `<7zCrc.h>`, `<Xz.h>`, `<XzCrc64.h>` (LZMA SDK / 7-zip)
- `<zstd.h>` (Zstandard)

**C functions from LZMA SDK:**
```c
SzAllocTemp, SzFreeTemp
XzDec_Init(XzDecHandle)
XzDec_Decompress(XzDecHandle, ...)
7zCrc64Table[0x100]
// via GSLzma.h wrappers
```

**C functions from Zstd:**
```c
ZSTD_getFrameContentSize(src, srcSize)
ZSTD_decompress(dst, dstCapacity, src, srcSize)
ZSTD_compress(dst, dstCapacity, src, srcSize, compressionLevel)
ZSTD_compressBound(srcSize)
```

### c) `GS/GSDump.cpp` (GS Dump recording/playback)
**External headers:**
- `<7zCrc.h>`, `<XzCrc64.h>`, `<XzEnc.h>` (LZMA/XZ encoding)
- `<zstd.h>` (Zstandard)

**C functions from XZ:**
```c
XzEnc_Construct(encHandle)
XzEnc_SetProps(encHandle, &props)
XzEnc_Encode(encHandle, outStream, inStream, &progress, &alloc)
XzEnc_Destroy(encHandle)
XzProps_Init(&props)
7zCrc_Init()
7zCrc64_Init()
```

**C functions from Zstd:**
```c
ZSTD_getFrameContentSize()
ZSTD_decompress()
ZSTD_compress()
ZSTD_compressBound()
ZSTD_createCCtx() / ZSTD_freeCCtx()
ZSTD_compressCCtx()
ZSTD_createDCtx() / ZSTD_freeDCtx()
ZSTD_decompressDCtx()
```

### d) `GS/GSUtil.cpp` (GS Utilities)
**External headers:**
- `<d3dcommon.h>`, `<dxgi.h>` (Win32 only — adapter enumeration)
- `<wil/com.h>` (Win32 only)
- `GS/Renderers/DX11/D3D.h` (Win32 only — for D3D::GetAdapterInfo)
- `GS/Renderers/Vulkan/GSDeviceVK.h` (if Vulkan enabled)

**COM interfaces (Win32 only):**
```cpp
IDXGIFactory1, IDXGIAdapter1  // via CreateDXGIFactory1()
IDXGIFactory5                  // via D3D::CreateFactory()
```

### e) `GS/GSCapture.cpp` (Video Capture — FFmpeg)
**External headers:**
```c
<libavcodec/avcodec.h>
<libavformat/avformat.h>
<libavutil/dict.h>
<libavutil/opt.h>
<libavutil/channel_layout.h>
<libavutil/pixdesc.h>
<libswscale/swscale.h>
<libswresample/swresample.h>
```

**C functions from FFmpeg (30+):**
```c
// AVCodec:
avcodec_find_encoder_by_name, avcodec_find_encoder
avcodec_alloc_context3, avcodec_open2, avcodec_free_context
avcodec_send_frame, avcodec_receive_packet
avcodec_parameters_from_context, avcodec_get_hw_config
avcodec_get_hw_frames_parameters, avcodec_parameters_to_context
// AVFormat:
avformat_alloc_output_context2, avformat_free_context
avformat_new_stream, avformat_write_header
av_write_frame, av_interleaved_write_frame
av_write_trailer
// AVUtil:
av_dict_set, av_dict_get, av_dict_free
av_opt_set, av_image_fill_arrays, av_image_get_buffer_size
av_frame_alloc, av_frame_free, av_frame_get_buffer
av_packet_alloc, av_packet_free, av_packet_unref
av_strerror
// SWScale:
sws_getContext, sws_scale, sws_freeContext
// SWResample:
swr_alloc_set_opts, swr_init, swr_convert
```

### f) `GS/MultiISA.cpp` (Multi-ISA Dispatch)
**External headers:**
- `<cpuinfo.h>` (cpuinfo — CPU feature detection)

**Functions:**
```c
cpuinfo_initialize()
cpuinfo_get_processor(0)
cpuinfo_get_package(0)
// cpuinfo types: struct cpuinfo_processor, struct cpuinfo_package
// cpuinfo fields: cpuinfo_vendor, cpuinfo_uarch, cpuinfo_features
```

### g) `GS/GSXXH.cpp` (XXHash)
**External headers:**
- `<xxhash.h>` (XXHash — xxhash library)

**Functions:**
```cpp
XXH3_hashLong_64b_internal(...)
XXH3_64bits_update(...)
XXH3_64bits_digest(...)
```

**Types:** `XXH3_state_t`, `xxh_u8`

---

## 6. COMMON RENDERER (`GS/Renderers/Common/`)

### Files
`GSDevice.cpp/.h`, `GSFastList.h`, `GSRenderer.cpp/.h`, `GSShaderEnums.h`, `GSTexture.cpp/.h`, `GSVertex.h`

### External Dependencies
- `imgui.h` — **GSDevice.cpp** includes `<imgui.h>` for debug OSD rendering

---

## 7. HW RENDERER (`GS/Renderers/HW/`)

### Files (11 files)
`GSHwHack.cpp/.h`, `GSRendererHW.cpp/.h`, `GSRendererHWMultiISA.cpp`, `GSTextureCache.cpp/.h`, `GSTextureReplacementLoaders.cpp/.h`, `GSTextureReplacements.cpp/.h`, `GSVertexHW.h`

### External Dependencies in `GSTextureReplacementLoaders.cpp`
- `<png.h>` (libpng — texture replacement loading)
- `<csetjmp>` (for png error handling)

**C functions from libpng (reading):**
```c
png_create_read_struct(PNG_LIBPNG_VER_STRING, ...)
png_create_info_struct(png_ptr)
png_init_io(png_ptr, fp)
png_read_info(png_ptr, info_ptr)
png_get_IHDR(png_ptr, info_ptr, ...)
png_set_expand(png_ptr)
png_set_gray_to_rgb(png_ptr)
png_set_palette_to_rgb(png_ptr)
png_read_image(png_ptr, row_pointers)
png_read_end(png_ptr, info_ptr)
png_destroy_read_struct(&png_ptr, &info_ptr, nullptr)
setjmp(png_jmpbuf(png_ptr))  // for error recovery
```

---

## SUMMARY TABLE

| External Library | Files Using It | Category | Rust Equivalent |
|-----------------|----------------|----------|-----------------|
| **Vulkan SDK** (`vulkan/vulkan.h`) | 17 files (Vulkan/) | Graphics | `ash` crate |
| **VMA** (vk_mem_alloc.h) | 17 files (Vulkan/) via VKLoader.h | Memory alloc | `gpu-allocator` crate |
| **shaderc** (shaderc/shaderc.h) | VKShaderCache.cpp | Shader compilation | `shaderc-sys` or pre-compile |
| **D3D11** (d3d11_1.h) | 8 files (DX11/) | Graphics | `wgpu` / `ash` (skip) |
| **D3D12** (directx/d3d12.h) | 11 files (DX12/) | Graphics | `wgpu` / `ash` (skip) |
| **DXGI** (dxgi1_5.h) | DX11+DX12 files | Graphics | skip (Vulkan only) |
| **D3D12MA** | GSDevice12 | Memory alloc | skip (Vulkan only) |
| **OpenGL/glad** | 22 files (OpenGL/) | Graphics | skip (Vulkan only) |
| **WIL** (wil/com.h) | DX11+DX12+GSDevice.cpp | Win32 COM helpers | `windows` crate |
| **libpng** (png.h) | GSPng.cpp, GSTextureReplacementLoaders.cpp | Image I/O | `image` crate |
| **zlib** (zlib.h) | GSPng.cpp (via png), GSUtil.cpp | Compression | `flate2` crate |
| **LZMA SDK** (7zCrc.h, Xz.h, Alloc.h) | GSLzma.cpp, GSDump.cpp | Compression | `xz2` / `lzma-rs` |
| **Zstd** (zstd.h) | GSLzma.cpp, GSDump.cpp | Compression | `zstd` crate |
| **cpuinfo** (cpuinfo.h) | MultiISA.cpp | CPU detection | `raw-cpuid` |
| **XXHash** (xxhash.h) | GSXXH.cpp | Hashing | `xxhash` crate |
| **FFmpeg** (libav*) | GSCapture.cpp | Video recording | `ffmpeg-next` or skip |
| **imgui** (imgui.h) | GSDevice.cpp (Common/) | Debug UI | Slint (built-in) |
| **X11** (X11/Xlib.h) | VKSwapChain.cpp, GLContextEGLX11.cpp | Linux display | skip (Windows) |

## RUST PORTING COMPLEXITY

| Module | Files | Ext Deps | Complexity | Strategy |
|--------|-------|----------|------------|----------|
| **Vulkan Renderer** | 17 files | vulkan, VMA, shaderc | **Highest** (~15K LOC) | Port manually with `ash` + `gpu-allocator` |
| **DX11 Renderer** | 8 files | d3d11, dxgi, WIL | **Skip** (not needed) | Drop, use Vulkan only |
| **DX12 Renderer** | 11 files | d3d12, dxgi, D3D12MA, WIL | **Skip** (not needed) | Drop, use Vulkan only |
| **OpenGL Renderer** | 22 files | glad, WGL/EGL | **Skip** (not needed) | Drop, use Vulkan only |
| **Common Renderer** | 8 files | imgui | **Medium** | Port base class, drop imgui |
| **HW Renderer** | 11 files | png | **High** (~10K LOC) | Core logic, port manually |
| **GSPng** | 2 files | libpng, zlib | **Low** | Replace with `image` crate |
| **GSLzma** | 2 files | LZMA SDK, zstd | **Low** | Replace with `xz2` + `zstd` |
| **GSDump** | 2 files | LZMA SDK, zstd | **Low** | Replace with `xz2` + `zstd` |
| **GSCapture** | 2 files | FFmpeg | **Medium** | Optional, port if needed |
| **GSXXH** | 2 files | xxhash | **Low** | Replace with `xxhash` crate |
| **MultiISA** | 2 files | cpuinfo | **Low** | Replace with `raw-cpuid` |

**Total external C/C++ function calls:** ~350+ functions across all files
**Total external types/structs/enums:** ~500+ types
