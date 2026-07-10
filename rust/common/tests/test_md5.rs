// Minimal test harness to validate md5_digest.rs against RFC 1321 test vectors.
// This file is independent of the rest of the crate and only imports the
// module under test. To run: from E:\project\pcsx2\rust\common, run
//   rustc --edition 2021 --test tests\test_md5.rs -o test_md5.exe
// then run test_md5.exe.

#[path = "../src/md5_digest.rs"]
mod md5;

fn hex(bytes: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[test]
fn rfc1321_empty() {
    assert_eq!(hex(&md5::MD5Digest::hash(b"")), "d41d8cd98f00b204e9800998ecf8427e");
}

#[test]
fn rfc1321_a() {
    assert_eq!(hex(&md5::MD5Digest::hash(b"a")), "0cc175b9c0f1b6a831c399e269772661");
}

#[test]
fn rfc1321_abc() {
    assert_eq!(hex(&md5::MD5Digest::hash(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
}

#[test]
fn rfc1321_message_digest() {
    assert_eq!(hex(&md5::MD5Digest::hash(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
}

#[test]
fn rfc1321_alphabet() {
    assert_eq!(hex(&md5::MD5Digest::hash(b"abcdefghijklmnopqrstuvwxyz")), "c3fcd3d76192e4007dfb496cca67e13b");
}

#[test]
fn rfc1321_alnum() {
    assert_eq!(
        hex(&md5::MD5Digest::hash(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789")),
        "d174ab98d277d9f5a5611c2c9f419d9f"
    );
}

#[test]
fn rfc1321_digits() {
    assert_eq!(
        hex(&md5::MD5Digest::hash(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")),
        "57edf4a22be3c955ac49da2e2107b67a"
    );
}

#[test]
fn incremental_matches_oneshot() {
    let mut ctx = md5::MD5Digest::new();
    ctx.update(b"ab");
    ctx.update(b"c");
    assert_eq!(hex(&ctx.finalize()), "900150983cd24fb0d6963f7d28e17f72");
}

#[test]
fn reset_works() {
    let mut ctx = md5::MD5Digest::new();
    ctx.update(b"abc");
    ctx.reset();
    ctx.update(b"abc");
    assert_eq!(hex(&ctx.finalize()), "900150983cd24fb0d6963f7d28e17f72");
}

#[test]
fn long_stress_split_7() {
    // 87-byte message, fed both in one shot and as 7-byte chunks. The
    // expected digest was produced by this implementation and verified
    // against an independent MD5 reference (RFC 1321 algorithm).
    let msg = b"The quick brown fox jumps over the lazy dogThe quick brown fox jumps over the lazy dog";
    let expected = "d27c6d8bcaa695e377d32387e115763c";
    assert_eq!(hex(&md5::MD5Digest::hash(msg)), expected);

    // Verify split into 7-byte chunks matches the one-shot.
    let mut ctx = md5::MD5Digest::new();
    for chunk in msg.chunks(7) {
        ctx.update(chunk);
    }
    assert_eq!(hex(&ctx.finalize()), expected);
}

#[test]
fn split_at_block_boundaries() {
    // Feed a long message split at every interesting boundary: sub-block,
    // block-aligned, just past a block, several blocks, etc.
    let payload = b"PCSX2-MD5-port-test-payload-XXXXXXXXX"; // 33 bytes
    let mut full = Vec::new();
    for _ in 0..100 {
        full.extend_from_slice(payload);
    }
    let one_shot = hex(&md5::MD5Digest::hash(&full));

    for split in &[1usize, 7, 31, 32, 33, 63, 64, 65, 127, 128, 129, 192, 255, 1023] {
        let mut ctx = md5::MD5Digest::new();
        for chunk in full.chunks(*split) {
            ctx.update(chunk);
        }
        assert_eq!(
            hex(&ctx.finalize()),
            one_shot,
            "split={} produced different digest",
            split
        );
    }
}
