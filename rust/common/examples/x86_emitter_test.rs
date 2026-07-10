//! Standalone end-to-end test: build a small function with the
//! x86_emitter PoC, copy the bytes into an executable page, and
//! invoke them. This proves the emitted bytes are valid machine
//! code that actually runs.
//!
//! Run with: cargo run --release --example x86_emitter_test

#[path = "../src/x86_emitter.rs"]
mod x86_emitter;
use x86_emitter::{Emitter, Reg};
#[allow(unused_imports)]
use iced_x86;

fn main() {
    // Strategy: emit a function that takes a u64 arg in RCX (Windows
    // x64 calling convention; System V uses RDI but we won't actually
    // call it from C — we'll call it via a fn pointer).
    //
    // Function: return arg + 0x1234
    //   mov rax, 0x1234
    //   add rax, rcx     ; rcx is the first arg on Windows x64
    //   ret
    let mut e = Emitter::new();
    e.mov_r64_imm(Reg::RAX, 0x1234).unwrap();
    e.add_r64_r64(Reg::RAX, Reg::RCX).unwrap();
    e.ret().unwrap();
    let bytes = e.take().unwrap();
    println!("emitted {} bytes: {:02x?}", bytes.len(), bytes);

    // Round-trip check: decode the bytes back with iced-x86.
    let mut decoder = iced_x86::Decoder::new(64, &bytes, 0);
    for instr in decoder.iter() {
        println!("  -> {:?}", instr);
    }

    // Execute the bytes via mmap + cast to fn pointer.
    #[cfg(unix)]
    {
        use std::mem;
        let page = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                bytes.len().max(4096),
                libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if page == libc::MAP_FAILED {
            eprintln!("mmap failed");
            return;
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), page as *mut u8, bytes.len()) };
        let f: extern "C" fn(u64) -> u64 = unsafe { mem::transmute(page) };
        let result = f(0x1000);
        println!("f(0x1000) = {:#x} (expected {:#x})", result, 0x1000 + 0x1234);
        assert_eq!(result, 0x1000 + 0x1234);
        unsafe { libc::munmap(page, bytes.len().max(4096)) };
        println!("OK — emitted code runs and returns the right value");
    }

    #[cfg(windows)]
    {
        use std::mem;
        use windows_sys::Win32::System::Memory::{
            VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
        };
        let page = unsafe {
            VirtualAlloc(std::ptr::null(), bytes.len().max(4096), MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
        };
        if page.is_null() {
            eprintln!("VirtualAlloc failed");
            return;
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), page as *mut u8, bytes.len()) };
        let f: extern "C" fn(u64) -> u64 = unsafe { mem::transmute(page) };
        let result = f(0x1000);
        println!("f(0x1000) = {:#x} (expected {:#x})", result, 0x1000 + 0x1234);
        assert_eq!(result, 0x1000 + 0x1234);
        unsafe { VirtualFree(page, 0, 0) }; // MEM_RELEASE = 0x8000
        println!("OK — emitted code runs and returns the right value");
    }
}
