//! End-to-end test of the `cpu_features` module backed by `raw-cpuid`.
//!
//! Builds on x86_64 (Windows in our test setup) and prints the
//! detected CPU vendor, family/model, and feature flags. The same
//! code works on Linux x86_64; on aarch64 the module is
//! `#[cfg(target_arch = "x86_64")]` gated, so this example compiles
//! to a no-op there.
//!
//! Run with: cargo run --release --example cpu_features_test

#[path = "../src/cpu_features.rs"]
mod cpu_features;
use cpu_features::{get_cpu_features, CpuFeatures};

fn main() {
    // The function is `Option<CpuFeatures>` so non-x86 platforms
    // gracefully report "no CPUID" rather than failing the build.
    let cf = match get_cpu_features() {
        Some(cf) => cf,
        None => {
            println!("skipped: not x86_64 (CPUID not available)");
            return;
        }
    };

    // Print a human-friendly summary. Format the flags as a compact
    // "yes/no" so the user can grep for what they need.
    println!("== CPUID snapshot ==");
    println!("  vendor           : {}", cf.vendor);
    println!("  brand            : {}", cf.brand);
    println!("  family           : {}", cf.family);
    println!("  model            : {}", cf.model);
    println!("  stepping         : {}", cf.stepping);
    println!("  max ext leaf     : {:#x}", cf.max_extended_leaf);
    println!();
    println!("  --- Feature flags ---");
    println!("  SSE / SSE2 / SSE3       : {} / {} / {}",
        yes_no(cf.has_sse), yes_no(cf.has_sse2), yes_no(cf.has_sse3));
    println!("  SSSE3 / SSE4.1 / SSE4.2 : {} / {} / {}",
        yes_no(cf.has_ssse3), yes_no(cf.has_sse4_1), yes_no(cf.has_sse4_2));
    println!("  AVX  / AVX2             : {} / {}", yes_no(cf.has_avx), yes_no(cf.has_avx2));
    println!("  AVX-512F                : {}", yes_no(cf.has_avx512_f));
    println!("  AES-NI / FMA            : {} / {}", yes_no(cf.has_aes_ni), yes_no(cf.has_fma));
    println!("  BMI1 / BMI2             : {} / {}", yes_no(cf.has_bmi1), yes_no(cf.has_bmi2));
    println!("  POPCNT / F16C / RDRAND  : {} / {} / {}",
        yes_no(cf.has_popcnt), yes_no(cf.has_f16c), yes_no(cf.has_rdrand));
    println!();
    println!("  summary: {}", cf.summary());

    // Sanity assertion: every x86_64 CPU since ~2010 supports SSE4.2.
    // If this fails the test is on a really weird machine.
    assert!(cf.has_sse4_2, "x86_64 host should always have SSE4.2");
    assert!(cf.has_avx, "modern x86_64 host should have AVX");

    println!("OK — cpu_features module works end-to-end (raw-cpuid backend)");
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}
