# - Try to find SHADERC
# Once done this will define
#  SHADERC_FOUND - System has SHADERC
#  SHADERC_INCLUDE_DIRS - The SHADERC include directories
#  SHADERC_LIBRARIES - The libraries needed to use SHADERC

find_path(
    SHADERC_INCLUDE_DIR shaderc/shaderc.h
    ${SHADERC_PATH_INCLUDES}
)

find_library(
    SHADERC_LIBRARY
    NAMES shaderc_shared.1 shaderc_shared
    PATHS ${ADDITIONAL_LIBRARY_PATHS} ${SHADERC_PATH_LIB}
)

# Stub fallback: if shaderc isn't installed (spirv-tools build fails on
# some platforms), fake a stub library so PCSX2 can configure. Vulkan
# shader compilation will fail at runtime, but the build proceeds.
if(NOT SHADERC_LIBRARY)
    message(STATUS "Shaderc not found — creating stub for build only")
    set(SHADERC_STUB_DIR "${CMAKE_BINARY_DIR}/_shaderc_stub")
    file(MAKE_DIRECTORY "${SHADERC_STUB_DIR}/include/shaderc")
    # Write a complete shaderc.h stub with all enums and types used by
    # PCSX2's Vulkan renderer. The dynamic loader will fail at runtime
    # when it tries to load shaderc_shared, but the build proceeds.
    file(WRITE "${SHADERC_STUB_DIR}/include/shaderc/shaderc.h" [=[/*
 * shaderc stub header — provides types/enums needed for compilation.
 * Real shaderc is loaded dynamically at runtime via dyn_shaderc.
 */
#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct shaderc_compiler_struct* shaderc_compiler_t;
typedef struct shaderc_compile_options_struct* shaderc_compile_options_t;
typedef struct shaderc_compilation_result_struct* shaderc_compilation_result_t;

typedef enum {
    shaderc_source_language_glsl,
    shaderc_source_language_hlsl
} shaderc_source_language;

typedef enum {
    shaderc_compilation_status_success = 0,
    shaderc_compilation_status_invalid_stage = 1,
    shaderc_compilation_status_compilation_error = 2,
    shaderc_compilation_status_internal_error = 3,
    shaderc_compilation_status_null_result_object = 4,
    shaderc_compilation_status_invalid_assembly = 5,
    shaderc_compilation_status_validation_error = 6,
    shaderc_compilation_status_transformation_error = 7,
    shaderc_compilation_status_configuration_error = 8
} shaderc_compilation_status;

typedef enum {
    shaderc_target_env_vulkan,
    shaderc_target_env_opengl,
    shaderc_target_env_opengl_compat,
    shaderc_target_env_webgpu
} shaderc_target_env;

typedef enum {
    shaderc_env_version_vulkan_1_0 = 1 << 16,
    shaderc_env_version_vulkan_1_1 = (1 << 16) | (1 << 8),
    shaderc_env_version_vulkan_1_2 = (1 << 16) | (2 << 8),
    shaderc_env_version_vulkan_1_3 = (1 << 16) | (3 << 8)
} shaderc_env_version;

typedef enum {
    shaderc_glsl_vertex_shader,
    shaderc_glsl_fragment_shader,
    shaderc_glsl_compute_shader,
    shaderc_glsl_geometry_shader,
    shaderc_glsl_tess_control_shader,
    shaderc_glsl_tess_evaluation_shader
} shaderc_shader_kind;

typedef enum {
    shaderc_optimization_level_zero,
    shaderc_optimization_level_size,
    shaderc_optimization_level_performance
} shaderc_optimization_level;

shaderc_compiler_t shaderc_compiler_initialize(void);
void shaderc_compiler_release(shaderc_compiler_t);
shaderc_compile_options_t shaderc_compile_options_initialize(void);
void shaderc_compile_options_release(shaderc_compile_options_t);
void shaderc_compile_options_set_source_language(shaderc_compile_options_t, shaderc_source_language);
void shaderc_compile_options_set_generate_debug_info(shaderc_compile_options_t);
void shaderc_compile_options_set_optimization_level(shaderc_compile_options_t, shaderc_optimization_level);
void shaderc_compile_options_set_target_env(shaderc_compile_options_t, shaderc_target_env, uint32_t);
shaderc_compilation_result_t shaderc_compile_into_spv(
    shaderc_compiler_t, const char* source_text, size_t source_text_size,
    shaderc_shader_kind kind, const char* input_file_name, const char* entry_point_name,
    shaderc_compile_options_t options);
void shaderc_result_release(shaderc_compilation_result_t);
size_t shaderc_result_get_length(shaderc_compilation_result_t);
size_t shaderc_result_get_num_warnings(shaderc_compilation_result_t);
const char* shaderc_result_get_bytes(shaderc_compilation_result_t);
const char* shaderc_result_get_error_message(shaderc_compilation_result_t);
shaderc_compilation_status shaderc_result_get_compilation_status(shaderc_compilation_result_t);

#ifdef __cplusplus
}
#endif
]=])
    set(SHADERC_INCLUDE_DIR "${SHADERC_STUB_DIR}/include")
    # PCSX2's VKShaderCache uses dyn_shaderc (function pointers loaded at
    # runtime), so the link step does not need a real shaderc.lib. We use
    # an INTERFACE IMPORTED target below, which requires no library file.
endif()

include(FindPackageHandleStandardArgs)
find_package_handle_standard_args(Shaderc DEFAULT_MSG
                                  SHADERC_INCLUDE_DIR)

if(SHADERC_FOUND)
    add_library(Shaderc::shaderc_shared INTERFACE IMPORTED)
    set_target_properties(Shaderc::shaderc_shared PROPERTIES
        INTERFACE_INCLUDE_DIRECTORIES "${SHADERC_INCLUDE_DIR}"
        INTERFACE_COMPILE_DEFINITIONS "SHADERC_SHAREDLIB"
    )
endif()

mark_as_advanced(SHADERC_INCLUDE_DIR SHADERC_LIBRARY)
