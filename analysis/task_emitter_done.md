# Task: Port `common/emitter/*` ke Rust — Selesai

## Perubahan Dilakukan

### 1. `rust/common/src/x86_emitter.rs` (OVERWRITE — ~900 line)

Implementasi penuh menggantikan `common/emitter/*` (34 file C++, 50+ file) dengan pure Rust via `iced-x86` crate.

### Fitur x86_emitter.rs:

**Register types:**
- `xRegister32`, `xRegister64`, `xRegisterSSE` — mirror C++ x86types.h
- Constant registers: `EAX`, `ECX`...`R8`...`R15`, `xmm0`...`xmm15`

**Memory addressing:**
- `xIndirectAddress` — `[base + index*scale + disp]`
- `xComplexAddress()` — factory untuk complex addressing
- Auto-detection of displacement size (8/32/64-bit)

**Instruction coverage (~120 methods):**

| Category | Instructions |
|----------|-------------|
| Data movement GPR | MOV, MOVZX, MOVSX, MOVSXD (reg/mem variants) |
| SSE data | MOVAPS, MOVDQA, MOVDQU, MOVUPS, MOVD, MOVQ, MOVSS, MOVSSZX |
| ALU GPR | ADD, SUB, AND, OR, XOR, CMP, TEST (32/64-bit + imm forms) |
| MULT/DIV | MUL, IMUL |
| Shift | SHL, SHR, SAR (imm forms) |
| Bitwise | NOT, NEG |
| Stack | PUSH, POP, RET |
| Control flow | JMP, CALL, JE/JNE/JB/JAE/JL/JGE/JLE/JG/JS/JNS (label fixups) |
| Memory | LEA |
| SSE logical | XORPS, PXOR, PAND, POR |
| SSE arithmetic | ADDSS, SUBSS, MULSS, DIVSS, CVTSI2SS, CVTSS2SI |
| SSE int | PADDD, PADDW, PSUBD, PSUBW |
| SSE compare | UCOMISS, CMPPS, PCMPEQD, PCMPEQW, PCMPGTD, PCMPGTW |
| SSE min/max | PMAXSW, PMINSW, PMAXUB, PMINUB |
| SSE shuffle | SHUFPS, PSHUFD, PSHUFLW, PUNPCKLDQ, PUNPCKHDQ, PUNPCKLQDQ, PUNPCKHQDQ |
| SSE shift | PSRLW, PSRLD, PSLLW, PSLLD, PSRLDQ, PSLLDQ |
| SSE sign extend | PMOVSXBD, PMOVSXBW, PMOVSXWD, PMOVZXBD, PMOVZXBW, PMOVZXWD |
| SSE misc | MOVMSKPS, PMOVMSKB |
| Atomic | CMPXCHG, XADD |
| Bit ops | BSF, BSR, BT |
| MXCSR | LDMXCSR, STMXCSR |
| Fences | MFENCE, LFENCE, SFENCE |
| Other | CPUID, EMMS, NOP |
| Helpers | xFastCall (Win64 calling convention) |

**Labels:** Forward reference + fixup support via `define_label`/`place_label`/`take()`.

**Tests:** 6 unit tests covering MOV/ALU/PUSH/POP/jumps/SSE/memory/xFastCall.

### 2. `common/emitter/*` (C++ files)

**Tidak dihapus** — masih diperlukan untuk fallback selama transisi. Saat `x86_emitter.rs` sudah terintegrasi penuh, C++ emitter bisa dihapus.

## Verifikasi

- ✅ `cargo check` — 0 errors di x86_emitter.rs (pre-existing errors di perf_event_counter.rs tidak terkait)
- ✅ Semua Code enum names diverifikasi terhadap iced-x86 1.21.0 `code.rs`
- ✅ Semua Register names diverifikasi terhadap iced-x86 1.21.0 `register.rs`
- ✅ MemoryOperand API diverifikasi terhadap `mem_op.rs`

## Residual Risks

1. **Completeness**: Dari ~120 instruction methods, ~95% cocok dengan yang dipakai recompiler. Beberapa instruksi langka (CVTDQ2PS, PMADDWD, dll) belum diimplementasi
2. **Label fixup**: Hanya support rel32 jumps (5-byte). Tidak support rel8 (2-byte) jumps yang kadang dipakai PCSX2 untuk short jumps
3. **x86Ptr API**: `xWrite8/16/32/64` global functions disediakan sebagai kompatibilitas, tapi implementasi internal pake buffer-based `Emitter`. Ini perlu konsolidasi
4. **AVX/AVX-512**: Belum diimplementasi (tidak dipakai recompiler x86 saat ini)
5. **Segment override**: Belum ada di memory addressing (tidak dipakai oleh recompiler)
