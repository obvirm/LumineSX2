# Agent Report: x86 Emitter — 16 Missing Instructions Added

## Task
Added 16 categories of missing x86 instructions to `E:\project\pcsx2\rust\common\src\x86_emitter.rs`

## Changed File
- `rust/common/src/x86_emitter.rs` — **742 → 880 lines** (+138 lines)

## Instructions Added

| Category | Methods | iced-x86 Code Enum |
|----------|---------|-------------------|
| INC/DEC | `inc_rm64(dst)`, `dec_rm64(dst)` | `Code::Inc_rm64`, `Code::Dec_rm64` |
| ADC/SBB | `adc_r64_r64(dst,src)`, `sbb_r64_r64(dst,src)` | `Code::Adc_r64_rm64`, `Code::Sbb_r64_rm64` |
| DIV/IDIV | `div_rm64(src)`, `idiv_rm64(src)` | `Code::Div_rm64`, `Code::Idiv_rm64` |
| CDQE | `cdqe()` | `Code::Cdqe` |
| CMOVcc (8) | `cmove_r64_r64`, `cmovne_r64_r64`, `cmovb_r64_r64`, `cmovae_r64_r64`, `cmovl_r64_r64`, `cmovge_r64_r64`, `cmovle_r64_r64`, `cmovg_r64_r64` | `Code::Cmove_r64_rm64`, etc. |
| SETcc (8) | `sete_rm8`, `setne_rm8`, `setb_rm8`, `setae_rm8`, `setl_rm8`, `setge_rm8`, `setle_rm8`, `setg_rm8` | `Code::Sete_rm8`, etc. |
| BSWAP | `bswap_r64(dst)` | `Code::Bswap_r64` |
| MOVBE | `movbe_r64_mem(dst,addr)` (bonus) | `Code::Movbe_r64_m64` |
| String | `stosb()`, `lodsb()` | `Code::Stosb_m8_AL`, `Code::Lodsb_AL_m8` |
| PANDN | `pandn_xmm_xmm(dst,src)` | `Code::Pandn_xmm_xmmm128` |
| MOVHLPS/MOVLHPS | `movhlps_xmm_xmm(dst,src)`, `movlhps_xmm_xmm(dst,src)` | `Code::Movhlps_xmm_xmm`, `Code::Movlhps_xmm_xmm` |

**Total: 30 new methods** covering all requested categories.

## Validation
- `cargo check --lib` — **0 errors from x86_emitter.rs** ✅
- All 26 errors are pre-existing (perf_event_counter, linux_misc, linux_host_sys, cpu_features, x11 crate — none related to our change)
- SETcc uses proper 8-bit register mapping (`Register::R8L` etc., verified against iced-x86 1.21 source)

## Design
- Added `emit_rm1(code, reg)` helper for single-operand rm instructions (INC, DEC, DIV, IDIV, BSWAP, SETcc)
- Added `r64_to_r8()` helper mapping 64-bit GPR IDs to 8-bit register variants (AL..BH, R8L..R15L)
- All new methods follow existing patterns (`emit_rr`, `emit_code`, `emit_rm1`)
