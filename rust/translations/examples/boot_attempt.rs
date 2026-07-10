//! examples/boot_attempt.rs
//!
//! An experimental "boot PCSX2 from these translations" example.
//! This will likely fail with `unimplemented!()` panic from many call sites,
//! but it lets us audit what janggal (anomalies) prevent this from running
//! like the original PCSX2.

use pcsx2_translations::pcsx2::VmManager;
use pcsx2_translations::pcsx2::FinalCore;
use pcsx2_translations::pcsx2::IopBios;
use pcsx2_translations::pcsx2::FinalPeripherals;
use pcsx2_translations::pcsx2::FinalInputPadEtc;
use pcsx2_translations::common::FinalCommon;
use pcsx2_translations::thirdparty::imgui::Imgui;
use pcsx2_translations::thirdparty::vulkan::VulkanCore;
use pcsx2_translations::thirdparty::rcheevos::Rcheevos;

fn main() {
    println!("PCSX2 Rust Translations - boot attempt");
    println!("=========================================");
    println!();
    println!("This will likely panic with `unimplemented!()` from various call sites.");
    println!("The goal is to enumerate what janggal prevents running like original PCSX2.");
    println!();

    // Stage 1: Init
    println!("[1/8] Init core VM state...");
    let mut vm = match VmManager::initialize() {
        Ok(v) => { println!("    OK"); v }
        Err(e) => { println!("    FAIL: {e}"); return; }
    };

    // Stage 2: Init subsystems (this is where most unimplemented! panics will fire)
    println!("[2/8] Init CDVD...");
    // cdvd::cdvdInit() will likely panic because it tries to access HW regs
    // which depend on having actual PCSX2 hardware state initialized.
    let _cdvd_ok = std::panic::catch_unwind(|| {
        FinalCore::init_subsystems();
    });
    println!("    {:?}", _cdvd_ok);

    // Stage 3: Init BIOS
    println!("[3/8] Init BIOS...");
    let _bios_ok = std::panic::catch_unwind(|| {
        IopBios::iopBiosInit();
    });
    println!("    {:?}", _bios_ok);

    // Stage 4: Try to boot ELF
    println!("[4/8] Boot ELF...");
    let _boot_ok = std::panic::catch_unwind(|| {
        let _ = VmManager::boot(&mut vm);
    });
    println!("    {:?}", _boot_ok);

    // Stage 5: ImGui
    println!("[5/8] ImGui context...");
    let _imgui_ok = std::panic::catch_unwind(|| {
        Imgui::create_context();
    });
    println!("    {:?}", _imgui_ok);

    // Stage 6: Vulkan
    println!("[6/8] Vulkan...");
    let _vulkan_ok = std::panic::catch_unwind(|| {
        // Just check the type system, don't call FFI
        let _: Option<VulkanCore::VkInstance> = None;
    });
    println!("    {:?}", _vulkan_ok);

    // Stage 7: Achievements
    println!("[7/8] Achievements...");
    let _ach_ok = std::panic::catch_unwind(|| {
        let _ = Rcheevos::rc_client_create(12);
    });
    println!("    {:?}", _ach_ok);

    // Stage 8: Run frame
    println!("[8/8] Run frame...");
    let _frame_ok = std::panic::catch_unwind(|| {
        // VmManager::run_frame will call into EE/IOP, which will panic.
        let _ = vm.run_one_frame();
    });
    println!("    {:?}", _frame_ok);

    println!();
    println!("Audit complete. Look at `_audit_runtime.log` for details.");
}
