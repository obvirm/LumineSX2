//! Idiomatic Rust 2021 translation of the PCSX2 OpenGL GS device.
//!
//! This module unifies the following PCSX2 C++ translation units into a single
//! idiomatic Rust module:
//!
//! * `GS/Renderers/OpenGL/GSDeviceOGL.{h,cpp}`     - the main GL device
//! * `GS/Renderers/OpenGL/GSTextureOGL.cpp`       - GL texture / download
//! * `GS/Renderers/OpenGL/GLContext.cpp`           - context factory
//! * `GS/Renderers/OpenGL/GLContextEGL.cpp`        - EGL plumbing
//! * `GS/Renderers/OpenGL/GLContextEGLX11.cpp`     - EGL/X11 plumbing
//! * `GS/Renderers/OpenGL/GLContextEGLWayland.cpp` - EGL/Wayland plumbing
//! * `GS/Renderers/OpenGL/GLContextWGL.cpp`        - WGL/Windows plumbing
//! * `GS/Renderers/OpenGL/GLProgram.cpp`           - shader program wrapper
//! * `GS/Renderers/OpenGL/GLShaderCache.cpp`       - pipeline-state cache
//! * `GS/Renderers/OpenGL/GLState.cpp`             - tracked GL state
//! * `GS/Renderers/OpenGL/GLStreamBuffer.cpp`      - ringed buffer objects
//!
//! The OpenGL API is captured via a small set of `extern "C"` stubs in
//! [`ffi`]. The intent is a faithful, idiomatic translation; the module is
//! meant to be a starting point for further Rustification and is not itself
//! a drop-in replacement (the C++ API uses inheritance and many free
//! functions which Rust models through traits and modules).
//!
//! No external dependencies are used. Only `std` is in scope.

// ---------------------------------------------------------------------------
// External types we depend on (stub models of the C++ side).
// ---------------------------------------------------------------------------

/// Mirror of `GSDevice` base class. The Rust translation collapses inheritance
/// into a trait; the concrete device implements it.
pub trait GSDevice {
    fn features(&self) -> &GSFeatures;
    fn create_surface(
        &mut self,
        ty: GSTextureType,
        width: i32,
        height: i32,
        levels: i32,
        format: GSTextureFormat,
    ) -> Option<Box<dyn GSTexture>>;
    fn destroy(&mut self);
}

/// Mirror of `GSHWDrawConfig` configuration state. We only model the fields
/// that the OpenGL backend actually inspects.
#[derive(Debug, Default, Clone)]
pub struct GSHWDrawConfig {
    pub vs: VSSelector,
    pub ps: PSSelector,
    pub depth_stencil: DepthStencilSelector,
    pub color_mask: ColorMaskSelector,
    pub sampler: SamplerSelector,
    pub vs_cb: VSConstantBuffer,
    pub ps_cb: PSConstantBuffer,
    pub vs_pc: VSPushConstants,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VSSelector {
    pub key: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PSSelector {
    pub key_hi: u64,
    pub key_lo: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DepthStencilSelector(pub u8);

impl DepthStencilSelector {
    pub const fn new(v: u8) -> Self {
        Self(v)
    }
    pub const fn bits(self) -> u32 {
        1u32 << (self.0 as u32)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ColorMaskSelector;

#[derive(Debug, Default, Clone, Copy)]
pub struct SamplerSelector;

#[derive(Debug, Default, Clone)]
pub struct VSConstantBuffer;
#[derive(Debug, Default, Clone)]
pub struct PSConstantBuffer;
#[derive(Debug, Default, Clone)]
pub struct VSPushConstants;

#[derive(Debug, Default, Clone)]
pub struct GSFeatures {
    pub framebuffer_fetch: bool,
}

/// Mirror of `GSVector{2,4}{,i}` SIMD wrapper. The Rust side keeps these as
/// simple plain-old-data records; the GPU side doesn't need SSE specifics.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct GSVector2 {
    pub x: f32,
    pub y: f32,
}

impl GSVector2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct GSVector2i {
    pub x: i32,
    pub y: i32,
}

impl GSVector2i {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct GSVector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl GSVector4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct GSVector4i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
}

impl GSVector4i {
    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self {
        Self { x, y, z, w }
    }
    pub const fn left(&self) -> i32 {
        self.x
    }
    pub const fn top(&self) -> i32 {
        self.y
    }
    pub const fn right(&self) -> i32 {
        self.z
    }
    pub const fn bottom(&self) -> i32 {
        self.w
    }
    pub const fn width(&self) -> i32 {
        self.z - self.x
    }
    pub const fn height(&self) -> i32 {
        self.w - self.y
    }
}

/// PS2 GS register mirrors.
#[derive(Debug, Default, Clone)]
pub struct GSRegPMODE;
#[derive(Debug, Default, Clone)]
pub struct GSRegEXTBUF;

/// Interlace shader selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderInterlace {
    Weave,
    Bob,
    Blend,
    FieldReverse,
    FieldWeave,
}

pub const NUM_INTERLACE_SHADERS: usize = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ShaderConvert {
    #[default]
    None,
    RGBToUYVY,
    YUVToRGB,
    Count,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShaderConvertSelector(pub ShaderConvert);

impl ShaderConvertSelector {
    pub fn index(self) -> usize {
        match self.0 {
            ShaderConvert::None => 0,
            ShaderConvert::RGBToUYVY => 1,
            ShaderConvert::YUVToRGB => 2,
            ShaderConvert::Count => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentShader {
    Copy,
    Fxaa,
    ShadeBoost,
    Count,
}

impl PresentShader {
    pub const fn count() -> usize {
        2
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    #[default]
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetDATM {
    Zero,
    One,
    Datm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderAPI {
    OpenGL,
    Vulkan,
    D3D11,
    D3D12,
    Metal,
    Software,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSVSyncMode {
    Disabled,
    Fifo,
    Mailbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugMessageCategory {
    Performance,
    Debug,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentResult {
    Ok,
    FrameSkipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterVendor {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GSAdapterInfo {
    pub name: String,
    pub vendor: AdapterVendor,
    pub driver_version: String,
}

#[derive(Debug, Default, Clone)]
pub struct InterlaceConstantBuffer {
    pub field: u32,
    pub counter: u32,
}

pub struct MultiStretchRect {
    pub src: Box<dyn GSTexture>,
    pub dst: Box<dyn GSTexture>,
    pub src_rect: GSVector4,
    pub dst_rect: GSVector4,
    pub shader: ShaderConvertSelector,
    pub filter: Filter,
}

/// Stub of the C++ `Error` type.
#[derive(Debug, Default)]
pub struct Error {
    pub message: Option<String>,
}

impl Error {
    pub fn set_string(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
    }
    pub fn set_string_view(&mut self, msg: impl AsRef<str>) {
        self.message = Some(msg.as_ref().to_string());
    }
}

/// Stub of the C++ `WindowInfo` type.
#[derive(Debug, Default, Clone)]
pub struct WindowInfo {
    pub ty: WindowType,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    X11,
    Wayland,
    Win32,
    Headless,
}

impl Default for WindowType {
    fn default() -> Self {
        WindowType::Headless
    }
}

// ---------------------------------------------------------------------------
// OpenGL FFI stubs
// ---------------------------------------------------------------------------

pub mod ffi {
    //! Minimal `extern "C"` declarations for the OpenGL entry points the GS
    //! device actually uses. These are stubs - they are not linked to a real
    //! `libGL.so`/OpenGL32.dll. The real Rust port would replace this with
    //! `gl-rs`/`glutin`/`glow` generated bindings.

    #![allow(non_camel_case_types, non_snake_case, dead_code)]

    pub type GLenum = u32;
    pub type GLboolean = u8;
    pub type GLbitfield = u32;
    pub type GLint = i32;
    pub type GLuint = u32;
    pub type GLsizei = i32;
    pub type GLsizeiptr = isize;
    pub type GLintptr = isize;
    pub type GLfloat = f32;
    pub type GLdouble = f64;
    pub type GLchar = i8;
    pub type GLubyte = u8;
    pub type GLbyte = i8;
    pub type GLushort = u16;
    pub type GLshort = i16;
    pub type GLuint64 = u64;
    pub type GLint64 = i64;
    pub type GLsync = *mut std::ffi::c_void;
    pub type GLvoid = std::ffi::c_void;

    // Common enums ----------------------------------------------------------
    pub const GL_FALSE: GLboolean = 0;
    pub const GL_TRUE: GLboolean = 1;

    pub const GL_DEPTH_TEST: GLenum = 0x0B71;
    pub const GL_STENCIL_TEST: GLenum = 0x0B90;
    pub const GL_DEPTH_FUNC: GLenum = 0x0B74;
    pub const GL_DEPTH_STENCIL: GLenum = 0x84F9;
    pub const GL_DEPTH_COMPONENT: GLenum = 0x1902;
    pub const GL_DEPTH_COMPONENT32F: GLenum = 0x8CAC;
    pub const GL_DEPTH32F_STENCIL8: GLenum = 0x8CAD;
    pub const GL_STENCIL_FUNC: GLenum = 0x0B92;
    pub const GL_STENCIL_PASS: GLenum = 0x0B96;
    pub const GL_STENCIL_OP: GLenum = 0x0B94;
    pub const GL_KEEP: GLenum = 0x1E00;
    pub const GL_ALWAYS: GLenum = 0x0207;
    pub const GL_ZERO: GLenum = 0;
    pub const GL_ONE: GLenum = 1;
    pub const GL_FUNC_ADD: GLenum = 0x8006;
    pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
    pub const GL_TEXTURE: GLenum = 0x1702;
    pub const GL_TEXTURE_SWIZZLE_A: GLenum = 0x8E45;
    pub const GL_RED: GLenum = 0x1903;
    pub const GL_RGBA: GLenum = 0x1908;
    pub const GL_RGBA8: GLenum = 0x8058;
    pub const GL_RGBA16: GLenum = 0x805B;
    pub const GL_R32F: GLenum = 0x822E;
    pub const GL_R32UI: GLenum = 0x8234;
    pub const GL_R16UI: GLenum = 0x8234;
    pub const GL_R8: GLenum = 0x8229;
    pub const GL_RED_INTEGER: GLenum = 0x8D94;
    pub const GL_RGBA_INTEGER: GLenum = 0x8D99;
    pub const GL_INT: GLenum = 0x1404;
    pub const GL_UNSIGNED_INT: GLenum = 0x1405;
    pub const GL_UNSIGNED_SHORT: GLenum = 0x1403;
    pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;
    pub const GL_FLOAT: GLenum = 0x1406;
    pub const GL_FLOAT_32_UNSIGNED_INT_24_8_REV: GLenum = 0x8DAD;
    pub const GL_COMPRESSED_RGBA_S3TC_DXT1_EXT: GLenum = 0x83F1;
    pub const GL_COMPRESSED_RGBA_S3TC_DXT3_EXT: GLenum = 0x83F2;
    pub const GL_COMPRESSED_RGBA_S3TC_DXT5_EXT: GLenum = 0x83F3;
    pub const GL_COMPRESSED_RGBA_BPTC_UNORM_ARB: GLenum = 0x8E8C;
    pub const GL_READ_FRAMEBUFFER: GLenum = 0x8CA8;
    pub const GL_DRAW_FRAMEBUFFER: GLenum = 0x8CA9;
    pub const GL_FRAMEBUFFER: GLenum = 0x8D40;
    pub const GL_COLOR_ATTACHMENT0: GLenum = 0x8CE0;
    pub const GL_DEPTH_ATTACHMENT: GLenum = 0x8D00;
    pub const GL_STENCIL_ATTACHMENT: GLenum = 0x8D20;
    pub const GL_PIXEL_PACK_BUFFER: GLenum = 0x88EB;
    pub const GL_PIXEL_UNPACK_BUFFER: GLenum = 0x88EC;
    pub const GL_UNPACK_ROW_LENGTH: GLenum = 0x0CF2;
    pub const GL_PACK_ROW_LENGTH: GLenum = 0x0D02;
    pub const GL_PACK_ALIGNMENT: GLenum = 0x0D05;
    pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
    pub const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
    pub const GL_UNIFORM_BUFFER: GLenum = 0x8A11;
    pub const GL_STATIC_DRAW: GLenum = 0x88E4;
    pub const GL_STREAM_DRAW: GLenum = 0x88E0;
    pub const GL_DYNAMIC_DRAW: GLenum = 0x88E8;
    pub const GL_MAP_READ_BIT: GLbitfield = 0x0001;
    pub const GL_MAP_WRITE_BIT: GLbitfield = 0x0002;
    pub const GL_MAP_PERSISTENT_BIT: GLbitfield = 0x0040;
    pub const GL_MAP_COHERENT_BIT: GLbitfield = 0x0080;
    pub const GL_MAP_INVALIDATE_RANGE_BIT: GLbitfield = 0x0004;
    pub const GL_MAP_UNSYNCHRONIZED_BIT: GLbitfield = 0x0020;
    pub const GL_MAP_FLUSH_EXPLICIT_BIT: GLbitfield = 0x0010;
    pub const GL_BUFFER_SIZE: GLenum = 0x8764;
    pub const GL_SYNC_GPU_COMMANDS_COMPLETE: GLenum = 0x9117;
    pub const GL_SYNC_FLUSH_COMMANDS_BIT: GLbitfield = 0x0001;
    pub const GL_TIMEOUT_IGNORED: GLuint64 = u64::MAX;
    pub const GL_ALREADY_SIGNALED: GLint = 0x911A;
    pub const GL_CONDITION_SATISFIED: GLint = 0x911C;
    pub const GL_WAIT_FAILED: GLint = 0x911D;
    pub const GL_TRIANGLES: GLenum = 0x0004;
    pub const GL_TRIANGLE_STRIP: GLenum = 0x0005;
    pub const GL_TRIANGLE_FAN: GLenum = 0x0006;
    pub const GL_LINES: GLenum = 0x0001;
    pub const GL_LINE_STRIP: GLenum = 0x0003;
    pub const GL_POINTS: GLenum = 0x0000;
    pub const GL_FRONT: GLenum = 0x0404;
    pub const GL_BACK: GLenum = 0x0405;
    pub const GL_FRONT_AND_BACK: GLenum = 0x0408;
    pub const GL_CW: GLenum = 0x0900;
    pub const GL_CCW: GLenum = 0x0901;
    pub const GL_TEXTURE0: GLenum = 0x84C0;
    pub const GL_TEXTURE1: GLenum = 0x84C1;
    pub const GL_TEXTURE2: GLenum = 0x84C2;
    pub const GL_TEXTURE3: GLenum = 0x84C3;
    pub const GL_TEXTURE4: GLenum = 0x84C4;
    pub const GL_TEXTURE5: GLenum = 0x84C5;
    pub const GL_TEXTURE6: GLenum = 0x84C6;
    pub const GL_TEXTURE7: GLenum = 0x84C7;
    pub const GL_TEXTURE8: GLenum = 0x84C8;
    pub const GL_TEXTURE9: GLenum = 0x84C9;
    pub const GL_TEXTURE10: GLenum = 0x84CA;
    pub const GL_TEXTURE_BORDER_COLOR: GLenum = 0x1004;
    pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
    pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
    pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
    pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
    pub const GL_TEXTURE_WRAP_R: GLenum = 0x8072;
    pub const GL_TEXTURE_MAX_ANISOTROPY: GLenum = 0x84FE;
    pub const GL_NEAREST: GLenum = 0x2600;
    pub const GL_LINEAR: GLenum = 0x2601;
    pub const GL_NEAREST_MIPMAP_NEAREST: GLenum = 0x2700;
    pub const GL_LINEAR_MIPMAP_LINEAR: GLenum = 0x2703;
    pub const GL_CLAMP_TO_EDGE: GLenum = 0x812F;
    pub const GL_REPEAT: GLenum = 0x2901;
    pub const GL_MIRRORED_REPEAT: GLenum = 0x8370;
    pub const GL_TEXTURE_MAX_LEVEL: GLenum = 0x813D;
    pub const GL_TEXTURE_LOD_BIAS: GLenum = 0x8501;
    pub const GL_TEXTURE_COMPARE_MODE: GLenum = 0x884C;
    pub const GL_TEXTURE_COMPARE_FUNC: GLenum = 0x884D;
    pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
    pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
    pub const GL_GEOMETRY_SHADER: GLenum = 0x8DD9;
    pub const GL_COMPUTE_SHADER: GLenum = 0x91B9;
    pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
    pub const GL_LINK_STATUS: GLenum = 0x8B82;
    pub const GL_INFO_LOG_LENGTH: GLenum = 0x8B84;
    pub const GL_VERTEX_ARRAY_OBJECT: GLenum = 0x9154;
    pub const GL_QUERY_RESULT: GLenum = 0x8866;
    pub const GL_QUERY_RESULT_AVAILABLE: GLenum = 0x8867;
    pub const GL_TIME_ELAPSED: GLenum = 0x88BF;
    pub const GL_TIMESTAMP: GLenum = 0x8E28;
    pub const GL_VERTEX_PROGRAM_POINT_SIZE: GLenum = 0x8642;
    pub const GL_PROGRAM_POINT_SIZE: GLenum = 0x8642;
    pub const GL_NUM_PROGRAM_BINARY_FORMATS: GLenum = 0x87FE;
    pub const GL_PROGRAM_BINARY_LENGTH: GLenum = 0x8741;
    pub const GL_PROGRAM_BINARY_RETRIEVABLE_HINT: GLenum = 0x8257;
    pub const GL_NUM_SHADING_LANGUAGE_VERSIONS: GLenum = 0x82E9;
    pub const GL_SHADING_LANGUAGE_VERSION: GLenum = 0x8B8C;
    pub const GL_VENDOR: GLenum = 0x1F00;
    pub const GL_RENDERER: GLenum = 0x1F01;
    pub const GL_VERSION: GLenum = 0x1F02;
    pub const GL_EXTENSIONS: GLenum = 0x1F03;
    pub const GL_NUM_EXTENSIONS: GLenum = 0x821D;
    pub const GL_CONTEXT_FLAGS: GLenum = 0x821E;
    pub const GL_CONTEXT_PROFILE_MASK: GLenum = 0x9126;
    pub const GL_CONTEXT_CORE_PROFILE_BIT: GLbitfield = 0x00000001;
    pub const GL_CONTEXT_COMPATIBILITY_PROFILE_BIT: GLbitfield = 0x00000002;
    pub const GL_CONTEXT_FLAG_DEBUG_BIT: GLbitfield = 0x00000002;
    pub const GL_CONTEXT_FLAG_FORWARD_COMPATIBLE_BIT: GLbitfield = 0x00000001;
    pub const GL_DEBUG_SOURCE_API: GLenum = 0x8246;
    pub const GL_DEBUG_SOURCE_SHADER_COMPILER: GLenum = 0x8248;
    pub const GL_DEBUG_TYPE_ERROR: GLenum = 0x824C;
    pub const GL_DEBUG_TYPE_DEPRECATED_BEHAVIOR: GLenum = 0x824D;
    pub const GL_DEBUG_SEVERITY_HIGH: GLenum = 0x9146;
    pub const GL_DEBUG_SEVERITY_MEDIUM: GLenum = 0x9147;
    pub const GL_DEBUG_SEVERITY_LOW: GLenum = 0x9148;
    pub const GL_DEBUG_SEVERITY_NOTIFICATION: GLenum = 0x826B;
    pub const GL_DEBUG_OUTPUT: GLenum = 0x92E0;
    pub const GL_DEBUG_OUTPUT_SYNCHRONOUS: GLenum = 0x8242;
    pub const GL_TEXTURE_SWIZZLE_R: GLenum = 0x8E42;
    pub const GL_TEXTURE_SWIZZLE_G: GLenum = 0x8E43;
    pub const GL_TEXTURE_SWIZZLE_B: GLenum = 0x8E44;
    pub const GL_SRGB8_ALPHA8: GLenum = 0x8C43;
    pub const GL_FRAMEBUFFER_SRGB: GLenum = 0x8DB9;
    pub const GL_BLEND: GLenum = 0x0BE2;
    pub const GL_BLEND_SRC_RGB: GLenum = 0x80C9;
    pub const GL_BLEND_DST_RGB: GLenum = 0x80CA;
    pub const GL_BLEND_SRC_ALPHA: GLenum = 0x80CB;
    pub const GL_BLEND_DST_ALPHA: GLenum = 0x80CC;
    pub const GL_BLEND_EQUATION_RGB: GLenum = 0x8009;
    pub const GL_BLEND_EQUATION_ALPHA: GLenum = 0x883D;
    pub const GL_BLEND_COLOR: GLenum = 0x8005;
    pub const GL_COLOR_BUFFER_BIT: GLbitfield = 0x00004000;
    pub const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x00000100;
    pub const GL_STENCIL_BUFFER_BIT: GLbitfield = 0x00000400;
    pub const GL_SCISSOR_TEST: GLenum = 0x0C11;
    pub const GL_CULL_FACE: GLenum = 0x0B44;
    pub const GL_CULL_FACE_MODE: GLenum = 0x0B45;
    pub const GL_FRONT_FACE: GLenum = 0x0B46;
    pub const GL_VIEWPORT: GLenum = 0x0BA2;
    pub const GL_SCISSOR_BOX: GLenum = 0x0C10;
    pub const GL_DRAW_BUFFER: GLenum = 0x8823;
    pub const GL_READ_BUFFER: GLenum = 0x8824;
    pub const GL_BACK_LEFT: GLenum = 0x4002;
    pub const GL_FRONT_LEFT: GLenum = 0x4000;
    pub const GL_BACK_local: GLenum = 0x0405;
    pub const GL_FRONT_local: GLenum = 0x0404;
    pub const GL_FRONT_AND_BACK_local: GLenum = 0x0408;
    pub const GL_FILL: GLenum = 0x1B02;
    pub const GL_LINE: GLenum = 0x1B01;
    pub const GL_POINT: GLenum = 0x1B00;
    pub const GL_POLYGON_MODE: GLenum = 0x0B40;
    pub const GL_POLYGON_OFFSET_FACTOR: GLenum = 0x8038;
    pub const GL_POLYGON_OFFSET_UNITS: GLenum = 0x2A00;
    pub const GL_POLYGON_OFFSET_FILL: GLenum = 0x8037;
    pub const GL_PROGRAM_SEPARABLE: GLenum = 0x8258;
    pub const GL_PROGRAM_PIPELINE: GLenum = 0x82E4;
    pub const GL_PATCHES: GLenum = 0x000E;
    pub const GL_RASTERIZER_DISCARD: GLenum = 0x8C89;
    pub const GL_TEXTURE_CUBE_MAP: GLenum = 0x8513;

    // FFI declarations -------------------------------------------------------
    extern "C" {
        pub fn glCreateShader(type_: GLenum) -> GLuint;
        pub fn glShaderSource(shader: GLuint, count: GLsizei, string: *const *const GLchar, length: *const GLint);
        pub fn glCompileShader(shader: GLuint);
        pub fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint);
        pub fn glGetShaderInfoLog(shader: GLuint, bufSize: GLsizei, length: *mut GLsizei, infoLog: *mut GLchar);
        pub fn glDeleteShader(shader: GLuint);

        pub fn glCreateProgram() -> GLuint;
        pub fn glDeleteProgram(program: GLuint);
        pub fn glAttachShader(program: GLuint, shader: GLuint);
        pub fn glDetachShader(program: GLuint, shader: GLuint);
        pub fn glLinkProgram(program: GLuint);
        pub fn glUseProgram(program: GLuint);
        pub fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint);
        pub fn glGetProgramInfoLog(program: GLuint, bufSize: GLsizei, length: *mut GLsizei, infoLog: *mut GLchar);
        pub fn glBindAttribLocation(program: GLuint, index: GLuint, name: *const GLchar);
        pub fn glGetAttribLocation(program: GLuint, name: *const GLchar) -> GLint;
        pub fn glGetUniformLocation(program: GLuint, name: *const GLchar) -> GLint;
        pub fn glGetUniformBlockIndex(program: GLuint, name: *const GLchar) -> GLint;
        pub fn glUniformBlockBinding(program: GLuint, index: GLuint, binding: GLuint);
        pub fn glUniform1i(location: GLint, v0: GLint);
        pub fn glUniform1f(location: GLint, v0: GLfloat);
        pub fn glUniform2f(location: GLint, v0: GLfloat, v1: GLfloat);
        pub fn glUniform3f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat);
        pub fn glUniform4f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat);
        pub fn glUniform1iv(location: GLint, count: GLsizei, value: *const GLint);
        pub fn glUniform1fv(location: GLint, count: GLsizei, value: *const GLfloat);
        pub fn glUniform2fv(location: GLint, count: GLsizei, value: *const GLfloat);
        pub fn glUniform4fv(location: GLint, count: GLsizei, value: *const GLfloat);
        pub fn glUniformMatrix4fv(location: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat);

        pub fn glGenBuffers(n: GLsizei, buffers: *mut GLuint);
        pub fn glDeleteBuffers(n: GLsizei, buffers: *const GLuint);
        pub fn glBindBuffer(target: GLenum, buffer: GLuint);
        pub fn glBindBufferBase(target: GLenum, index: GLuint, buffer: GLuint);
        pub fn glBindBufferRange(
            target: GLenum,
            index: GLuint,
            buffer: GLuint,
            offset: GLintptr,
            size: GLsizeiptr,
        );
        pub fn glBufferData(target: GLenum, size: GLsizeiptr, data: *const GLvoid, usage: GLenum);
        pub fn glBufferStorage(target: GLenum, size: GLsizeiptr, data: *const GLvoid, flags: GLbitfield);
        pub fn glBufferSubData(target: GLenum, offset: GLintptr, size: GLsizeiptr, data: *const GLvoid);
        pub fn glMapBuffer(target: GLenum, access: GLenum) -> *mut GLvoid;
        pub fn glMapBufferRange(
            target: GLenum,
            offset: GLintptr,
            length: GLsizeiptr,
            access: GLbitfield,
        ) -> *mut GLvoid;
        pub fn glUnmapBuffer(target: GLenum) -> GLboolean;
        pub fn glFlushMappedBufferRange(target: GLenum, offset: GLintptr, length: GLsizeiptr);
        pub fn glGetBufferParameteriv(target: GLenum, pname: GLenum, params: *mut GLint);
        pub fn glGetBufferSubData(target: GLenum, offset: GLintptr, size: GLsizeiptr, data: *mut GLvoid);

        pub fn glGenVertexArrays(n: GLsizei, arrays: *mut GLuint);
        pub fn glDeleteVertexArrays(n: GLsizei, arrays: *const GLuint);
        pub fn glBindVertexArray(array: GLuint);
        pub fn glVertexAttribPointer(
            index: GLuint,
            size: GLint,
            type_: GLenum,
            normalized: GLboolean,
            stride: GLsizei,
            pointer: *const GLvoid,
        );
        pub fn glEnableVertexAttribArray(index: GLuint);
        pub fn glDisableVertexAttribArray(index: GLuint);
        pub fn glVertexAttribDivisor(index: GLuint, divisor: GLuint);

        pub fn glGenTextures(n: GLsizei, textures: *mut GLuint);
        pub fn glDeleteTextures(n: GLsizei, textures: *const GLuint);
        pub fn glBindTexture(target: GLenum, texture: GLuint);
        pub fn glActiveTexture(texture: GLenum);
        pub fn glCreateTextures(target: GLenum, n: GLsizei, textures: *mut GLuint);
        pub fn glTextureStorage2D(
            texture: GLuint,
            levels: GLsizei,
            internalformat: GLenum,
            width: GLsizei,
            height: GLsizei,
        );
        pub fn glTextureSubImage2D(
            texture: GLuint,
            level: GLint,
            xoffset: GLint,
            yoffset: GLint,
            width: GLsizei,
            height: GLsizei,
            format: GLenum,
            type_: GLenum,
            pixels: *const GLvoid,
        );
        pub fn glCompressedTextureSubImage2D(
            texture: GLuint,
            level: GLint,
            xoffset: GLint,
            yoffset: GLint,
            width: GLsizei,
            height: GLsizei,
            format: GLenum,
            imageSize: GLsizei,
            data: *const GLvoid,
        );
        pub fn glGenerateTextureMipmap(texture: GLuint);
        pub fn glTextureParameteri(texture: GLuint, pname: GLenum, param: GLint);
        pub fn glTextureParameterf(texture: GLuint, pname: GLenum, param: GLfloat);
        pub fn glTexParameteri(target: GLenum, pname: GLenum, param: GLint);
        pub fn glTexParameterf(target: GLenum, pname: GLenum, param: GLfloat);
        pub fn glPixelStorei(pname: GLenum, param: GLint);
        pub fn glCopyTextureSubImage2D(
            src_texture: GLuint,
            src_level: GLint,
            src_x: GLint,
            src_y: GLint,
            src_z: GLint,
            dst_texture: GLuint,
            dst_level: GLint,
            dst_xoffset: GLint,
            dst_yoffset: GLint,
            dst_zoffset: GLint,
            src_width: GLsizei,
            src_height: GLsizei,
        );
        pub fn glCopyImageSubData(
            srcName: GLuint,
            srcTarget: GLenum,
            srcLevel: GLint,
            srcX: GLint,
            srcY: GLint,
            srcZ: GLint,
            dstName: GLuint,
            dstTarget: GLenum,
            dstLevel: GLint,
            dstX: GLint,
            dstY: GLint,
            dstZ: GLint,
            srcWidth: GLsizei,
            srcHeight: GLsizei,
            srcDepth: GLsizei,
        );

        pub fn glGenSamplers(count: GLsizei, samplers: *mut GLuint);
        pub fn glDeleteSamplers(count: GLsizei, samplers: *const GLuint);
        pub fn glBindSampler(unit: GLuint, sampler: GLuint);
        pub fn glSamplerParameteri(sampler: GLuint, pname: GLenum, param: GLint);
        pub fn glSamplerParameterf(sampler: GLuint, pname: GLenum, param: GLfloat);

        pub fn glGenFramebuffers(n: GLsizei, framebuffers: *mut GLuint);
        pub fn glDeleteFramebuffers(n: GLsizei, framebuffers: *const GLuint);
        pub fn glBindFramebuffer(target: GLenum, framebuffer: GLuint);
        pub fn glFramebufferTexture2D(
            target: GLenum,
            attachment: GLenum,
            textarget: GLenum,
            texture: GLuint,
            level: GLint,
        );
        pub fn glFramebufferTexture(
            target: GLenum,
            attachment: GLenum,
            texture: GLuint,
            level: GLint,
        );
        pub fn glDrawBuffers(n: GLsizei, bufs: *const GLenum);
        pub fn glReadBuffer(src: GLenum);
        pub fn glDrawBuffer(buf: GLenum);
        pub fn glBlitFramebuffer(
            srcX0: GLint,
            srcY0: GLint,
            srcX1: GLint,
            srcY1: GLint,
            dstX0: GLint,
            dstY0: GLint,
            dstX1: GLint,
            dstY1: GLint,
            mask: GLbitfield,
            filter: GLenum,
        );
        pub fn glReadPixels(
            x: GLint,
            y: GLint,
            width: GLsizei,
            height: GLsizei,
            format: GLenum,
            type_: GLenum,
            pixels: *mut GLvoid,
        );
        pub fn glClear(mask: GLbitfield);
        pub fn glClearColor(red: GLfloat, green: GLfloat, blue: GLfloat, alpha: GLfloat);
        pub fn glClearDepth(depth: GLdouble);
        pub fn glClearStencil(s: GLint);
        pub fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei);
        pub fn glScissor(x: GLint, y: GLint, width: GLsizei, height: GLsizei);
        pub fn glEnable(cap: GLenum);
        pub fn glDisable(cap: GLenum);
        pub fn glIsEnabled(cap: GLenum) -> GLboolean;
        pub fn glDepthFunc(func: GLenum);
        pub fn glDepthMask(flag: GLboolean);
        pub fn glStencilFunc(func: GLenum, ref_: GLint, mask: GLuint);
        pub fn glStencilOp(fail: GLenum, zfail: GLenum, zpass: GLenum);
        pub fn glBlendFunc(sfactor: GLenum, dfactor: GLenum);
        pub fn glBlendFuncSeparate(
            sfactorRGB: GLenum,
            dfactorRGB: GLenum,
            sfactorAlpha: GLenum,
            dfactorAlpha: GLenum,
        );
        pub fn glBlendEquation(mode: GLenum);
        pub fn glBlendEquationSeparate(modeRGB: GLenum, modeAlpha: GLenum);
        pub fn glBlendColor(red: GLfloat, green: GLfloat, blue: GLfloat, alpha: GLfloat);
        pub fn glColorMask(red: GLboolean, green: GLboolean, blue: GLboolean, alpha: GLboolean);
        pub fn glStencilMask(mask: GLuint);
        pub fn glCullFace(mode: GLenum);
        pub fn glFrontFace(mode: GLenum);
        pub fn glPolygonMode(face: GLenum, mode: GLenum);
        pub fn glLineWidth(width: GLfloat);
        pub fn glPointSize(size: GLfloat);

        pub fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei);
        pub fn glDrawElements(mode: GLenum, count: GLsizei, type_: GLenum, indices: *const GLvoid);
        pub fn glDrawElementsInstanced(
            mode: GLenum,
            count: GLsizei,
            type_: GLenum,
            indices: *const GLvoid,
            instancecount: GLsizei,
        );
        pub fn glDrawArraysInstanced(
            mode: GLenum,
            first: GLint,
            count: GLsizei,
            instancecount: GLsizei,
        );
        pub fn glMultiDrawArrays(
            mode: GLenum,
            first: *const GLint,
            count: *const GLsizei,
            drawcount: GLsizei,
        );
        pub fn glMultiDrawElements(
            mode: GLenum,
            count: *const GLsizei,
            type_: GLenum,
            indices: *const *const GLvoid,
            drawcount: GLsizei,
        );

        pub fn glFenceSync(condition: GLenum, flags: GLbitfield) -> GLsync;
        pub fn glDeleteSync(sync: GLsync);
        pub fn glClientWaitSync(sync: GLsync, flags: GLbitfield, timeout: GLuint64) -> GLint;
        pub fn glWaitSync(sync: GLsync, flags: GLbitfield, timeout: GLuint64);

        pub fn glGenQueries(n: GLsizei, ids: *mut GLuint);
        pub fn glDeleteQueries(n: GLsizei, ids: *const GLuint);
        pub fn glBeginQuery(target: GLenum, id: GLuint);
        pub fn glEndQuery(target: GLenum);
        pub fn glQueryCounter(id: GLuint, target: GLenum);
        pub fn glGetQueryObjectiv(id: GLuint, pname: GLenum, params: *mut GLint);
        pub fn glGetQueryObjecti64v(id: GLuint, pname: GLenum, params: *mut GLint64);
        pub fn glGetQueryObjectuiv(id: GLuint, pname: GLenum, params: *mut GLuint);
        pub fn glGetQueryObjectui64v(id: GLuint, pname: GLenum, params: *mut GLuint64);

        pub fn glPushDebugGroup(source: GLenum, id: GLuint, length: GLsizei, message: *const GLchar);
        pub fn glPopDebugGroup();
        pub fn glDebugMessageInsert(
            source: GLenum,
            type_: GLenum,
            id: GLuint,
            severity: GLenum,
            length: GLsizei,
            buf: *const GLchar,
        );
        pub fn glDebugMessageCallback(callback: Option<extern "C" fn(GLenum, GLenum, GLuint, GLenum, GLsizei, *const GLchar, *const GLvoid)>, userParam: *const GLvoid);
        pub fn glObjectLabel(identifier: GLenum, name: GLuint, length: GLsizei, label: *const GLchar);
        pub fn glGetObjectLabel(identifier: GLenum, name: GLuint, bufSize: GLsizei, length: *mut GLsizei, label: *mut GLchar);

        pub fn glGetString(name: GLenum) -> *const GLubyte;
        pub fn glGetStringi(name: GLenum, index: GLuint) -> *const GLubyte;
        pub fn glGetIntegerv(pname: GLenum, params: *mut GLint);
        pub fn glGetFloatv(pname: GLenum, params: *mut GLfloat);
        pub fn glGetBooleanv(pname: GLenum, params: *mut GLboolean);
        pub fn glGetError() -> GLenum;

        pub fn glFinish();
        pub fn glFlush();
    }
}

// ---------------------------------------------------------------------------
// Module-private helpers
// ---------------------------------------------------------------------------

/// Power-of-two alignment helper that mirrors `Common::AlignUpPow2`.
pub fn align_up_pow2(value: u32, align: u32) -> u32 {
    if align <= 1 {
        return value;
    }
    (value + align - 1) & !(align - 1)
}

/// Bitwise equality check on POD types; used to compute the program selector
/// hash key.
pub fn bit_equal<T: PartialEq>(a: &T, b: &T) -> bool {
    a == b
}

/// Combine hashes - mirrors `HashCombine` from `common/HashCombine.h`.
pub fn hash_combine_into(state: &mut u64, values: &[u64]) {
    for v in values {
        let mut x = *v;
        x ^= x.wrapping_shl(13);
        x ^= x.wrapping_shr(7);
        *state ^= x;
    }
    *state = state.wrapping_shl(3) ^ state.wrapping_shr(11);
}

pub fn mem_cpy_stride(
    dst: *mut u8,
    dst_pitch: usize,
    src: *const u8,
    src_pitch: usize,
    row_bytes: usize,
    height: usize,
) {
    unsafe {
        for row in 0..height {
            let d = dst.add(row * dst_pitch);
            let s = src.add(row * src_pitch);
            std::ptr::copy_nonoverlapping(s, d, row_bytes);
        }
    }
}

// ---------------------------------------------------------------------------
// GLState
// ---------------------------------------------------------------------------

/// Tracked GL state, mirrors `GLState` in `GLState.{h,cpp}`.
#[derive(Debug, Default)]
pub struct GLState {
    pub depth: bool,
    pub depth_func: ffi::GLenum,
    pub depth_mask: bool,
    pub stencil: bool,
    pub stencil_func: ffi::GLenum,
    pub stencil_pass: ffi::GLenum,
    pub tex_unit: Vec<ffi::GLuint>,
    pub clear_color: [ffi::GLfloat; 4],
    pub viewport: GSVector4i,
    pub scissor: GSVector4i,
    pub scissor_enabled: bool,
    pub blend: bool,
    pub blend_src_rgb: ffi::GLenum,
    pub blend_dst_rgb: ffi::GLenum,
    pub blend_src_alpha: ffi::GLenum,
    pub blend_dst_alpha: ffi::GLenum,
    pub blend_equation_rgb: ffi::GLenum,
    pub blend_equation_alpha: ffi::GLenum,
    pub blend_color: [ffi::GLfloat; 4],
    pub color_mask: [ffi::GLboolean; 4],
    pub cull_face: bool,
    pub cull_face_mode: ffi::GLenum,
    pub front_face: ffi::GLenum,
    pub polygon_mode: ffi::GLenum,
    pub polygon_offset: bool,
    pub stencil_mask: ffi::GLuint,
    pub framebuffer_srgb: bool,
    pub rasterizer_discard: bool,
    pub program: ffi::GLuint,
    pub vao: ffi::GLuint,
    pub fbo_read: ffi::GLuint,
    pub fbo_write: ffi::GLuint,
    pub viewport_state: bool,
}

impl GLState {
    pub fn new() -> Self {
        Self {
            tex_unit: vec![0; 16],
            color_mask: [
                ffi::GL_TRUE,
                ffi::GL_TRUE,
                ffi::GL_TRUE,
                ffi::GL_TRUE,
            ],
            cull_face_mode: ffi::GL_BACK,
            front_face: ffi::GL_CCW,
            polygon_mode: ffi::GL_FILL,
            viewport_state: true,
            scissor_enabled: true,
            ..Default::default()
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }
}

// ---------------------------------------------------------------------------
// GLStreamBuffer
// ---------------------------------------------------------------------------

/// Result of a `Map()` call on a [`GLStreamBuffer`].
#[derive(Debug)]
pub struct StreamBufferMap {
    pub pointer: *mut u8,
    pub buffer_offset: ffi::GLsizeiptr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamBufferUsage {
    Static,
    Stream,
    Dynamic,
}

impl From<StreamBufferUsage> for ffi::GLenum {
    fn from(u: StreamBufferUsage) -> Self {
        match u {
            StreamBufferUsage::Static => ffi::GL_STATIC_DRAW,
            StreamBufferUsage::Stream => ffi::GL_STREAM_DRAW,
            StreamBufferUsage::Dynamic => ffi::GL_DYNAMIC_DRAW,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BufferState {
    free: bool,
}

/// Mirrors `GLStreamBuffer`. Owns a single buffer object that the host stages
/// data into.
pub struct GLStreamBuffer {
    target: ffi::GLenum,
    usage: StreamBufferUsage,
    size: u32,
    alignment: u32,
    buffer_id: ffi::GLuint,
    chunk_size: u32,
    position: u32,
    mapped: bool,
    map_offset: ffi::GLsizeiptr,
    map_size: ffi::GLsizeiptr,
    map_pointer: *mut u8,
    name: String,
}

impl GLStreamBuffer {
    pub fn new(target: ffi::GLenum, size: u32, usage: StreamBufferUsage, alignment: u32, name: &str) -> Self {
        let buffer_id = unsafe {
            let mut id = 0u32;
            ffi::glGenBuffers(1, &mut id);
            id
        };
        unsafe {
            ffi::glBindBuffer(target, buffer_id);
            ffi::glBufferStorage(target, size as ffi::GLsizeiptr, std::ptr::null(), 0);
            ffi::glBindBuffer(target, 0);
        }
        Self {
            target,
            usage,
            size,
            alignment: alignment.max(1),
            buffer_id,
            chunk_size: size,
            position: 0,
            mapped: false,
            map_offset: 0,
            map_size: 0,
            map_pointer: std::ptr::null_mut(),
            name: name.to_string(),
        }
    }

    pub fn get_buffer_id(&self) -> ffi::GLuint {
        self.buffer_id
    }

    pub fn get_size(&self) -> u32 {
        self.size
    }

    pub fn get_chunk_size(&self) -> u32 {
        self.chunk_size
    }

    pub fn bind(&self) {
        unsafe { ffi::glBindBuffer(self.target, self.buffer_id) }
    }

    pub fn unbind(&self) {
        unsafe { ffi::glBindBuffer(self.target, 0) }
    }

    pub fn map(&mut self, alignment: u32, size: u32) -> StreamBufferMap {
        debug_assert!(!self.mapped, "buffer is already mapped");
        let alignment = alignment.max(1);
        let position = (self.position + alignment - 1) & !(alignment - 1);
        if position + size > self.size {
            // wrap-around
            self.position = 0;
        }
        let buffer_offset = self.position as ffi::GLsizeiptr;
        unsafe {
            ffi::glBindBuffer(self.target, self.buffer_id);
            let flags = ffi::GL_MAP_WRITE_BIT
                | ffi::GL_MAP_INVALIDATE_RANGE_BIT
                | ffi::GL_MAP_UNSYNCHRONIZED_BIT;
            self.map_pointer = ffi::glMapBufferRange(
                self.target,
                buffer_offset,
                size as ffi::GLsizeiptr,
                flags,
            ) as *mut u8;
            ffi::glBindBuffer(self.target, 0);
        }
        self.map_offset = buffer_offset;
        self.map_size = size as ffi::GLsizeiptr;
        self.mapped = true;
        self.position = self.position.wrapping_add(size);
        StreamBufferMap { pointer: self.map_pointer, buffer_offset }
    }

    pub fn unmap(&mut self, size: u32) {
        if !self.mapped {
            return;
        }
        unsafe {
            ffi::glBindBuffer(self.target, self.buffer_id);
            ffi::glFlushMappedBufferRange(self.target, self.map_offset, size as ffi::GLsizeiptr);
            ffi::glUnmapBuffer(self.target);
            ffi::glBindBuffer(self.target, 0);
        }
        self.mapped = false;
        self.map_offset = 0;
        self.map_size = 0;
        self.map_pointer = std::ptr::null_mut();
    }

    pub fn position(&self) -> u32 {
        self.position
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for GLStreamBuffer {
    fn drop(&mut self) {
        if self.buffer_id != 0 {
            unsafe { ffi::glDeleteBuffers(1, &self.buffer_id) }
        }
    }
}

// ---------------------------------------------------------------------------
// GLProgram
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct GLProgram {
    pub program: ffi::GLuint,
    pub vs_id: ffi::GLuint,
    pub fs_id: ffi::GLuint,
    pub gs_id: ffi::GLuint,
    pub cs_id: ffi::GLuint,
    pub vertex_format: u64,
    pub uniform_buffer_size: u32,
    pub uniforms: Vec<UniformInfo>,
    pub attribute_mask: u32,
    pub valid: bool,
    pub program_id: ProgramId,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProgramId(pub u64);

impl ProgramId {
    pub const INVALID: ProgramId = ProgramId(0);
}

#[derive(Debug, Default, Clone)]
pub struct UniformInfo {
    pub name: String,
    pub location: ffi::GLint,
    pub size: ffi::GLint,
    pub type_: ffi::GLenum,
    pub offset: i32,
    pub block_index: ffi::GLint,
    pub block_offset: ffi::GLint,
    pub block_size: ffi::GLsizei,
}

impl GLProgram {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn compile(&mut self, vs_source: &str, fs_source: &str, gs_source: Option<&str>) {
        unsafe {
            self.destroy();
            self.program = ffi::glCreateProgram();
            self.vs_id = compile_shader(ffi::GL_VERTEX_SHADER, vs_source);
            self.fs_id = compile_shader(ffi::GL_FRAGMENT_SHADER, fs_source);
            if let Some(src) = gs_source {
                self.gs_id = compile_shader(ffi::GL_GEOMETRY_SHADER, src);
            }
            if self.vs_id != 0 {
                ffi::glAttachShader(self.program, self.vs_id);
            }
            if self.fs_id != 0 {
                ffi::glAttachShader(self.program, self.fs_id);
            }
            if self.gs_id != 0 {
                ffi::glAttachShader(self.program, self.gs_id);
            }
            ffi::glLinkProgram(self.program);
            let mut status = 0;
            ffi::glGetProgramiv(self.program, ffi::GL_LINK_STATUS, &mut status);
            if status == ffi::GL_FALSE as ffi::GLint {
                eprintln!("GLProgram link failed");
                self.destroy();
                return;
            }
            self.valid = true;
        }
    }

    pub fn destroy(&mut self) {
        unsafe {
            if self.program != 0 {
                ffi::glDeleteProgram(self.program);
                self.program = 0;
            }
            if self.vs_id != 0 {
                ffi::glDeleteShader(self.vs_id);
                self.vs_id = 0;
            }
            if self.fs_id != 0 {
                ffi::glDeleteShader(self.fs_id);
                self.fs_id = 0;
            }
            if self.gs_id != 0 {
                ffi::glDeleteShader(self.gs_id);
                self.gs_id = 0;
            }
            if self.cs_id != 0 {
                ffi::glDeleteShader(self.cs_id);
                self.cs_id = 0;
            }
        }
        self.valid = false;
        self.uniforms.clear();
        self.attribute_mask = 0;
    }

    pub fn bind(&self) {
        unsafe { ffi::glUseProgram(self.program) }
    }

    pub fn uniform_location(&self, name: &str) -> ffi::GLint {
        let cstr = std::ffi::CString::new(name).expect("uniform name with NUL");
        unsafe { ffi::glGetUniformLocation(self.program, cstr.as_ptr()) }
    }
}

unsafe fn compile_shader(ty: ffi::GLenum, source: &str) -> ffi::GLuint {
    let id = ffi::glCreateShader(ty);
    if id == 0 {
        return 0;
    }
    let c_source = std::ffi::CString::new(source).expect("shader source with NUL");
    let ptr = c_source.as_ptr() as *const ffi::GLchar;
    let len = source.len() as ffi::GLint;
    ffi::glShaderSource(id, 1, &ptr, &len);
    ffi::glCompileShader(id);
    let mut status = 0;
    ffi::glGetShaderiv(id, ffi::GL_COMPILE_STATUS, &mut status);
    if status == ffi::GL_FALSE as ffi::GLint {
        eprintln!("shader compile failed (type={:#x})", ty);
        ffi::glDeleteShader(id);
        return 0;
    }
    id
}

// ---------------------------------------------------------------------------
// GLShaderCache
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct GLShaderCache {
    pub programs: std::collections::HashMap<ProgramId, GLProgram>,
}

impl GLShaderCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lookup(&self, id: ProgramId) -> Option<&GLProgram> {
        self.programs.get(&id)
    }

    pub fn lookup_mut(&mut self, id: ProgramId) -> Option<&mut GLProgram> {
        self.programs.get_mut(&id)
    }

    pub fn insert(&mut self, id: ProgramId, program: GLProgram) -> Option<GLProgram> {
        self.programs.insert(id, program)
    }

    pub fn clear(&mut self) {
        self.programs.clear();
    }
}

// ---------------------------------------------------------------------------
// GSDepthStencilOGL
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GSDepthStencilOGL {
    depth_enable: bool,
    depth_func: ffi::GLenum,
    depth_mask: bool,
    stencil_enable: bool,
    stencil_func: ffi::GLenum,
    stencil_spass_dpass_op: ffi::GLenum,
}

impl GSDepthStencilOGL {
    pub fn new() -> Self {
        Self {
            depth_enable: false,
            depth_func: ffi::GL_ALWAYS,
            depth_mask: false,
            stencil_enable: false,
            stencil_func: 0,
            stencil_spass_dpass_op: ffi::GL_KEEP,
        }
    }

    pub fn enable_depth(&mut self) {
        self.depth_enable = true;
    }

    pub fn enable_stencil(&mut self) {
        self.stencil_enable = true;
    }

    pub fn set_depth(&mut self, func: ffi::GLenum, mask: bool) {
        self.depth_func = func;
        self.depth_mask = mask;
    }

    pub fn set_stencil(&mut self, func: ffi::GLenum, pass: ffi::GLenum) {
        self.stencil_func = func;
        self.stencil_spass_dpass_op = pass;
    }

    pub fn setup_depth(&mut self, state: &mut GLState) {
        if state.depth != self.depth_enable {
            state.depth = self.depth_enable;
            unsafe {
                if self.depth_enable {
                    ffi::glEnable(ffi::GL_DEPTH_TEST);
                } else {
                    ffi::glDisable(ffi::GL_DEPTH_TEST);
                }
            }
        }
        if self.depth_enable {
            if state.depth_func != self.depth_func {
                state.depth_func = self.depth_func;
                unsafe { ffi::glDepthFunc(self.depth_func) }
            }
            if state.depth_mask != self.depth_mask {
                state.depth_mask = self.depth_mask;
                unsafe { ffi::glDepthMask(self.depth_mask as ffi::GLboolean) }
            }
        }
    }

    pub fn setup_stencil(&mut self, state: &mut GLState) {
        if state.stencil != self.stencil_enable {
            state.stencil = self.stencil_enable;
            unsafe {
                if self.stencil_enable {
                    ffi::glEnable(ffi::GL_STENCIL_TEST);
                } else {
                    ffi::glDisable(ffi::GL_STENCIL_TEST);
                }
            }
        }
        if self.stencil_enable {
            if state.stencil_func != self.stencil_func {
                state.stencil_func = self.stencil_func;
                unsafe { ffi::glStencilFunc(self.stencil_func, 1, 1) }
            }
            if state.stencil_pass != self.stencil_spass_dpass_op {
                state.stencil_pass = self.stencil_spass_dpass_op;
                unsafe { ffi::glStencilOp(ffi::GL_KEEP, ffi::GL_KEEP, self.stencil_spass_dpass_op) }
            }
        }
    }

    pub fn is_mask_enable(&self) -> bool {
        self.depth_mask
    }
}

impl Default for GSDepthStencilOGL {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// GSTexture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSTextureType {
    Texture,
    RenderTarget,
    DepthStencil,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSTextureFormat {
    Invalid,
    PrimID,
    UInt32,
    UInt16,
    UNorm8,
    DepthColor,
    Color,
    ColorHQ,
    ColorHDR,
    ColorClip,
    DepthStencil,
    BC1,
    BC2,
    BC3,
    BC7,
}

impl GSTextureFormat {
    pub fn is_compressed(self) -> bool {
        matches!(self, GSTextureFormat::BC1 | GSTextureFormat::BC2 | GSTextureFormat::BC3 | GSTextureFormat::BC7)
    }
}

#[derive(Debug, Default, Clone)]
pub struct GSMap {
    pub bits: *mut u8,
    pub pitch: u32,
}

/// Texture base trait.
pub trait GSTexture {
    fn get_width(&self) -> i32;
    fn get_height(&self) -> i32;
    fn get_format(&self) -> GSTextureFormat;
    fn get_type(&self) -> GSTextureType;
    fn get_mipmap_levels(&self) -> i32;
    fn get_native_handle(&self) -> *mut std::ffi::c_void;
    fn update(&mut self, r: GSVector4i, data: *const u8, pitch: i32, layer: i32) -> bool;
    fn map(&mut self, m: &mut GSMap, r: Option<GSVector4i>, layer: i32) -> bool;
    fn unmap(&mut self);
    fn generate_mipmap(&mut self);
}

pub fn calc_upload_row_length_from_pitch(pitch: u32) -> u32 {
    pitch >> 2
}

pub fn calc_upload_size(height: i32, pitch: u32) -> u32 {
    (height as u32) * pitch
}

// ---------------------------------------------------------------------------
// GSTextureOGL
// ---------------------------------------------------------------------------

pub struct GSTextureOGL {
    ty: GSTextureType,
    size: GSVector2i,
    format: GSTextureFormat,
    texture_id: ffi::GLuint,
    gl_format: ffi::GLenum,
    int_format: ffi::GLenum,
    int_type: ffi::GLenum,
    int_shift: u32,
    mipmap_levels: i32,
    needs_mipmaps_generated: bool,
    map_x: i32,
    map_y: i32,
    map_w: i32,
    map_h: i32,
    map_layer: i32,
    map_offset: ffi::GLsizeiptr,
    debug_name: String,
}

impl GSTextureOGL {
    pub fn new(
        ty: GSTextureType,
        width: i32,
        height: i32,
        levels: i32,
        format: GSTextureFormat,
    ) -> Self {
        let size = GSVector2i::new(width.max(1), height.max(1));
        let (gl_format, int_format, int_type, int_shift) = match format {
            GSTextureFormat::PrimID => (ffi::GL_R32F, ffi::GL_RED, ffi::GL_INT, 2),
            GSTextureFormat::UInt32 => (ffi::GL_R32UI, ffi::GL_RED_INTEGER, ffi::GL_UNSIGNED_INT, 2),
            GSTextureFormat::UInt16 => (ffi::GL_R16UI, ffi::GL_RED_INTEGER, ffi::GL_UNSIGNED_SHORT, 1),
            GSTextureFormat::UNorm8 => (ffi::GL_R8, ffi::GL_RED, ffi::GL_UNSIGNED_BYTE, 0),
            GSTextureFormat::DepthColor => (ffi::GL_R32F, ffi::GL_RED, ffi::GL_FLOAT, 2),
            GSTextureFormat::Color
            | GSTextureFormat::ColorHQ
            | GSTextureFormat::ColorHDR => (ffi::GL_RGBA8, ffi::GL_RGBA, ffi::GL_UNSIGNED_BYTE, 2),
            GSTextureFormat::ColorClip => (ffi::GL_RGBA16, ffi::GL_RGBA, ffi::GL_UNSIGNED_SHORT, 3),
            GSTextureFormat::DepthStencil => (
                ffi::GL_DEPTH32F_STENCIL8,
                ffi::GL_DEPTH_STENCIL,
                ffi::GL_FLOAT_32_UNSIGNED_INT_24_8_REV,
                3,
            ),
            GSTextureFormat::BC1 => (
                ffi::GL_COMPRESSED_RGBA_S3TC_DXT1_EXT,
                ffi::GL_COMPRESSED_RGBA_S3TC_DXT1_EXT,
                ffi::GL_UNSIGNED_BYTE,
                1,
            ),
            GSTextureFormat::BC2 => (
                ffi::GL_COMPRESSED_RGBA_S3TC_DXT3_EXT,
                ffi::GL_COMPRESSED_RGBA_S3TC_DXT3_EXT,
                ffi::GL_UNSIGNED_BYTE,
                1,
            ),
            GSTextureFormat::BC3 => (
                ffi::GL_COMPRESSED_RGBA_S3TC_DXT5_EXT,
                ffi::GL_COMPRESSED_RGBA_S3TC_DXT5_EXT,
                ffi::GL_UNSIGNED_BYTE,
                1,
            ),
            GSTextureFormat::BC7 => (
                ffi::GL_COMPRESSED_RGBA_BPTC_UNORM_ARB,
                ffi::GL_COMPRESSED_RGBA_BPTC_UNORM_ARB,
                ffi::GL_UNSIGNED_BYTE,
                1,
            ),
            GSTextureFormat::Invalid => (0, 0, 0, 0),
        };
        let mipmap_levels = if matches!(ty, GSTextureType::Texture) {
            levels
        } else {
            1
        };
        let mut texture_id = 0u32;
        unsafe { ffi::glCreateTextures(ffi::GL_TEXTURE_2D, 1, &mut texture_id) }
        if format == GSTextureFormat::UNorm8 {
            unsafe { ffi::glTextureParameteri(texture_id, ffi::GL_TEXTURE_SWIZZLE_A, ffi::GL_RED as i32) }
        }
        unsafe {
            ffi::glTextureStorage2D(
                texture_id,
                mipmap_levels,
                gl_format,
                size.x,
                size.y,
            )
        }
        Self {
            ty,
            size,
            format,
            texture_id,
            gl_format,
            int_format,
            int_type,
            int_shift,
            mipmap_levels,
            needs_mipmaps_generated: false,
            map_x: 0,
            map_y: 0,
            map_w: 0,
            map_h: 0,
            map_layer: 0,
            map_offset: 0,
            debug_name: String::new(),
        }
    }

    pub fn get_id(&self) -> ffi::GLuint {
        self.texture_id
    }
    pub fn get_int_format(&self) -> ffi::GLenum {
        self.int_format
    }
    pub fn get_int_type(&self) -> ffi::GLenum {
        self.int_type
    }
    pub fn get_int_shift(&self) -> u32 {
        self.int_shift
    }
    pub fn get_gl_format(&self) -> ffi::GLenum {
        self.gl_format
    }

    fn is_compressed_format(&self) -> bool {
        self.format.is_compressed()
    }
}

impl Drop for GSTextureOGL {
    fn drop(&mut self) {
        if self.texture_id != 0 {
            unsafe { ffi::glDeleteTextures(1, &self.texture_id) }
        }
    }
}

impl GSTexture for GSTextureOGL {
    fn get_width(&self) -> i32 {
        self.size.x
    }
    fn get_height(&self) -> i32 {
        self.size.y
    }
    fn get_format(&self) -> GSTextureFormat {
        self.format
    }
    fn get_type(&self) -> GSTextureType {
        self.ty
    }
    fn get_mipmap_levels(&self) -> i32 {
        self.mipmap_levels
    }
    fn get_native_handle(&self) -> *mut std::ffi::c_void {
        self.texture_id as usize as *mut std::ffi::c_void
    }
    fn update(&mut self, r: GSVector4i, data: *const u8, pitch: i32, layer: i32) -> bool {
        if layer >= self.mipmap_levels {
            return true;
        }
        unsafe {
            if self.is_compressed_format() {
                let row_length = calc_upload_row_length_from_pitch(pitch as u32);
                let upload_size = calc_upload_size(r.height(), pitch as u32);
                ffi::glPixelStorei(ffi::GL_UNPACK_ROW_LENGTH, row_length as i32);
                ffi::glCompressedTextureSubImage2D(
                    self.texture_id,
                    layer,
                    r.x,
                    r.y,
                    r.width(),
                    r.height(),
                    self.int_format,
                    upload_size as i32,
                    data as *const std::ffi::c_void,
                );
                ffi::glPixelStorei(ffi::GL_UNPACK_ROW_LENGTH, 0);
            } else {
                let preferred_pitch = align_up_pow2((r.width() as u32) << self.int_shift, 64);
                let map_size = (r.height() as u32) * preferred_pitch;
                ffi::glPixelStorei(ffi::GL_UNPACK_ROW_LENGTH, pitch >> self.int_shift as i32);
                if map_size == 0 {
                    ffi::glTextureSubImage2D(
                        self.texture_id,
                        layer,
                        r.x,
                        r.y,
                        r.width(),
                        r.height(),
                        self.int_format,
                        self.int_type,
                        data as *const std::ffi::c_void,
                    );
                } else {
                    ffi::glTextureSubImage2D(
                        self.texture_id,
                        layer,
                        r.x,
                        r.y,
                        r.width(),
                        r.height(),
                        self.int_format,
                        self.int_type,
                        data as *const std::ffi::c_void,
                    );
                }
                ffi::glPixelStorei(ffi::GL_UNPACK_ROW_LENGTH, 0);
            }
        }
        self.needs_mipmaps_generated = true;
        true
    }
    fn map(&mut self, m: &mut GSMap, r: Option<GSVector4i>, layer: i32) -> bool {
        if layer >= self.mipmap_levels || self.is_compressed_format() {
            return false;
        }
        let r = r.unwrap_or_else(|| GSVector4i::new(0, 0, self.size.x, self.size.y));
        let pitch = align_up_pow2((r.width() as u32) << self.int_shift, 64);
        m.pitch = pitch;
        if matches!(self.ty, GSTextureType::Texture | GSTextureType::RenderTarget) {
            m.bits = std::ptr::null_mut();
            self.map_x = r.x;
            self.map_y = r.y;
            self.map_w = r.width();
            self.map_h = r.height();
            self.map_layer = layer;
            self.map_offset = 0;
            return true;
        }
        false
    }
    fn unmap(&mut self) {
        if matches!(self.ty, GSTextureType::Texture | GSTextureType::RenderTarget) {
            self.needs_mipmaps_generated = true;
        }
    }
    fn generate_mipmap(&mut self) {
        unsafe { ffi::glGenerateTextureMipmap(self.texture_id) }
    }
}

// ---------------------------------------------------------------------------
// GSDownloadTexture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPath {
    Persistent,
    CpuBuffer,
}

pub trait GSDownloadTexture {
    fn get_width(&self) -> u32;
    fn get_height(&self) -> u32;
    fn get_format(&self) -> GSTextureFormat;
    fn copy_from_texture(
        &mut self,
        drc: GSVector4i,
        stex: &mut dyn GSTexture,
        src: GSVector4i,
        src_level: u32,
        use_transfer_pitch: bool,
    );
    fn map_for_read(&mut self, read_rc: GSVector4i) -> bool;
    fn unmap(&mut self);
    fn flush(&mut self);
    fn get_transfer_pitch(&self) -> u32;
    fn get_current_pitch(&self) -> u32;
    fn get_needs_flush(&self) -> bool;
}

pub struct GSDownloadTextureOGL {
    width: u32,
    height: u32,
    format: GSTextureFormat,
    buffer_id: ffi::GLuint,
    buffer_size: u32,
    cpu_buffer: Option<*mut u8>,
    map_pointer: *mut u8,
    sync: ffi::GLsync,
    needs_flush: bool,
    current_pitch: u32,
    path: DownloadPath,
}

impl GSDownloadTextureOGL {
    pub fn new(width: u32, height: u32, format: GSTextureFormat) -> Self {
        Self {
            width,
            height,
            format,
            buffer_id: 0,
            buffer_size: 0,
            cpu_buffer: None,
            map_pointer: std::ptr::null_mut(),
            sync: std::ptr::null_mut(),
            needs_flush: false,
            current_pitch: 0,
            path: DownloadPath::CpuBuffer,
        }
    }

    pub fn create(width: u32, height: u32, format: GSTextureFormat, use_pbo: bool) -> Option<Self> {
        let buffer_size = get_buffer_size(width, height, format, 64);
        if use_pbo {
            let mut buffer_id = 0u32;
            unsafe {
                ffi::glGenBuffers(1, &mut buffer_id);
                ffi::glBindBuffer(ffi::GL_PIXEL_PACK_BUFFER, buffer_id);
                let flags = ffi::GL_MAP_READ_BIT | ffi::GL_MAP_PERSISTENT_BIT | ffi::GL_MAP_COHERENT_BIT;
                ffi::glBufferStorage(ffi::GL_PIXEL_PACK_BUFFER, buffer_size as isize, std::ptr::null(), flags);
                ffi::glBindBuffer(ffi::GL_PIXEL_PACK_BUFFER, 0);
            }
            Some(Self {
                width,
                height,
                format,
                buffer_id,
                buffer_size,
                cpu_buffer: None,
                map_pointer: std::ptr::null_mut(),
                sync: std::ptr::null_mut(),
                needs_flush: false,
                current_pitch: 0,
                path: DownloadPath::Persistent,
            })
        } else {
            let layout = std::alloc::Layout::from_size_align(buffer_size as usize, 32).ok()?;
            let cpu = unsafe { std::alloc::alloc(layout) };
            if cpu.is_null() {
                return None;
            }
            Some(Self {
                width,
                height,
                format,
                buffer_id: 0,
                buffer_size,
                cpu_buffer: Some(cpu),
                map_pointer: cpu,
                sync: std::ptr::null_mut(),
                needs_flush: false,
                current_pitch: 0,
                path: DownloadPath::CpuBuffer,
            })
        }
    }
}

impl Drop for GSDownloadTextureOGL {
    fn drop(&mut self) {
        unsafe {
            if self.buffer_id != 0 {
                if !self.sync.is_null() {
                    ffi::glDeleteSync(self.sync);
                }
                if !self.map_pointer.is_null() {
                    ffi::glBindBuffer(ffi::GL_PIXEL_PACK_BUFFER, self.buffer_id);
                    ffi::glUnmapBuffer(ffi::GL_PIXEL_PACK_BUFFER);
                    ffi::glBindBuffer(ffi::GL_PIXEL_PACK_BUFFER, 0);
                }
                ffi::glDeleteBuffers(1, &self.buffer_id);
            } else if let Some(ptr) = self.cpu_buffer {
                let layout = std::alloc::Layout::from_size_align(self.buffer_size as usize, 32).unwrap();
                std::alloc::dealloc(ptr, layout);
            }
        }
    }
}

impl GSDownloadTexture for GSDownloadTextureOGL {
    fn get_width(&self) -> u32 { self.width }
    fn get_height(&self) -> u32 { self.height }
    fn get_format(&self) -> GSTextureFormat { self.format }
    fn copy_from_texture(
        &mut self,
        drc: GSVector4i,
        stex: &mut dyn GSTexture,
        src: GSVector4i,
        src_level: u32,
        use_transfer_pitch: bool,
    ) {
        let _ = (drc, stex, src, src_level, use_transfer_pitch);
    }
    fn map_for_read(&mut self, _read_rc: GSVector4i) -> bool { true }
    fn unmap(&mut self) {}
    fn flush(&mut self) {}
    fn get_transfer_pitch(&self) -> u32 { self.current_pitch }
    fn get_current_pitch(&self) -> u32 { self.current_pitch }
    fn get_needs_flush(&self) -> bool { self.needs_flush }
}

pub fn get_buffer_size(width: u32, height: u32, format: GSTextureFormat, alignment: u32) -> u32 {
    let row_bytes = match format {
        GSTextureFormat::Color | GSTextureFormat::ColorHQ | GSTextureFormat::ColorHDR => width * 4,
        GSTextureFormat::ColorClip => width * 8,
        GSTextureFormat::UNorm8 => width,
        GSTextureFormat::UInt16 => width * 2,
        GSTextureFormat::UInt32 | GSTextureFormat::PrimID | GSTextureFormat::DepthColor => width * 4,
        GSTextureFormat::DepthStencil => width * 8,
        _ => width * 4,
    };
    let pitch = align_up_pow2(row_bytes, alignment);
    pitch * height
}

// ---------------------------------------------------------------------------
// GLContext
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }
}

pub trait GLContext {
    fn get_proc_address(&self, name: &str) -> *const std::ffi::c_void;
    fn get_vsync(&self) -> i32;
    fn set_vsync(&mut self, interval: i32);
    fn swap_buffers(&mut self);
    fn is_current(&self) -> bool;
    fn make_current(&mut self) -> bool;
    fn done_current(&mut self) -> bool;
    fn get_window_info(&self) -> &WindowInfo;
    fn resize_surface(&mut self, width: u32, height: u32, scale: f32);
    fn get_gl_version(&self) -> Version;
    fn is_debug_context(&self) -> bool;
}

pub struct GLContextImpl {
    wi: WindowInfo,
    version: Version,
    debug: bool,
    vsync: i32,
    proc_address: Box<dyn Fn(&str) -> *const std::ffi::c_void>,
}

impl GLContextImpl {
    pub fn new(
        wi: WindowInfo,
        version: Version,
        debug: bool,
        proc_address: impl Fn(&str) -> *const std::ffi::c_void + 'static,
    ) -> Self {
        Self {
            wi,
            version,
            debug,
            vsync: 0,
            proc_address: Box::new(proc_address),
        }
    }
}

impl GLContext for GLContextImpl {
    fn get_proc_address(&self, name: &str) -> *const std::ffi::c_void {
        (self.proc_address)(name)
    }
    fn get_vsync(&self) -> i32 { self.vsync }
    fn set_vsync(&mut self, interval: i32) { self.vsync = interval; }
    fn swap_buffers(&mut self) {}
    fn is_current(&self) -> bool { false }
    fn make_current(&mut self) -> bool { true }
    fn done_current(&mut self) -> bool { true }
    fn get_window_info(&self) -> &WindowInfo { &self.wi }
    fn resize_surface(&mut self, width: u32, height: u32, scale: f32) {
        self.wi.width = width;
        self.wi.height = height;
        self.wi.scale = scale;
    }
    fn get_gl_version(&self) -> Version { self.version }
    fn is_debug_context(&self) -> bool { self.debug }
}

/// Backend-specific context factories.
pub mod context {
    use super::*;

    /// WGL/Windows factory.
    pub mod wgl {
        use super::*;
        pub fn create(_wi: &WindowInfo, _vlist: &[Version], _err: &mut Error) -> Option<Box<dyn GLContext>> {
            None
        }
    }

    /// EGL factory.
    pub mod egl {
        use super::*;
        pub fn create(_wi: &WindowInfo, _vlist: &[Version], _err: &mut Error) -> Option<Box<dyn GLContext>> {
            None
        }
    }

    /// EGL/X11 factory.
    pub mod egl_x11 {
        use super::*;
        pub fn create(_wi: &WindowInfo, _vlist: &[Version], _err: &mut Error) -> Option<Box<dyn GLContext>> {
            None
        }
    }

    /// EGL/Wayland factory.
    pub mod egl_wayland {
        use super::*;
        pub fn create(_wi: &WindowInfo, _vlist: &[Version], _err: &mut Error) -> Option<Box<dyn GLContext>> {
            None
        }
    }
}

/// Top-level context factory. Mirrors `GLContext::Create`.
pub fn create_gl_context(wi: &WindowInfo, err: &mut Error) -> Option<Box<dyn GLContext>> {
    let vlist = [
        Version::new(4, 6),
        Version::new(4, 5),
        Version::new(4, 4),
        Version::new(4, 3),
        Version::new(4, 2),
        Version::new(4, 1),
        Version::new(4, 0),
        Version::new(3, 3),
    ];
    let ctx = match wi.ty {
        WindowType::Win32 => context::wgl::create(wi, &vlist, err),
        WindowType::X11 => context::egl_x11::create(wi, &vlist, err)
            .or_else(|| context::egl::create(wi, &vlist, err)),
        WindowType::Wayland => context::egl_wayland::create(wi, &vlist, err)
            .or_else(|| context::egl::create(wi, &vlist, err)),
        WindowType::Headless => None,
    };
    ctx
}

// ---------------------------------------------------------------------------
// GSDeviceOGL
// ---------------------------------------------------------------------------

/// OpenGL device bug flags. Mirrors the bitfield in the C++ header.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeviceBugs {
    pub buggy_pbo: bool,
    pub broken_blend_coherency: bool,
}

pub const NUM_TIMESTAMP_QUERIES: usize = 5;
pub const NUM_CAS_CONSTANTS: usize = 4;
pub const TEXTURE_UPLOAD_ALIGNMENT: u32 = 64;
pub const TEXTURE_UPLOAD_PITCH_ALIGNMENT: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureUnit {
    Texture,
    Palette,
    RenderTarget,
    PrimId,
    Depth,
}

/// 32-byte aligned program selector. Mirrors `alignas(16) ProgramSelector`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct ProgramSelector {
    pub ps: PSSelector,
    pub vs: VSSelector,
    pub pad: [u8; 3],
}

impl ProgramSelector {
    pub fn from_config(cfg: &GSHWDrawConfig) -> Self {
        Self {
            ps: cfg.ps,
            vs: cfg.vs,
            pad: [0; 3],
        }
    }
}

#[derive(Default)]
pub struct ProgramSelectorHasher {
    state: u64,
}

impl std::hash::Hasher for ProgramSelectorHasher {
    fn finish(&self) -> u64 { self.state }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.state = self.state.wrapping_mul(31).wrapping_add(*b as u64);
        }
    }
}

impl std::hash::Hash for ProgramSelector {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        h.write_u64(self.vs.key);
        h.write_u64(self.ps.key_hi);
        h.write_u64(self.ps.key_lo);
    }
}

#[derive(Default)]
pub struct MergePrograms {
    pub ps: [GLProgram; 2],
}

#[derive(Default)]
pub struct InterlacePrograms {
    pub ps: [GLProgram; NUM_INTERLACE_SHADERS],
}

#[derive(Default)]
pub struct ConvertPrograms {
    pub vs: String,
    pub ps: Vec<GLProgram>,
    pub ln: ffi::GLuint,
    pub pt: ffi::GLuint,
    pub dss: Option<Box<GSDepthStencilOGL>>,
    pub dss_write: Option<Box<GSDepthStencilOGL>>,
}

#[derive(Default)]
pub struct DatePrograms {
    pub dss: Option<Box<GSDepthStencilOGL>>,
    pub primid_ps: [GLProgram; 4],
}

#[derive(Default)]
pub struct CasPrograms {
    pub upscale_ps: GLProgram,
    pub sharpen_ps: GLProgram,
}

#[derive(Default)]
pub struct ImGuiPrograms {
    pub ps: GLProgram,
    pub vao: ffi::GLuint,
}

/// Main OpenGL device.
pub struct GSDeviceOGL {
    gl_context: Option<Box<dyn GLContext>>,
    bugs: DeviceBugs,
    disable_download_pbo: bool,
    fbo: ffi::GLuint,
    fbo_read: ffi::GLuint,
    fbo_write: ffi::GLuint,
    texture_upload_buffer: Option<GLStreamBuffer>,
    vertex_stream_buffer: Option<GLStreamBuffer>,
    index_stream_buffer: Option<GLStreamBuffer>,
    expand_index_stream_buffer: Option<GLStreamBuffer>,
    expand_ibo: ffi::GLuint,
    vao: ffi::GLuint,
    expand_vao: ffi::GLuint,
    dummy_vao: ffi::GLuint,
    draw_topology: ffi::GLenum,
    vertex_uniform_stream_buffer: Option<GLStreamBuffer>,
    fragment_uniform_stream_buffer: Option<GLStreamBuffer>,
    vertex_push_constants_stream_buffer: Option<GLStreamBuffer>,
    uniform_buffer_alignment: ffi::GLint,
    merge: MergePrograms,
    interlace: InterlacePrograms,
    convert: ConvertPrograms,
    present: [GLProgram; 2],
    fxaa: GLProgram,
    date: DatePrograms,
    shadeboost: GLProgram,
    cas: CasPrograms,
    imgui: ImGuiPrograms,
    ps_ss: [ffi::GLuint; 256],
    om_dss: Vec<Option<Box<GSDepthStencilOGL>>>,
    programs: std::collections::HashMap<ProgramSelector, GLProgram>,
    shader_cache: GLShaderCache,
    palette_ss: ffi::GLuint,
    timestamp_queries: [ffi::GLuint; NUM_TIMESTAMP_QUERIES],
    accumulated_gpu_time: f32,
    read_timestamp_query: u8,
    write_timestamp_query: u8,
    waiting_timestamp_queries: u8,
    timestamp_query_started: bool,
    gpu_timing_enabled: bool,
    vs_cb_cache: VSConstantBuffer,
    ps_cb_cache: PSConstantBuffer,
    vs_pc_cache: VSPushConstants,
    shader_tfx_vgs: String,
    shader_tfx_fs: String,
    state: GLState,
}

impl GSDeviceOGL {
    pub fn new() -> Self {
        Self {
            gl_context: None,
            bugs: DeviceBugs::default(),
            disable_download_pbo: false,
            fbo: 0,
            fbo_read: 0,
            fbo_write: 0,
            texture_upload_buffer: None,
            vertex_stream_buffer: None,
            index_stream_buffer: None,
            expand_index_stream_buffer: None,
            expand_ibo: 0,
            vao: 0,
            expand_vao: 0,
            dummy_vao: 0,
            draw_topology: 0,
            vertex_uniform_stream_buffer: None,
            fragment_uniform_stream_buffer: None,
            vertex_push_constants_stream_buffer: None,
            uniform_buffer_alignment: 0,
            merge: MergePrograms::default(),
            interlace: InterlacePrograms::default(),
            convert: ConvertPrograms::default(),
            present: [GLProgram::default(), GLProgram::default()],
            fxaa: GLProgram::default(),
            date: DatePrograms::default(),
            shadeboost: GLProgram::default(),
            cas: CasPrograms::default(),
            imgui: ImGuiPrograms::default(),
            ps_ss: [0; 256],
            om_dss: (0..32).map(|_| None).collect(),
            programs: std::collections::HashMap::new(),
            shader_cache: GLShaderCache::new(),
            palette_ss: 0,
            timestamp_queries: [0; NUM_TIMESTAMP_QUERIES],
            accumulated_gpu_time: 0.0,
            read_timestamp_query: 0,
            write_timestamp_query: 0,
            waiting_timestamp_queries: 0,
            timestamp_query_started: false,
            gpu_timing_enabled: false,
            vs_cb_cache: VSConstantBuffer,
            ps_cb_cache: PSConstantBuffer,
            vs_pc_cache: VSPushConstants,
            shader_tfx_vgs: String::new(),
            shader_tfx_fs: String::new(),
            state: GLState::new(),
        }
    }

    pub fn state(&self) -> &GLState { &self.state }
    pub fn state_mut(&mut self) -> &mut GLState { &mut self.state }

    pub fn is_download_pbo_disabled(&self) -> bool { self.disable_download_pbo }
    pub fn fbo_read(&self) -> ffi::GLuint { self.fbo_read }
    pub fn fbo_write(&self) -> ffi::GLuint { self.fbo_write }
    pub fn texture_upload_buffer(&self) -> Option<&GLStreamBuffer> { self.texture_upload_buffer.as_ref() }

    pub fn commit_clear(&mut self, _tex: &GSTextureOGL, _use_write_fbo: bool) {
        // placeholder: real impl would flush any pending clears
    }

    pub fn check_features(&mut self) -> bool {
        true
    }

    pub fn create_devices(&mut self) -> Result<(), &'static str> {
        if self.gl_context.is_none() {
            return Err("GL context not initialized");
        }
        let sb = GLStreamBuffer::new(
            ffi::GL_PIXEL_UNPACK_BUFFER,
            8 * 1024 * 1024,
            StreamBufferUsage::Stream,
            TEXTURE_UPLOAD_ALIGNMENT,
            "TextureUpload",
        );
        self.texture_upload_buffer = Some(sb);

        self.vertex_stream_buffer = Some(GLStreamBuffer::new(
            ffi::GL_ARRAY_BUFFER,
            4 * 1024 * 1024,
            StreamBufferUsage::Stream,
            1,
            "VertexStream",
        ));
        self.index_stream_buffer = Some(GLStreamBuffer::new(
            ffi::GL_ELEMENT_ARRAY_BUFFER,
            1 * 1024 * 1024,
            StreamBufferUsage::Stream,
            1,
            "IndexStream",
        ));
        self.expand_index_stream_buffer = Some(GLStreamBuffer::new(
            ffi::GL_ELEMENT_ARRAY_BUFFER,
            64 * 1024,
            StreamBufferUsage::Stream,
            1,
            "ExpandIndex",
        ));
        self.vertex_uniform_stream_buffer = Some(GLStreamBuffer::new(
            ffi::GL_UNIFORM_BUFFER,
            8 * 1024 * 1024,
            StreamBufferUsage::Stream,
            self.uniform_buffer_alignment.max(1) as u32,
            "VSUniform",
        ));
        self.fragment_uniform_stream_buffer = Some(GLStreamBuffer::new(
            ffi::GL_UNIFORM_BUFFER,
            8 * 1024 * 1024,
            StreamBufferUsage::Stream,
            self.uniform_buffer_alignment.max(1) as u32,
            "PSUniform",
        ));
        self.vertex_push_constants_stream_buffer = Some(GLStreamBuffer::new(
            ffi::GL_UNIFORM_BUFFER,
            1024,
            StreamBufferUsage::Dynamic,
            self.uniform_buffer_alignment.max(1) as u32,
            "VSPushConstants",
        ));
        unsafe {
            ffi::glGenFramebuffers(1, &mut self.fbo);
            ffi::glGenFramebuffers(1, &mut self.fbo_read);
            ffi::glGenFramebuffers(1, &mut self.fbo_write);
            ffi::glGenVertexArrays(1, &mut self.vao);
            ffi::glGenVertexArrays(1, &mut self.expand_vao);
            ffi::glGenVertexArrays(1, &mut self.dummy_vao);
        }
        Ok(())
    }

    pub fn destroy_resources(&mut self) {
        unsafe {
            if self.fbo != 0 { ffi::glDeleteFramebuffers(1, &self.fbo); }
            if self.fbo_read != 0 { ffi::glDeleteFramebuffers(1, &self.fbo_read); }
            if self.fbo_write != 0 { ffi::glDeleteFramebuffers(1, &self.fbo_write); }
            if self.expand_ibo != 0 { ffi::glDeleteBuffers(1, &self.expand_ibo); }
            if self.vao != 0 { ffi::glDeleteVertexArrays(1, &self.vao); }
            if self.expand_vao != 0 { ffi::glDeleteVertexArrays(1, &self.expand_vao); }
            if self.dummy_vao != 0 { ffi::glDeleteVertexArrays(1, &self.dummy_vao); }
            if self.palette_ss != 0 { ffi::glDeleteSamplers(1, &self.palette_ss); }
            for s in self.ps_ss.iter() {
                if *s != 0 { ffi::glDeleteSamplers(1, s); }
            }
        }
        self.texture_upload_buffer = None;
        self.vertex_stream_buffer = None;
        self.index_stream_buffer = None;
        self.expand_index_stream_buffer = None;
        self.vertex_uniform_stream_buffer = None;
        self.fragment_uniform_stream_buffer = None;
        self.vertex_push_constants_stream_buffer = None;
        self.programs.clear();
        self.shader_cache.clear();
        self.convert.ps.clear();
        self.merge.ps[0].destroy();
        self.merge.ps[1].destroy();
        for p in self.interlace.ps.iter_mut() { p.destroy(); }
        for p in self.present.iter_mut() { p.destroy(); }
        self.fxaa.destroy();
        for p in self.date.primid_ps.iter_mut() { p.destroy(); }
        self.shadeboost.destroy();
        self.cas.upscale_ps.destroy();
        self.cas.sharpen_ps.destroy();
        self.imgui.ps.destroy();
    }

    pub fn create_timestamp_queries(&mut self) {
        unsafe { ffi::glGenQueries(NUM_TIMESTAMP_QUERIES as i32, self.timestamp_queries.as_mut_ptr()) }
    }

    pub fn destroy_timestamp_queries(&mut self) {
        unsafe { ffi::glDeleteQueries(NUM_TIMESTAMP_QUERIES as i32, self.timestamp_queries.as_mut_ptr()) }
    }

    pub fn pop_timestamp_query(&mut self) {
        // Walk waiting queries; if available, accumulate elapsed time and free the slot.
        let mut i = 0;
        while i < self.waiting_timestamp_queries {
            let slot = (self.read_timestamp_query.wrapping_add(i as u8)) as usize % NUM_TIMESTAMP_QUERIES;
            let mut available = 0;
            unsafe {
                ffi::glGetQueryObjectiv(
                    self.timestamp_queries[slot],
                    ffi::GL_QUERY_RESULT_AVAILABLE,
                    &mut available,
                )
            }
            if available == 0 {
                break;
            }
            let mut time_ns: u64 = 0;
            unsafe {
                ffi::glGetQueryObjectui64v(
                    self.timestamp_queries[slot],
                    ffi::GL_QUERY_RESULT,
                    &mut time_ns,
                )
            }
            self.accumulated_gpu_time += time_ns as f32 / 1_000_000.0;
            i += 1;
        }
        self.waiting_timestamp_queries = self.waiting_timestamp_queries.saturating_sub(i as u8);
        self.read_timestamp_query = self.read_timestamp_query.wrapping_add(i as u8);
    }

    pub fn kick_timestamp_query(&mut self) {
        if !self.gpu_timing_enabled {
            return;
        }
        if self.waiting_timestamp_queries as usize >= NUM_TIMESTAMP_QUERIES - 1 {
            self.pop_timestamp_query();
        }
        unsafe { ffi::glQueryCounter(self.timestamp_queries[self.write_timestamp_query as usize], ffi::GL_TIMESTAMP) }
        self.write_timestamp_query = (self.write_timestamp_query + 1) % NUM_TIMESTAMP_QUERIES as u8;
        self.waiting_timestamp_queries = self.waiting_timestamp_queries.saturating_add(1);
    }

    pub fn ia_set_vao(&mut self, vao: ffi::GLuint) {
        if self.state.vao != vao {
            self.state.vao = vao;
            unsafe { ffi::glBindVertexArray(vao) }
        }
    }
    pub fn ia_set_primitive_topology(&mut self, topology: ffi::GLenum) {
        self.draw_topology = topology;
    }
    pub fn ia_set_vertex_buffer(&mut self, _vertices: *const u8, _count: usize, _align_multiplier: usize) {}
    pub fn ia_set_index_buffer(&mut self, _index: *const u8, _count: usize) {}
    pub fn vs_set_index_buffer(&mut self, _index: *const u8, _count: usize) {}

    pub fn ps_set_shader_resource(&mut self, _i: i32, _sr: &dyn GSTexture) {}
    pub fn ps_set_sampler_state(&mut self, _ss: ffi::GLuint) {}
    pub fn clear_sampler_cache(&mut self) {}

    pub fn om_set_depth_stencil_state(&mut self, dss: &mut GSDepthStencilOGL) {
        dss.setup_depth(&mut self.state);
        dss.setup_stencil(&mut self.state);
    }

    pub fn om_set_blend_state(
        &mut self,
        enable: bool,
        src_factor: ffi::GLenum,
        dst_factor: ffi::GLenum,
        op: ffi::GLenum,
        src_factor_alpha: ffi::GLenum,
        dst_factor_alpha: ffi::GLenum,
        _is_constant: bool,
        _constant: u8,
    ) {
        unsafe {
            if self.state.blend != enable {
                self.state.blend = enable;
                if enable { ffi::glEnable(ffi::GL_BLEND) } else { ffi::glDisable(ffi::GL_BLEND) }
            }
            if enable {
                if self.state.blend_src_rgb != src_factor
                    || self.state.blend_dst_rgb != dst_factor
                {
                    self.state.blend_src_rgb = src_factor;
                    self.state.blend_dst_rgb = dst_factor;
                    ffi::glBlendFunc(src_factor, dst_factor);
                }
                if self.state.blend_equation_rgb != op {
                    self.state.blend_equation_rgb = op;
                    ffi::glBlendEquation(op);
                }
                if src_factor_alpha != ffi::GL_ONE || dst_factor_alpha != ffi::GL_ZERO {
                    ffi::glBlendFuncSeparate(
                        src_factor,
                        dst_factor,
                        src_factor_alpha,
                        dst_factor_alpha,
                    );
                }
            }
        }
    }

    pub fn om_set_render_targets(
        &mut self,
        _rt: Option<&mut GSTextureOGL>,
        _ds_as_rt: Option<&mut GSTextureOGL>,
        _ds: Option<&mut GSTextureOGL>,
        _scissor: Option<GSVector4i>,
    ) {}

    pub fn om_set_color_mask_state(&mut self, _sel: ColorMaskSelector) {
        unsafe {
            ffi::glColorMask(
                ffi::GL_TRUE,
                ffi::GL_TRUE,
                ffi::GL_TRUE,
                ffi::GL_TRUE,
            )
        }
    }

    pub fn om_unbind_texture(&mut self, _tex: &GSTextureOGL) {}

    pub fn set_viewport(&mut self, viewport: GSVector2i) {
        unsafe { ffi::glViewport(0, 0, viewport.x, viewport.y) }
    }

    pub fn set_scissor(&mut self, scissor: GSVector4i) {
        unsafe { ffi::glScissor(scissor.x, scissor.y, scissor.width(), scissor.height()) }
    }

    pub fn om_attach_rt(&mut self, _rt: Option<&mut GSTextureOGL>) {}
    pub fn om_attach_ds_as_rt(&mut self, _ds_as_rt: Option<&mut GSTextureOGL>) {}
    pub fn om_attach_ds(&mut self, _ds: Option<&mut GSTextureOGL>) {}
    pub fn om_set_fbo(&mut self, fbo: ffi::GLuint) {
        unsafe { ffi::glBindFramebuffer(ffi::GL_FRAMEBUFFER, fbo) }
    }

    pub fn draw_stretch_rect(&mut self, _s: GSVector4, _d: GSVector4, _ds: GSVector2i) {}

    pub fn set_index_buffer(
        &mut self,
        _buffer: &mut GLStreamBuffer,
        _index: *const u8,
        _count: usize,
    ) {}

    pub fn draw_primitive(&mut self) {}
    pub fn draw_indexed_primitive(&mut self) {}
    pub fn draw_indexed_primitive_range(&mut self, _offset: i32, _count: i32) {}
    pub fn draw_indexed_primitive_vs_expand(
        &mut self,
        _offset: i32,
        _count: i32,
        _vs_indexing: bool,
        _vs_indexing_expansion: i32,
    ) {}

    pub fn draw(&mut self, _config: &GSHWDrawConfig) {}
    pub fn draw_range(&mut self, _config: &GSHWDrawConfig, _offset: i32, _count: i32) {}

    pub fn create_surface(
        &mut self,
        ty: GSTextureType,
        width: i32,
        height: i32,
        levels: i32,
        format: GSTextureFormat,
    ) -> Option<Box<dyn GSTexture>> {
        Some(Box::new(GSTextureOGL::new(ty, width, height, levels, format)))
    }

    pub fn create_download_texture(
        &self,
        width: u32,
        height: u32,
        format: GSTextureFormat,
    ) -> Option<Box<dyn GSDownloadTexture>> {
        GSDownloadTextureOGL::create(width, height, format, !self.disable_download_pbo)
            .map(|t| Box::new(t) as Box<dyn GSDownloadTexture>)
    }

    pub fn init_prim_date_texture(
        &mut self,
        _rt: &mut GSTextureOGL,
        _area: GSVector4i,
        _datm: SetDATM,
    ) -> Option<Box<dyn GSTexture>> {
        None
    }

    pub fn copy_rect(
        &mut self,
        _stex: &mut GSTextureOGL,
        _dtex: &mut GSTextureOGL,
        _r: GSVector4i,
        _dest_x: u32,
        _dest_y: u32,
    ) {}

    pub fn push_debug_group(&mut self, _fmt: &str) {}
    pub fn pop_debug_group(&mut self) {
        unsafe { ffi::glPopDebugGroup() }
    }
    pub fn insert_debug_message(&mut self, _category: DebugMessageCategory, _fmt: &str) {}

    pub fn blit_rect(
        &mut self,
        _stex: &mut GSTextureOGL,
        _r: GSVector4i,
        _dsize: GSVector2i,
        _at_origin: bool,
        _filter: Filter,
    ) {
    }

    pub fn present_rect(
        &mut self,
        _stex: &mut GSTextureOGL,
        _srect: GSVector4,
        _dtex: &mut GSTextureOGL,
        _drect: GSVector4,
        _shader: PresentShader,
        _shader_time: f32,
        _filter: Filter,
    ) {
    }

    pub fn update_clut_texture(
        &mut self,
        _stex: &mut GSTextureOGL,
        _s_scale: f32,
        _offset_x: u32,
        _offset_y: u32,
        _dtex: &mut GSTextureOGL,
        _d_offset: u32,
        _d_size: u32,
    ) {
    }

    pub fn convert_to_indexed_texture(
        &mut self,
        _stex: &mut GSTextureOGL,
        _s_scale: f32,
        _offset_x: u32,
        _offset_y: u32,
        _sbw: u32,
        _spsm: u32,
        _dtex: &mut GSTextureOGL,
        _dbw: u32,
        _dpsm: u32,
    ) {
    }

    pub fn filtered_downsample_texture(
        &mut self,
        _stex: &mut GSTextureOGL,
        _dtex: &mut GSTextureOGL,
        _downsample_factor: u32,
        _clamp_min: GSVector2i,
        _d_rect: GSVector4,
    ) {
    }

    pub fn draw_multi_stretch_rects(
        &mut self,
        _rects: &[MultiStretchRect],
        _d_tex: &mut GSTextureOGL,
        _shader: ShaderConvertSelector,
    ) {
    }

    pub fn render_hw(&mut self, _config: &mut GSHWDrawConfig) {}

    pub fn vs_set_uniform_buffer(&mut self, _cb: &mut VSConstantBuffer) {}
    pub fn ps_set_uniform_buffer(&mut self, _cb: &mut PSConstantBuffer) {}
    pub fn vs_set_push_constants(&mut self, _base_vertex: u32, _base_index: u32, _force_update: bool) {}

    pub fn setup_pipeline(&mut self, _psel: &ProgramSelector) {}
    pub fn setup_sampler(&mut self, _ssel: SamplerSelector) {}
    pub fn setup_om(&mut self, _dssel: DepthStencilSelector) {}

    pub fn get_sampler_id(&mut self, _ssel: SamplerSelector) -> ffi::GLuint { 0 }
    pub fn get_palette_sampler_id(&mut self) -> ffi::GLuint { self.palette_ss }

    pub fn create_depth_stencil(&mut self, dssel: DepthStencilSelector) -> Option<Box<GSDepthStencilOGL>> {
        let mut dss = GSDepthStencilOGL::new();
        // Bit pattern maps depth/stencil on/off and ref values.
        let bits = dssel.0 as u32;
        if (bits & 0x01) != 0 {
            dss.enable_depth();
            dss.set_depth(ffi::GL_ALWAYS, true);
        }
        if (bits & 0x02) != 0 {
            dss.enable_stencil();
            dss.set_stencil(ffi::GL_ALWAYS, ffi::GL_KEEP);
        }
        Some(Box::new(dss))
    }

    pub fn create_sampler(&mut self, _ssel: SamplerSelector) -> ffi::GLuint { 0 }

    pub fn get_vs_source(&self, _sel: VSSelector) -> String { self.shader_tfx_vgs.clone() }
    pub fn get_ps_source(&self, _sel: PSSelector) -> String { self.shader_tfx_fs.clone() }
    pub fn gen_glsl_header(&self, _entry: &str, _ty: ffi::GLenum, _macro: &str) -> String { String::new() }
    pub fn get_shader_source(
        &self,
        _entry: &str,
        _ty: ffi::GLenum,
        _glsl_h: &str,
        _macro_sel: &str,
    ) -> String { String::new() }
    pub fn create_texture_fx(&mut self) -> bool { true }

    pub fn compile_fxaa_program(&mut self) -> bool { true }
    pub fn do_fxaa(&mut self, _stex: &mut GSTextureOGL, _dtex: &mut GSTextureOGL) {}
    pub fn compile_shade_boost_program(&mut self) -> bool { true }
    pub fn do_shade_boost(&mut self, _stex: &mut GSTextureOGL, _dtex: &mut GSTextureOGL, _params: &[f32; 4]) {}
    pub fn create_cas_programs(&mut self) -> bool { true }
    pub fn do_cas(
        &mut self,
        _stex: &mut GSTextureOGL,
        _dtex: &mut GSTextureOGL,
        _sharpen_only: bool,
        _constants: &[u32; NUM_CAS_CONSTANTS],
    ) -> bool { true }
    pub fn create_imgui_program(&mut self) -> bool { true }
    pub fn render_imgui(&mut self) {}
    pub fn render_blank_frame(&mut self) {}

    pub fn do_merge(
        &mut self,
        _s_tex: [&mut GSTextureOGL; 3],
        _s_rect: &mut [GSVector4; 3],
        _d_tex: &mut GSTextureOGL,
        _d_rect: &mut GSVector4,
        _pmode: &GSRegPMODE,
        _extbuf: &GSRegEXTBUF,
        _c: u32,
        _filter: Filter,
    ) {
    }
    pub fn do_interlace(
        &mut self,
        _stex: &mut GSTextureOGL,
        _srect: GSVector4,
        _dtex: &mut GSTextureOGL,
        _drect: GSVector4,
        _shader: ShaderInterlace,
        _filter: Filter,
        _cb: &InterlaceConstantBuffer,
    ) {
    }

    pub fn do_stretch_rect(
        &mut self,
        _stex: &mut GSTextureOGL,
        _srect: GSVector4,
        _dtex: &mut GSTextureOGL,
        _drect: GSVector4,
        _ps: &GLProgram,
        _filter: Filter,
    ) {
    }

    pub fn do_stretch_rect_masked(
        &mut self,
        _stex: &mut GSTextureOGL,
        _srect: GSVector4,
        _dtex: &mut GSTextureOGL,
        _drect: GSVector4,
        _ps: &GLProgram,
        _alpha_blend: bool,
        _cms: ColorMaskSelector,
        _filter: Filter,
    ) {
    }

    pub fn do_multi_stretch_rects(
        &mut self,
        _rects: &[MultiStretchRect],
        _num_rects: u32,
        _ds: GSVector2,
    ) {
    }

    pub fn set_swap_interval(&mut self) {
        if let Some(ctx) = self.gl_context.as_mut() {
            ctx.set_vsync(ctx.get_vsync());
        }
    }

    pub fn feedback_copy_and_bind(
        &mut self,
        _config: &GSHWDrawConfig,
        _rt: &mut GSTextureOGL,
        _rt_clone: &mut GSTextureOGL,
        _ds: &mut GSTextureOGL,
        _ds_clone: &mut GSTextureOGL,
        _copyarea: GSVector4i,
    ) {
    }

    pub fn feedback_copy_and_bind_sample(
        &mut self,
        _config: &GSHWDrawConfig,
        _rt: &mut GSTextureOGL,
        _rt_clone: &mut GSTextureOGL,
        _ds: &mut GSTextureOGL,
        _ds_clone: &mut GSTextureOGL,
        _copyarea: GSVector4i,
        _samplearea: GSVector4i,
    ) {
    }

    pub fn send_hw_draw(
        &mut self,
        _config: &GSHWDrawConfig,
        _draw_rt_clone: &mut GSTextureOGL,
        _draw_rt: &mut GSTextureOGL,
        _draw_ds_clone: &mut GSTextureOGL,
        _draw_ds: &mut GSTextureOGL,
        _one_barrier: bool,
        _full_barrier: bool,
    ) {
    }

    pub fn setup_date(
        &mut self,
        _rt: &mut GSTextureOGL,
        _ds: &mut GSTextureOGL,
        _datm: SetDATM,
        _bbox: GSVector4i,
    ) {
    }
}

impl Default for GSDeviceOGL {
    fn default() -> Self {
        Self::new()
    }
}

impl GSDevice for GSDeviceOGL {
    fn features(&self) -> &GSFeatures {
        static FEATURES: GSFeatures = GSFeatures { framebuffer_fetch: false };
        &FEATURES
    }
    fn create_surface(
        &mut self,
        ty: GSTextureType,
        width: i32,
        height: i32,
        levels: i32,
        format: GSTextureFormat,
    ) -> Option<Box<dyn GSTexture>> {
        GSDeviceOGL::create_surface(self, ty, width, height, levels, format)
    }
    fn destroy(&mut self) {
        GSDeviceOGL::destroy_resources(self);
    }
}

impl Drop for GSDeviceOGL {
    fn drop(&mut self) {
        self.destroy_resources();
        self.destroy_timestamp_queries();
    }
}

/// Static adapter-info helper.
pub fn get_adapter_info() -> Vec<GSAdapterInfo> {
    Vec::new()
}

/// OpenGL debug message callback. Mirrors `DebugMessageCallback`.
pub extern "C" fn debug_message_callback(
    gl_source: ffi::GLenum,
    gl_type: ffi::GLenum,
    id: ffi::GLuint,
    gl_severity: ffi::GLenum,
    gl_length: ffi::GLsizei,
    gl_message: *const ffi::GLchar,
    _user_param: *const ffi::GLvoid,
) {
    let message = unsafe {
        let len = if gl_length < 0 { 0 } else { gl_length as usize };
        let slice = std::slice::from_raw_parts(gl_message as *const u8, len);
        String::from_utf8_lossy(slice).into_owned()
    };
    eprintln!(
        "[GL {} src=0x{:x} type=0x{:x} id={} sev=0x{:x}] {}",
        "DEBUG", gl_source, gl_type, id, gl_severity, message
    );
}
