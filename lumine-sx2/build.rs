fn main() {
    slint_build::compile("ui/main.slint").expect("Slint build failed");
    
    #[cfg(feature = "pcsx2-core")]
    {
        let build = "E:/project/pcsx2/build";
        let vcpkg = "C:/Users/X/vcpkg/installed/x64-windows/lib";
        
        // PCSX2 core + common + capi (linked separately from cmake build)
        println!("cargo:rustc-link-search=native={}/pcsx2", build);
        println!("cargo:rustc-link-search=native={}/common", build);
        println!("cargo:rustc-link-search=native={}/capi", build);
        println!("cargo:rustc-link-lib=static=pcsx2_capi");
        println!("cargo:rustc-link-lib=static=PCSX2");
        println!("cargo:rustc-link-lib=static=common");
        
        // Rust CDVD static lib (pure Rust CHD/CSO/ISO reader)
        println!("cargo:rustc-link-search=native=E:/project/pcsx2/pcsx2/rust/cdvd/target/release");
        println!("cargo:rustc-link-lib=static=pcsx2_cdvd");
        
        // GS ISA variants (required)
        println!("cargo:rustc-link-search=native={}/pcsx2", build);
        for gs in &["GS-avx2", "GS-avx", "GS-sse4"] {
            println!("cargo:rustc-link-lib=static={}", gs);
        }
        
        // GS ISA variants
        
        // 3rd-party
        let third = format!("{}/3rdparty", build);
        for (dir, lib) in &[
            ("fmt", "fmt"),
            ("libchdr", "libchdr"),
            ("lzma", "pcsx2-lzma"),
            ("soundtouch", "pcsx2-soundtouch"),
            ("zydis", "zydis"),
            ("glad", "glad"),
            ("imgui", "imgui"),
            ("D3D12MemAlloc", "D3D12MemAlloc"),
            ("cpuinfo", "cpuinfo"),
            ("rcheevos", "rcheevos"),
            ("rainterface", "rainterface"),
            ("freesurround", "freesurround"),
            ("discord-rpc", "discord-rpc"),
            ("simpleini", "simpleini"),
            ("libzip", "zip"),
            ("cubeb", "cubeb"),
            ("ccc", "ccc"),
            ("demangler", "demanglegnu"),
        ] {
            println!("cargo:rustc-link-search=native={}/{}", third, dir);
            println!("cargo:rustc-link-lib=static={}", lib);
        }
        
        // vcpkg
        println!("cargo:rustc-link-search=native={}", vcpkg);
        for lib in &[
            "freetype", "jpeg", "libpng16", "z",
            "SDL3", "c4core", "fmt", "bz2",
            "brotlidec", "brotlienc", "brotlicommon",
            "DirectX-Headers", "DirectX-Guids",
            "ryml", "lz4", "zstd",
            "plutosvg", "plutovg",
            "libwebp", "libwebpdemux", "libwebpmux", "libsharpyuv",
        ] {
            println!("cargo:rustc-link-lib=static={}", lib);
        }
        
        // Windows
        for lib in &[
            "ole32", "shell32", "advapi32", "ntdll", "ws2_32", "userenv",
            "bcrypt", "gdi32", "d3d12", "dxgi", "d3dcompiler", "opengl32",
            "winmm", "imm32", "version", "setupapi", "hid", "xinput",
            "dinput8", "iphlpapi", "uuid", "ksuser", "mfplat", "mfuuid",
        ] {
            println!("cargo:rustc-link-lib={}", lib);
        }
        
        println!("cargo:rerun-if-changed={}/capi/pcsx2_capi.lib", build);
        
        // Increase default stack size. /FORCE:MULTIPLE is required here: the
        // `Host::` interface is provided by BOTH pcsx2_capi.lib (our headless/
        // Slint host implementation) and the GUI `Host.cpp` object inside
        // PCSX2.lib. We want the capi version; /FORCE:MULTIPLE lets the linker
        // pick it. We pair this with the LLVM lld-link (see .cargo/config.toml)
        // because MSVC link.exe crashes with STATUS_STACK_BUFFER_OVERRUN
        // (0xc0000409) on this very large link line.
        println!("cargo:rustc-link-arg=/STACK:8388608");
        println!("cargo:rustc-link-arg=/FORCE:MULTIPLE");
    }
}

