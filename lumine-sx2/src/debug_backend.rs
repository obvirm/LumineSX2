// Debugger backend - FFI to PCSX2 DebugInterface
// Wires Slint UI to PCSX2 debug tools

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::sync::{Arc, Mutex};

// ─── FFI to PCSX2 DebugInterface ───
extern "C" {
    // CPU info
    fn DebugInterface_isAlive(cpu_type: c_int) -> bool;
    fn DebugInterface_getPC(cpu_type: c_int) -> c_uint;
    fn DebugInterface_setPC(cpu_type: c_int, pc: c_uint);
    fn DebugInterface_getRegisterCount(cpu_type: c_int, category: c_int) -> c_int;
    fn DebugInterface_getRegisterName(cpu_type: c_int, category: c_int, index: c_int) -> *const c_char;
    fn DebugInterface_getRegister(cpu_type: c_int, category: c_int, index: c_int) -> c_uint;
    fn DebugInterface_setRegister(cpu_type: c_int, category: c_int, index: c_int, value: c_uint);
    fn DebugInterface_getRegister128(cpu_type: c_int, category: c_int, index: c_int, out: *mut [u8; 16]);
    fn DebugInterface_setRegister128(cpu_type: c_int, category: c_int, index: c_int, data: *const [u8; 16]);
    fn DebugInterface_getRegisterSize(cpu_type: c_int, category: c_int) -> c_int;
    fn DebugInterface_getRegisterCategoryCount(cpu_type: c_int) -> c_int;
    fn DebugInterface_getRegisterCategoryName(cpu_type: c_int, index: c_int) -> *const c_char;

    // Memory
    fn DebugInterface_Read8(cpu_type: c_int, addr: c_uint) -> u8;
    fn DebugInterface_Read16(cpu_type: c_int, addr: c_uint) -> u16;
    fn DebugInterface_Read32(cpu_type: c_int, addr: c_uint) -> u32;
    fn DebugInterface_Read64(cpu_type: c_int, addr: c_uint) -> u64;
    fn DebugInterface_Read128(cpu_type: c_int, addr: c_uint, out: *mut [u8; 16]);
    fn DebugInterface_Write8(cpu_type: c_int, addr: c_uint, val: u8);
    fn DebugInterface_Write16(cpu_type: c_int, addr: c_uint, val: u16);
    fn DebugInterface_Write32(cpu_type: c_int, addr: c_uint, val: u32);
    fn DebugInterface_Write64(cpu_type: c_int, addr: c_uint, val: u64);
    fn DebugInterface_Write128(cpu_type: c_int, addr: c_uint, data: *const [u8; 16]);

    // Breakpoints
    fn CBreakPoints_IsAddressBreakPoint(cpu_type: c_int, addr: c_uint, enabled: *mut bool) -> bool;
    fn CBreakPoints_AddBreakPoint(cpu_type: c_int, addr: c_uint, enabled: bool);
    fn CBreakPoints_RemoveBreakPoint(cpu_type: c_int, addr: c_uint);
    fn CBreakPoints_SwitchBreakPoint(cpu_type: c_int, addr: c_uint);
    fn CBreakPoints_GetBreakpointCount(cpu_type: c_int) -> c_uint;
    fn CBreakPoints_GetBreakpointInfo(cpu_type: c_int, index: c_int, out_addr: *mut c_uint, out_enabled: *mut bool, out_condition: *mut *const c_char);

    // Execution control
    fn DebugInterface_resumeCpu(cpu_type: c_int);
    fn DebugInterface_pauseCpu(cpu_type: c_int);
    fn DebugInterface_stepInto(cpu_type: c_int);
    fn DebugInterface_stepOver(cpu_type: c_int);
    fn DebugInterface_stepOut(cpu_type: c_int);

    // Stack
    fn MipsStackWalk_GetStack(cpu_type: c_int, out_frames: *mut *mut c_void, out_count: *mut c_int);
    fn MipsStackWalk_FreeFrames(frames: *mut c_void, count: c_int);

    // Symbols
    fn DebugInterface_GetSymbolCount(cpu_type: c_int) -> c_int;
    fn DebugInterface_GetSymbolInfo(cpu_type: c_int, index: c_int, out_name: *mut *const c_char, out_addr: *mut c_uint, out_type: *mut c_int);
    fn DebugInterface_FreeString(s: *const c_char);

    // Threads (IOP)
    fn DebugInterface_GetThreadCount(cpu_type: c_int) -> c_int;
    fn DebugInterface_GetThreadInfo(cpu_type: c_int, index: c_int, out_id: *mut c_int, out_pc: *mut c_uint, out_entry: *mut c_uint, out_priority: *mut c_int, out_state: *mut *const c_char, out_wait_type: *mut *const c_char, out_wait_id: *mut *const c_char);

    // Modules (IOP)
    fn DebugInterface_GetModuleCount(cpu_type: c_int) -> c_int;
    fn DebugInterface_GetModuleInfo(cpu_type: c_int, index: c_int, out_name: *mut *const c_char, out_version: *mut *const c_char, out_entry: *mut c_uint, out_gp: *mut c_uint, out_text: *mut c_uint, out_data: *mut c_uint, out_bss: *mut c_uint);
}

// ─── Constants ───
const CPU_EE: c_int = 0;
const CPU_IOP: c_int = 1;
const CPU_VU0: c_int = 2;
const CPU_VU1: c_int = 3;

// ─── Debug API ───
pub struct DebugApi;

impl DebugApi {
    // ─── CPU Info ───
    pub fn is_alive(cpu: i32) -> bool {
        unsafe { DebugInterface_isAlive(cpu as c_int) }
    }

    pub fn get_pc(cpu: i32) -> u32 {
        unsafe { DebugInterface_getPC(cpu as c_int) }
    }

    pub fn set_pc(cpu: i32, pc: u32) {
        unsafe { DebugInterface_setPC(cpu as c_int, pc); }
    }

    // ─── Registers ───
    pub fn get_register_category_count(cpu: i32) -> i32 {
        unsafe { DebugInterface_getRegisterCategoryCount(cpu as c_int) }
    }

    pub fn get_register_category_name(cpu: i32, index: i32) -> String {
        unsafe {
            let ptr = DebugInterface_getRegisterCategoryName(cpu as c_int, index as c_int);
            if ptr.is_null() { String::new() } else { CStr::from_ptr(ptr).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn get_register_count(cpu: i32, category: i32) -> i32 {
        unsafe { DebugInterface_getRegisterCount(cpu as c_int, category as c_int) }
    }

    pub fn get_register_name(cpu: i32, category: i32, index: i32) -> String {
        unsafe {
            let ptr = DebugInterface_getRegisterName(cpu as c_int, category as c_int, index as c_int);
            if ptr.is_null() { String::new() } else { CStr::from_ptr(ptr).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn get_register(cpu: i32, category: i32, index: i32) -> u32 {
        unsafe { DebugInterface_getRegister(cpu as c_int, category as c_int, index as c_int) }
    }

    pub fn set_register(cpu: i32, category: i32, index: i32, value: u32) {
        unsafe { DebugInterface_setRegister(cpu as c_int, category as c_int, index as c_int, value); }
    }

    pub fn get_register128(cpu: i32, category: i32, index: i32) -> [u8; 16] {
        let mut out = [0u8; 16];
        unsafe { DebugInterface_getRegister128(cpu as c_int, category as c_int, index as c_int, &mut out); }
        out
    }

    pub fn set_register128(cpu: i32, category: i32, index: i32, data: &[u8; 16]) {
        unsafe { DebugInterface_setRegister128(cpu as c_int, category as c_int, index as c_int, data); }
    }

    pub fn get_register_size(cpu: i32, category: i32) -> i32 {
        unsafe { DebugInterface_getRegisterSize(cpu as c_int, category as c_int) }
    }

    // ─── Memory ───
    pub fn read8(cpu: i32, addr: u32) -> u8 {
        unsafe { DebugInterface_Read8(cpu as c_int, addr) }
    }

    pub fn read16(cpu: i32, addr: u32) -> u16 {
        unsafe { DebugInterface_Read16(cpu as c_int, addr) }
    }

    pub fn read32(cpu: i32, addr: u32) -> u32 {
        unsafe { DebugInterface_Read32(cpu as c_int, addr) }
    }

    pub fn read64(cpu: i32, addr: u32) -> u64 {
        unsafe { DebugInterface_Read64(cpu as c_int, addr) }
    }

    pub fn read128(cpu: i32, addr: u32) -> [u8; 16] {
        let mut out = [0u8; 16];
        unsafe { DebugInterface_Read128(cpu as c_int, addr, &mut out); }
        out
    }

    pub fn write8(cpu: i32, addr: u32, val: u8) {
        unsafe { DebugInterface_Write8(cpu as c_int, addr, val); }
    }

    pub fn write16(cpu: i32, addr: u32, val: u16) {
        unsafe { DebugInterface_Write16(cpu as c_int, addr, val); }
    }

    pub fn write32(cpu: i32, addr: u32, val: u32) {
        unsafe { DebugInterface_Write32(cpu as c_int, addr, val); }
    }

    pub fn write64(cpu: i32, addr: u32, val: u64) {
        unsafe { DebugInterface_Write64(cpu as c_int, addr, val); }
    }

    pub fn write128(cpu: i32, addr: u32, data: &[u8; 16]) {
        unsafe { DebugInterface_Write128(cpu as c_int, addr, data); }
    }

    // ─── Breakpoints ───
    pub fn is_breakpoint(cpu: i32, addr: u32) -> bool {
        let mut enabled = false;
        unsafe { CBreakPoints_IsAddressBreakPoint(cpu as c_int, addr, &mut enabled) }
    }

    pub fn add_breakpoint(cpu: i32, addr: u32, enabled: bool) {
        unsafe { CBreakPoints_AddBreakPoint(cpu as c_int, addr, enabled); }
    }

    pub fn remove_breakpoint(cpu: i32, addr: u32) {
        unsafe { CBreakPoints_RemoveBreakPoint(cpu as c_int, addr); }
    }

    pub fn toggle_breakpoint(cpu: i32, addr: u32) {
        unsafe { CBreakPoints_SwitchBreakPoint(cpu as c_int, addr); }
    }

    // ─── Execution Control ───
    pub fn resume(cpu: i32) {
        unsafe { DebugInterface_resumeCpu(cpu as c_int); }
    }

    pub fn pause(cpu: i32) {
        unsafe { DebugInterface_pauseCpu(cpu as c_int); }
    }

    pub fn step_into(cpu: i32) {
        unsafe { DebugInterface_stepInto(cpu as c_int); }
    }

    pub fn step_over(cpu: i32) {
        unsafe { DebugInterface_stepOver(cpu as c_int); }
    }

    pub fn step_out(cpu: i32) {
        unsafe { DebugInterface_stepOut(cpu as c_int); }
    }

    // ─── Hex formatting ───
    pub fn format_hex32(val: u32) -> String {
        format!("0x{:08X}", val)
    }

    pub fn format_hex64(val: u64) -> String {
        format!("0x{:016X}", val)
    }

    pub fn format_float(val: u32) -> String {
        format!("{}", f32::from_bits(val))
    }

    pub fn format_double(val: u64) -> String {
        format!("{}", f64::from_bits(val))
    }

    pub fn format_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
    }

    pub fn format_ascii(bytes: &[u8]) -> String {
        bytes.iter().map(|&b| if b >= 0x20 && b < 0x7F { b as char } else { '.' }).collect()
    }
}
