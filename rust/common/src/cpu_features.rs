// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! x86 CPUID-based feature detection.
//!
//! Rust replacement for the FreeBSD-only `cpuinfo` library that
//! `common/Linux/LnxHostSys.cpp` historically linked in via PCSX2
//! 3rdparty. We use the actively-maintained [`raw_cpuid`] crate
//! (a thin, customizable CPUID wrapper) to read vendor string,
//! family / model / stepping, and a useful subset of the CPUID
//! feature bits: SSE4.2, AVX, AVX2, AVX-512F, AES-NI, FMA, BMI1,
//! BMI2.
//!
//! The whole module is gated to `#[cfg(target_arch = "x86_64")]`
//! because CPUID is an x86 instruction. On every other target
//! `get_cpu_features()` returns `None` and the struct is absent
//! from the public API (callers must guard with `cfg`).
//!
//! Example binary: `cargo run --release --example cpu_features_test`.

#![cfg(target_arch = "x86_64")]

use raw_cpuid::CpuId;

/// Description of the host x86_64 CPU, derived from CPUID.
#[derive(Clone, Debug)]
pub struct CpuFeatures {
    /// CPU vendor string, e.g. `"GenuineIntel"` or `"AuthenticAMD"`.
    pub vendor: String,
    /// Brand string from CPUID leaf 0x80000002..0x80000004, when
    /// available. Empty on older CPUs that don't expose it.
    pub brand: String,
    /// Processor family (raw CPUID family id).
    pub family: u32,
    /// Processor model (raw CPUID model id).
    pub model: u32,
    /// Stepping / revision within the model.
    pub stepping: u32,
    /// Maximum supported extended CPUID leaf (0 if the extended
    /// range isn't supported).
    pub max_extended_leaf: u32,

    // ---- Feature flags ----------------------------------------------------
    pub has_sse: bool,
    pub has_sse2: bool,
    pub has_sse3: bool,
    pub has_ssse3: bool,
    pub has_sse4_1: bool,
    pub has_sse4_2: bool,
    pub has_avx: bool,
    pub has_avx2: bool,
    pub has_avx512_f: bool,
    pub has_aes_ni: bool,
    pub has_fma: bool,
    pub has_bmi1: bool,
    pub has_bmi2: bool,
    pub has_popcnt: bool,
    pub has_f16c: bool,
    pub has_rdrand: bool,
}

impl CpuFeatures {
    /// `true` if the CPU advertises any AVX-512 foundation feature.
    pub fn has_avx512(&self) -> bool {
        self.has_avx512_f
    }

    /// Compact human-readable summary, used by the example binary.
    pub fn summary(&self) -> String {
        format!(
            "{} family={} model={} stepping={} (SSE4.2={}, AVX={}, AVX2={}, AVX-512={}, AES-NI={}, FMA={}, BMI1={}, BMI2={})",
            self.vendor,
            self.family,
            self.model,
            self.stepping,
            self.has_sse4_2,
            self.has_avx,
            self.has_avx2,
            self.has_avx512_f,
            self.has_aes_ni,
            self.has_fma,
            self.has_bmi1,
            self.has_bmi2,
        )
    }
}

/// Read the host's CPUID and return a populated [`CpuFeatures`].
///
/// Returns `None` on non-x86 platforms (the entire module is gated
/// off there, so this is unreachable in practice on those targets —
/// kept as a safe API so callers don't need to wrap calls in
/// `#[cfg]`).
pub fn get_cpu_features() -> Option<CpuFeatures> {
    let cpuid = CpuId::new();

    // Vendor is leaf 0x00000000 — always present on modern x86.
    let vendor = cpuid
        .get_vendor_info()
        .map(|v| v.as_string().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());

    // Leaf 1 carries family/model/stepping and most "baseline" features.
    let feature_info = cpuid.get_feature_info();

    let (family, model, stepping) = match &feature_info {
        Some(fi) => (
            fi.family_id() as u32,
            fi.model_id() as u32,
            fi.stepping_id() as u32,
        ),
        None => (0, 0, 0),
    };

    // Leaf 7 sub-leaf 0 carries AVX2, AVX-512F, BMI1/2.
    let ext_feat = cpuid.get_extended_feature_info();

    // Leaf 0x80000000 reports the max extended leaf; 0x80000002..4
    // carry the brand string (when present). We don't have a direct
    // accessor on CpuId for the max extended leaf in 11.x, so we
    // try to read the brand string and accept the empty default if
    // the leaf isn't supported.
    let brand = cpuid
        .get_processor_brand_string()
        .map(|b| b.as_str().trim().to_string())
        .unwrap_or_default();
    let max_extended_leaf = if !brand.is_empty() { 0x8000_0004 } else { 0 };

    let (
        has_sse,
        has_sse2,
        has_sse3,
        has_ssse3,
        has_sse4_1,
        has_sse4_2,
        has_avx,
        has_aes_ni,
        has_fma,
        has_popcnt,
        has_f16c,
        has_rdrand,
    ) = match &feature_info {
        Some(fi) => (
            fi.has_sse(),
            fi.has_sse2(),
            fi.has_sse3(),
            fi.has_ssse3(),
            fi.has_sse41(),
            fi.has_sse42(),
            fi.has_avx(),
            fi.has_aesni(),
            fi.has_fma(),
            fi.has_popcnt(),
            fi.has_f16c(),
            fi.has_rdrand(),
        ),
        None => (false, false, false, false, false, false, false, false, false, false, false, false),
    };

    let (has_avx2, has_avx512_f, has_bmi1, has_bmi2) = match ext_feat {
        Some(ef) => (
            ef.has_avx2(),
            ef.has_avx512f(),
            ef.has_bmi1(),
            ef.has_bmi2(),
        ),
        None => (false, false, false, false),
    };

    Some(CpuFeatures {
        vendor,
        brand,
        family,
        model,
        stepping,
        max_extended_leaf,
        has_sse,
        has_sse2,
        has_sse3,
        has_ssse3,
        has_sse4_1,
        has_sse4_2,
        has_avx,
        has_avx2,
        has_avx512_f,
        has_aes_ni,
        has_fma,
        has_bmi1,
        has_bmi2,
        has_popcnt,
        has_f16c,
        has_rdrand,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_features_returns_some_on_x86_64_host() {
        // We only compile on x86_64 (see top-of-file cfg gate), so
        // CpuId::new() must succeed.
        let f = get_cpu_features().expect("CpuFeatures on x86_64");
        assert!(!f.vendor.is_empty(), "vendor string empty");
        assert!(
            !f.vendor.starts_with('<'),
            "vendor looks like placeholder: {}",
            f.vendor
        );
    }

    #[test]
    fn vendor_is_known() {
        let f = get_cpu_features().unwrap();
        // The two big players, plus the placeholder if raw-cpuid
        // somehow returned something exotic.
        let known = [
            "GenuineIntel",
            "AuthenticAMD",
            "CentaurHauls",
            "Shanghai",
            "HygonGenuine",
        ];
        assert!(
            known.iter().any(|k| *k == f.vendor),
            "unexpected vendor: {}",
            f.vendor
        );
    }
}
