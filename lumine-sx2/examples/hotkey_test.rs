// Integration test / example for the Hotkey FFI parity with Qt's HotkeySettingsWidget.
//
// Verifies that we can enumerate PCSX2 hotkeys, read their current bindings,
// set a new binding, and clear it — all through the C API bridge.
//
// Run (debug, fast since lib is already built):
//   cargo run --example hotkey_test --features pcsx2-core
//
// NOTE: pcsx2_initialize() must succeed (needs the PCSX2 resources/fonts on disk).
// A real key capture (`capture_hotkey_*`) requires a live input event and is only
// exercised from the UI; here we validate the get/set/clear round-trip.

use lumine_sx2::pcsx2_capi::Pcsx2Api;

fn main() {
    println!("[TEST] Initializing PCSX2 core for hotkey test...");
    if !Pcsx2Api::initialize("E:\\project\\ps2\\bios") {
        eprintln!("[TEST] pcsx2_initialize() failed — cannot run hotkey FFI test.");
        return;
    }
    Pcsx2Api::set_bios_dir("E:\\project\\ps2\\bios");

    // 1) Enumerate hotkeys.
    let list = Pcsx2Api::get_hotkey_list();
    println!("[TEST] get_hotkey_list() returned {} hotkeys", list.len());
    if list.is_empty() {
        eprintln!("[TEST] FAIL: no hotkeys enumerated (InputManager hook or bindings not ready?)");
        return;
    }
    for (i, h) in list.iter().take(5).enumerate() {
        println!(
            "[TEST]   [{}] cat={} name={} display={} binding='{}'",
            i, h.category, h.name, h.display_name, h.binding
        );
    }

    // 2) Read a known binding (pick the first hotkey to keep it deterministic).
    let first = &list[0];
    let before = Pcsx2Api::get_hotkey_binding(&first.name);
    println!("[TEST] get_hotkey_binding('{}') = '{}'", first.name, before);

    // 3) Set a dummy binding, then read it back.
    let dummy = "Keyboard/KeyZ";
    Pcsx2Api::set_hotkey_binding(&first.name, dummy);
    let after_set = Pcsx2Api::get_hotkey_binding(&first.name);
    println!("[TEST] after set -> get_hotkey_binding('{}') = '{}'", first.name, after_set);
    if after_set == dummy {
        println!("[TEST] PASS: set/get hotkey binding round-trip works.");
    } else {
        eprintln!("[TEST] FAIL: set_hotkey_binding did not persist (got '{}').", after_set);
    }

    // 4) Clear it, confirm empty.
    Pcsx2Api::clear_hotkey_binding(&first.name);
    let after_clear = Pcsx2Api::get_hotkey_binding(&first.name);
    println!("[TEST] after clear -> get_hotkey_binding('{}') = '{}'", first.name, after_clear);
    if after_clear.is_empty() {
        println!("[TEST] PASS: clear_hotkey_binding works.");
    } else {
        eprintln!("[TEST] FAIL: clear_hotkey_binding did not empty (got '{}').", after_clear);
    }

    // 5) Capture API sanity (begin/cancel should not crash; poll returns None w/o input).
    Pcsx2Api::capture_hotkey_begin();
    let captured = Pcsx2Api::poll_hotkey_capture();
    println!("[TEST] poll_hotkey_capture() during idle = {:?} (expected None)", captured);
    Pcsx2Api::capture_hotkey_cancel();
    println!("[TEST] capture begin/poll/cancel executed without panic.");

    // Restore the original binding so we don't leave test state behind.
    if !before.is_empty() {
        Pcsx2Api::set_hotkey_binding(&first.name, &before);
    }

    println!("[TEST] Done.");
}
