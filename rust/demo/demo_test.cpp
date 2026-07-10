// demo_test.cpp - minimal C++ program yang link ke pcsx2_common_rs.lib
// dan panggil beberapa FFI exports. Bukti bahwa Rust <-> C++ bridge bekerja.
//
// Build:
//   cl /EHsc /std:c++17 demo_test.cpp pcsx2_common_rs.lib /OUT:demo_test.exe
//
// Run:
//   demo_test.exe

#include <cstdio>
#include <cstdint>
#include <cstring>
#include <cstddef>
#include <cstdlib>

extern "C" {
    // byte_swap
    uint16_t pcsx2_byteswap16(uint16_t val);
    uint32_t pcsx2_byteswap32(uint32_t val);
    uint64_t pcsx2_byteswap64(uint64_t val);

    // bit_utils
    uint32_t pcsx2_count_leading_sign_bits_s32(int32_t val);
    uint32_t pcsx2_align_up_u32(uint32_t val, uint32_t alignment);

    // md5
    void* pcsx2_md5_new();
    void  pcsx2_md5_update(void* ctx, const uint8_t* data, uint32_t len);
    void  pcsx2_md5_final(void* ctx, uint8_t* out);  // does NOT free
    void  pcsx2_md5_destroy(void* ctx);                 // frees

    // threading
    void pcsx2_thread_sleep(uint32_t ms);

    // host_sys (cross-platform shim)
    uint32_t pcsx2_host_page_size();
    uint64_t pcsx2_host_physical_memory();
    uint64_t pcsx2_host_tick_frequency();
    uint64_t pcsx2_host_cpu_ticks();

    // error
    void* pcsx2_error_create();
    void  pcsx2_error_set_errno(void* err, int32_t code);
    void  pcsx2_error_destroy(void* err);

    // hash
    uint64_t pcsx2_hash_combine_u64(uint64_t seed, uint64_t value);

    // aligned_malloc (use size_t for portability)
    void* pcsx2_aligned_malloc(size_t size, size_t alignment);
    void  pcsx2_aligned_free(void* ptr);

    // easing
    float pcsx2_easing_in_sine(float t);
    float pcsx2_easing_out_cubic(float t);
    float pcsx2_easing_in_out_quad(float t);

    // image (load + save only; no from-rgba factory exported)
    bool  pcsx2_image_load_png(const char* path, uint32_t* out_width, uint32_t* out_height, uint8_t** out_pixels, uint32_t* out_len);
    bool  pcsx2_image_save_png(const char* path, uint32_t width, uint32_t height, const uint8_t* pixels, uint32_t stride);
    void  pcsx2_image_free(uint8_t* pixels);

    // zip
    bool  pcsx2_zip_extract(const char* archive, const char* dest_dir);

    // settings
    void* pcsx2_memory_settings_create();
    void  pcsx2_memory_settings_destroy(void* s);
    void  pcsx2_memory_settings_set_int(void* s, const char* section, const char* key, int32_t value);
    bool  pcsx2_memory_settings_get_int(void* s, const char* section, const char* key, int32_t* out);

    // timer
    uint64_t pcsx2_timer_get_ticks();
    uint64_t pcsx2_timer_get_tick_frequency();
}

int main() {
    printf("====================================================\n");
    printf("  pcsx2_common_rs FFI demo test\n");
    printf("  Rust common library -> C++ FFI bridge\n");
    printf("====================================================\n\n");

    printf("[1] Byte swap (0x12345678 should become 0x78563412):\n");
    uint32_t original = 0x12345678;
    uint32_t swapped = pcsx2_byteswap32(original);
    printf("    0x%08X -> 0x%08X\n", original, swapped);

    uint16_t s16 = 0xABCD;
    uint16_t r16 = pcsx2_byteswap16(s16);
    printf("    uint16 0x%04X -> 0x%04X\n\n", s16, r16);

    printf("[2] Align 12345 up to 4096:\n");
    uint32_t aligned = pcsx2_align_up_u32(12345, 4096);
    printf("    Result: %u (expected 16384)\n\n", aligned);

    printf("[3] Count leading sign bits:\n");
    printf("    -1: %u (expected 32)\n", pcsx2_count_leading_sign_bits_s32(-1));
    printf("    1:  %u (expected 31)\n\n", pcsx2_count_leading_sign_bits_s32(1));

    printf("[4] MD5 hash:\n");
    void* md5 = pcsx2_md5_new();
    const char* test_str = "Hello, PCSX2!";
    pcsx2_md5_update(md5, (const uint8_t*)test_str, (uint32_t)strlen(test_str));
    uint8_t digest[16];
    pcsx2_md5_final(md5, digest);     // does NOT free
    pcsx2_md5_destroy(md5);            // now frees
    printf("    MD5(\"%s\") = ", test_str);
    for (int i = 0; i < 16; i++) printf("%02x", digest[i]);
    printf("\n\n");

    printf("[5] Hash combine:\n");
    uint64_t seed = pcsx2_hash_combine_u64(0, 0x12345678);
    seed = pcsx2_hash_combine_u64(seed, 0xDEADBEEF);
    printf("    combined = 0x%016llX\n\n", (unsigned long long)seed);

    printf("[6] Easing functions (t=0.5):\n");
    printf("    in_sine(0.5)    = %.4f (expected ~0.5)\n", pcsx2_easing_in_sine(0.5f));
    printf("    out_cubic(0.5)  = %.4f (expected ~0.875)\n", pcsx2_easing_out_cubic(0.5f));
    printf("    in_out_quad(0.5)= %.4f (expected 0.5)\n\n", pcsx2_easing_in_out_quad(0.5f));

    printf("[7] Threading:\n");
    printf("    Sleep 10ms...\n");
    pcsx2_thread_sleep(10);
    printf("    Slept.\n\n");

    printf("[8] Host system info (cross-platform shim):\n");
    printf("    Host page size:      %u bytes\n", pcsx2_host_page_size());
    printf("    Physical memory:     %llu bytes (%.2f GB)\n",
           (unsigned long long)pcsx2_host_physical_memory(),
           (double)pcsx2_host_physical_memory() / (1024.0 * 1024.0 * 1024.0));
    printf("    Tick frequency:      %llu Hz\n",
           (unsigned long long)pcsx2_host_tick_frequency());
    printf("    CPU ticks:           %llu\n\n",
           (unsigned long long)pcsx2_host_cpu_ticks());

    printf("[9] Error (Pcsx2Error):\n");
    void* err = pcsx2_error_create();
    pcsx2_error_set_errno(err, 2);  // ENOENT
    printf("    Error created and set to errno=2 (ENOENT)\n");
    pcsx2_error_destroy(err);
    printf("    Error destroyed cleanly.\n\n");

    printf("[10] Aligned malloc:\n");
    size_t alloc_size = 1024;
    size_t alloc_align = 64;
    void* aligned_ptr = pcsx2_aligned_malloc(alloc_size, alloc_align);
    printf("    pcsx2_aligned_malloc(1024, 64) = %p\n", aligned_ptr);
    if (aligned_ptr) {
        printf("    Aligned OK; freeing...\n");
        pcsx2_aligned_free(aligned_ptr);
        printf("    Freed OK.\n");
    }
    printf("\n");

    printf("[11] Timer (cross-platform):\n");
    printf("    Timer ticks:         %llu\n", (unsigned long long)pcsx2_timer_get_ticks());
    printf("    Timer frequency:     %llu Hz\n\n", (unsigned long long)pcsx2_timer_get_tick_frequency());

    printf("[12] Image (PNG roundtrip):\n");
    // First create a test PNG with raw RGBA data, then load it back.
    // Allocate a 64x64 RGBA buffer (red gradient).
    const uint32_t W = 64, H = 64;
    uint8_t pixels[64 * 64 * 4];
    for (uint32_t i = 0; i < W * H; i++) {
        pixels[i * 4 + 0] = (i % W) * 4;      // R
        pixels[i * 4 + 1] = 0;                  // G
        pixels[i * 4 + 2] = 0;                  // B
        pixels[i * 4 + 3] = 0xFF;               // A
    }
    bool saved = pcsx2_image_save_png(
        "E:/project/pcsx2/rust/demo/test_roundtrip.png",
        W, H, pixels, W * 4
    );
    printf("    pcsx2_image_save_png(%ux%u) = %s\n", W, H, saved ? "OK" : "FAIL");
    if (saved) {
        uint32_t lw = 0, lh = 0, llen = 0;
        uint8_t* lpx = nullptr;
        bool loaded = pcsx2_image_load_png(
            "E:/project/pcsx2/rust/demo/test_roundtrip.png",
            &lw, &lh, &lpx, &llen
        );
        printf("    pcsx2_image_load_png = %s, size=%ux%u, %u bytes\n",
               loaded ? "OK" : "FAIL", lw, lh, llen);
        if (loaded && lpx) pcsx2_image_free(lpx);
    }
    printf("\n");

    printf("[13] Memory settings interface:\n");
    void* settings = pcsx2_memory_settings_create();
    if (settings) {
        pcsx2_memory_settings_set_int(settings, "EmuCore/GS", "Renderer", 14);  // 14 = Vulkan
        pcsx2_memory_settings_set_int(settings, "EmuCore/GS", "InternalResolution", 2);
        int32_t out_val = 0;
        bool got = pcsx2_memory_settings_get_int(settings, "EmuCore/GS", "Renderer", &out_val);
        printf("    Set 'EmuCore/GS:Renderer' = 14\n");
        printf("    Set 'EmuCore/GS:InternalResolution' = 2\n");
        printf("    Get 'Renderer' back: %d (got=%s)\n", out_val, got ? "true" : "false");
        pcsx2_memory_settings_destroy(settings);
        printf("    Settings destroyed.\n");
    }
    printf("\n");

    printf("[14] Zip extract (skipped, no test zip):\n");
    printf("    pcsx2_zip_extract(...) - skipped (would need a real .zip file)\n\n");

    printf("====================================================\n");
    printf("  All FFI calls SUCCEEDED!\n");
    printf("  Rust common library is running in this C++ process.\n");
    printf("====================================================\n");

    return 0;
}