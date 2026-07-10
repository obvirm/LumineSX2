//! `CtestTests` - idiomatic Rust translation of the PCSX2 ctest suite.
//!
//! This module re-implements a minimum-viable subset of the utilities
//! exercised by the C++ GoogleTest cases under `tests/ctest/` (byteswap,
//! file system, path, small string, string util, GS swizzle, memory
//! interface, and patch engine) and exposes one `#[test]` function per
//! subsystem. It compiles only during `cargo test` and depends solely on
//! the standard library.

#[cfg(test)]
mod ctest_tests {
    use std::cmp::Ordering;
    use std::collections::HashMap;
    use std::fmt::Display;
    use std::fs;
    use std::io;
    use std::path::Path;

    // =================================================================
    // Byteswap
    // =================================================================

    #[inline] fn bswap16(x: u16) -> u16 { x.swap_bytes() }
    #[inline] fn bswap32(x: u32) -> u32 { x.swap_bytes() }
    #[inline] fn bswap64(x: u64) -> u64 { x.swap_bytes() }

    #[test]
    fn test_byteswap() {
        // From byteswap_tests.cpp: ByteSwap(ByteSwap(...)) and round-trips.
        assert_eq!(bswap16(0xabcd), 0xcdab);
        assert_eq!(bswap32(0xabcdef01), 0x01efcdab);
        assert_eq!(bswap64(0xabcdef0123456789u64), 0x8967452301efcdab);
        assert_eq!(bswap32(0x80123456u32), 0x56341280);
        for v in [0u16, 1, 0x1234, 0xffffu16] {
            assert_eq!(bswap16(bswap16(v)), v);
        }
        for v in [0u32, 1, 0x12345678, 0xdeadbeef, 0xffff_ffff] {
            assert_eq!(bswap32(bswap32(v)), v);
        }
        for v in [0u64, 1, 0x123456789abcdef0, u64::MAX] {
            assert_eq!(bswap64(bswap64(v)), v);
        }
    }

    // =================================================================
    // File system
    // =================================================================

    fn file_exists(p: &str) -> bool {
        Path::new(p).is_file()
    }

    fn recursive_copy(src: &Path, dst: &Path) -> io::Result<()> {
        if src.is_file() {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(src, dst)?;
            return Ok(());
        }
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let e = entry?;
            recursive_copy(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    }

    fn unique_tmp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "ctest_{}_{}_{}",
            label,
            std::process::id(),
            nanos
        ))
    }

    #[test]
    fn test_filesystem() {
        let base = unique_tmp_dir("filesystem");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // file_exists: false for missing, true after write
        let f = base.join("hello.txt");
        assert!(!file_exists(f.to_str().unwrap()));
        fs::write(&f, b"hello").unwrap();
        assert!(file_exists(f.to_str().unwrap()));

        // recursive_copy: build a small tree and copy it
        let src = base.join("src");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(src.join("a.txt"), b"alpha").unwrap();
        fs::write(src.join("nested").join("b.txt"), b"beta").unwrap();

        let dst = base.join("dst");
        recursive_copy(&src, &dst).unwrap();
        assert!(file_exists(dst.join("a.txt").to_str().unwrap()));
        assert!(file_exists(dst.join("nested").join("b.txt").to_str().unwrap()));
        assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(fs::read(dst.join("nested").join("b.txt")).unwrap(), b"beta");

        let _ = fs::remove_dir_all(&base);
    }

    // =================================================================
    // Path
    // =================================================================

    fn sep() -> char {
        if cfg!(windows) { '\\' } else { '/' }
    }

    fn is_sep(c: char) -> bool {
        c == '/' || c == '\\'
    }

    fn to_native_path(s: &str) -> String {
        let mut out = if cfg!(windows) {
            s.replace('/', "\\")
        } else {
            s.replace('\\', "/")
        };
        // Trim trailing separators (but never past the leading double-sep on
        // Windows, which marks a UNC root).
        let trim_sep = if cfg!(windows) { '\\' } else { '/' };
        let min_len = if cfg!(windows) && out.starts_with("\\\\") { 2 } else { 0 };
        while out.len() > min_len && out.ends_with(trim_sep) {
            out.pop();
        }
        out
    }

    fn is_absolute(s: &str) -> bool {
        if s.is_empty() { return false; }
        if cfg!(windows) {
            let bytes = s.as_bytes();
            if bytes.len() >= 2 && bytes[1] == b':' {
                if bytes.len() >= 3 && is_sep(bytes[2] as char) { return true; }
                return false;
            }
            if s.starts_with("\\\\") { return true; }
            false
        } else {
            s.starts_with('/')
        }
    }

    fn path_combine(a: &str, b: &str) -> String {
        let a_clean = a.trim_end_matches(is_sep);
        let b_clean = b.trim_start_matches(is_sep);
        let mut out = String::from(a_clean);
        if !a_clean.is_empty() && !b_clean.is_empty() {
            out.push(sep());
        }
        out.push_str(b_clean);
        out
    }

    fn canonicalize(s: &str) -> String {
        if s.is_empty() { return to_native_path(""); }
        let native = to_native_path(s);
        let bytes = native.as_bytes();
        let mut prefix = String::new();
        let mut first = 0;
        if cfg!(windows) && bytes.len() >= 2 && bytes[1] == b':' {
            prefix = format!("{}{}", &native[..1], sep());
            first = 2;
            if first < bytes.len() && is_sep(bytes[first] as char) {
                first += 1;
            }
        } else if cfg!(windows) && bytes.len() >= 2 && bytes[0] == b'\\' && bytes[1] == b'\\' {
            prefix = "\\\\".into();
            first = 2;
        } else if !native.is_empty() && bytes[0] == b'/' {
            prefix = sep().to_string();
            first = 1;
        }
        let mut parts: Vec<&str> = Vec::new();
        for part in native[first..].split(|c: char| c == sep()) {
            match part {
                "" | "." => {}
                ".." => {
                    if !parts.is_empty() && parts[parts.len() - 1] != ".." {
                        parts.pop();
                    } else {
                        parts.push("..");
                    }
                }
                _ => parts.push(part),
            }
        }
        let mut out = prefix;
        out.push_str(&parts.join(&sep().to_string()));
        out
    }

    fn get_file_name(s: &str) -> String {
        if s.is_empty() { return String::new(); }
        let native = to_native_path(s);
        match native.rfind(sep()) {
            Some(i) => native[i + 1..].to_string(),
            None => native,
        }
    }

    fn get_directory(s: &str) -> String {
        if s.is_empty() { return String::new(); }
        let native = to_native_path(s);
        match native.rfind(sep()) {
            Some(i) => native[..i].to_string(),
            None => String::new(),
        }
    }

    fn get_file_title(s: &str) -> String {
        let name = get_file_name(s);
        if name.is_empty() || name == "." { return String::new(); }
        match name.rfind('.') {
            Some(i) if i > 0 => name[..i].to_string(),
            _ => name,
        }
    }

    fn get_extension(s: &str) -> String {
        let name = get_file_name(s);
        if name.is_empty() || name == "." { return String::new(); }
        match name.rfind('.') {
            Some(i) if i > 0 => name[i + 1..].to_string(),
            _ => String::new(),
        }
    }

    fn change_file_name(s: &str, new_name: &str) -> String {
        let dir = get_directory(s);
        path_combine(&dir, new_name)
    }

    fn append_directory(base: &str, dir: &str) -> String {
        if base.is_empty() { return dir.to_string(); }
        let native = to_native_path(base);
        match native.find(sep()) {
            Some(idx) => {
                let head = &native[..idx];
                let tail = &native[idx + sep().len_utf8()..];
                if dir.is_empty() {
                    if tail.is_empty() {
                        head.to_string()
                    } else {
                        format!("{}{}{}", head, sep(), tail)
                    }
                } else {
                    format!("{}{}{}{}{}", head, sep(), dir, sep(), tail)
                }
            }
            None => {
                if dir.is_empty() {
                    base.to_string()
                } else {
                    format!("{}{}{}", dir, sep(), base)
                }
            }
        }
    }

    fn make_relative(path: &str, base: &str) -> String {
        if path.is_empty() { return to_native_path(""); }
        if base.is_empty() { return to_native_path(path); }
        let p_parts: Vec<&str> = to_native_path(path).split(sep()).collect();
        let b_parts: Vec<&str> = to_native_path(base).split(sep()).collect();
        let mut common = 0;
        while common < p_parts.len()
            && common < b_parts.len()
            && p_parts[common] == b_parts[common]
        {
            common += 1;
        }
        let mut out: Vec<String> = Vec::new();
        for _ in common..b_parts.len() {
            out.push("..".to_string());
        }
        for part in &p_parts[common..] {
            out.push((*part).to_string());
        }
        if out.is_empty() {
            to_native_path("")
        } else {
            out.join(&sep().to_string())
        }
    }

    fn create_file_url(s: &str) -> String {
        let native = to_native_path(s);
        if cfg!(windows) {
            let bytes = native.as_bytes();
            if native.starts_with("\\\\") {
                format!("file://{}", &native[2..].replace('\\', "/"))
            } else if bytes.len() >= 2 && bytes[1] == b':' {
                let drive = &native[..1];
                let rest = &native[2..]
                    .trim_start_matches(|c: char| is_sep(c))
                    .replace('\\', "/");
                format!("file:///{}/{}", drive, rest)
            } else {
                format!("file://{}", native.replace('\\', "/"))
            }
        } else {
            format!("file://{}", native)
        }
    }

    #[test]
    fn test_path() {
        // ToNativePath
        assert_eq!(to_native_path(""), "");
        if cfg!(windows) {
            assert_eq!(to_native_path("foo"), "foo");
            assert_eq!(to_native_path("foo\\"), "foo");
            assert_eq!(to_native_path("foo\\\\bar"), "foo\\bar");
            assert_eq!(to_native_path("foo\\bar"), "foo\\bar");
            assert_eq!(to_native_path("foo/bar"), "foo\\bar");
            assert_eq!(to_native_path("\\\\foo\\bar\\baz"), "\\\\foo\\bar\\baz");
        } else {
            assert_eq!(to_native_path("foo"), "foo");
            assert_eq!(to_native_path("foo/"), "foo");
            assert_eq!(to_native_path("foo//bar"), "foo/bar");
            assert_eq!(to_native_path("foo/bar"), "foo/bar");
            assert_eq!(to_native_path("foo\\bar"), "foo/bar");
            assert_eq!(to_native_path("/foo/bar/baz"), "/foo/bar/baz");
        }

        // IsAbsolute
        assert!(!is_absolute(""));
        assert!(!is_absolute("foo"));
        assert!(!is_absolute("foo/bar"));
        if cfg!(windows) {
            assert!(is_absolute("C:\\foo/bar"));
            assert!(is_absolute("C://foo\\bar"));
            assert!(!is_absolute("\\foo/bar"));
            assert!(is_absolute("\\\\foo\\bar\\baz"));
        } else {
            assert!(is_absolute("/foo/bar"));
        }

        // Canonicalize
        assert_eq!(canonicalize(""), to_native_path(""));
        assert_eq!(canonicalize("foo/bar/../baz"), to_native_path("foo/baz"));
        assert_eq!(canonicalize("foo/bar/./baz"), to_native_path("foo/bar/baz"));
        assert_eq!(canonicalize("foo/./bar/./baz"), to_native_path("foo/bar/baz"));
        assert_eq!(canonicalize("foo/bar/../baz/../foo"), to_native_path("foo/foo"));
        assert_eq!(canonicalize("foo/bar/../baz/./foo"), to_native_path("foo/baz/foo"));
        assert_eq!(canonicalize("./foo"), to_native_path("foo"));
        assert_eq!(canonicalize("../foo"), to_native_path("../foo"));
        assert_eq!(
            canonicalize("foo/b🙃ar/../b🙃az/./foo"),
            to_native_path("foo/b🙃az/foo")
        );
        if !cfg!(windows) {
            assert_eq!(canonicalize("/foo/bar/../baz/./foo"), "/foo/baz/foo");
        }

        // Combine
        assert_eq!(path_combine("", ""), to_native_path(""));
        assert_eq!(path_combine("foo", "bar"), to_native_path("foo/bar"));
        assert_eq!(path_combine("foo/bar", "baz"), to_native_path("foo/bar/baz"));
        assert_eq!(path_combine("foo/bar/", "/baz/"), to_native_path("foo/bar/baz"));
        assert_eq!(path_combine("foo//bar", "baz/"), to_native_path("foo/bar/baz"));

        // GetFileName
        assert_eq!(get_file_name(""), "");
        assert_eq!(get_file_name("foo"), "foo");
        assert_eq!(get_file_name("foo.txt"), "foo.txt");
        assert_eq!(get_file_name("foo/bar/baz"), "baz");
        assert_eq!(get_file_name("foo/bar/baz.txt"), "baz.txt");

        // GetFileTitle
        assert_eq!(get_file_title("foo"), "foo");
        assert_eq!(get_file_title("foo.txt"), "foo");
        assert_eq!(get_file_title("foo/bar/baz"), "baz");
        assert_eq!(get_file_title("foo/bar/baz.txt"), "baz");

        // GetDirectory
        assert_eq!(get_directory(""), "");
        assert_eq!(get_directory("foo"), "");
        assert_eq!(get_directory("foo.txt"), "");
        assert_eq!(get_directory("foo/bar/baz"), "foo/bar");

        // GetExtension
        assert_eq!(get_extension("foo"), "");
        assert_eq!(get_extension("foo.txt"), "txt");
        assert_eq!(get_extension("foo."), "");
        assert_eq!(get_extension("a/b/foo.txt"), "txt");

        // ChangeFileName
        assert_eq!(change_file_name("", ""), to_native_path(""));
        assert_eq!(change_file_name("foo/bar", "baz"), to_native_path("foo/baz"));
        assert_eq!(change_file_name("foo/bar.txt", "baz.txt"), to_native_path("foo/baz.txt"));

        // AppendDirectory
        assert_eq!(append_directory("foo/bar", "baz"), to_native_path("foo/baz/bar"));
        assert_eq!(append_directory("", "baz"), to_native_path("baz"));
        assert_eq!(append_directory("", ""), to_native_path(""));
        assert_eq!(append_directory("foo/bar", "🙃"), to_native_path("foo/🙃/bar"));

        // MakeRelative
        assert_eq!(make_relative("", ""), to_native_path(""));
        assert_eq!(make_relative("foo", ""), to_native_path("foo"));
        assert_eq!(make_relative("", "foo"), to_native_path(""));
        assert_eq!(make_relative("foo", "bar"), to_native_path("foo"));
        let abs_root: &str = if cfg!(windows) { "C:\\" } else { "/" };
        assert_eq!(make_relative(&format!("{}foo", abs_root), &format!("{}bar", abs_root)),
            to_native_path("../foo"));
        assert_eq!(make_relative(&format!("{}foo/bar", abs_root), &format!("{}foo", abs_root)),
            to_native_path("bar"));
        assert_eq!(make_relative(&format!("{}foo/bar", abs_root), &format!("{}foo/baz", abs_root)),
            to_native_path("../bar"));

        // CreateFileURL
        if cfg!(windows) {
            assert_eq!(create_file_url("C:\\foo\\bar"), "file:///C:/foo/bar");
        } else {
            assert_eq!(create_file_url("/foo/bar"), "file:///foo/bar");
        }
    }

    // =================================================================
    // SmallString
    // =================================================================

    struct SmallString<const N: usize> {
        buf: [u8; N],
        len: usize,
    }

    impl<const N: usize> SmallString<N> {
        fn new() -> Self {
            Self { buf: [0u8; N], len: 0 }
        }

        fn from_str(s: &str) -> Self {
            let mut me = Self::new();
            for c in s.chars() {
                me.push(c);
            }
            me
        }

        fn push(&mut self, c: char) {
            let mut scratch = [0u8; 4];
            let encoded = c.encode_utf8(&mut scratch);
            for b in encoded.bytes() {
                if self.len < N {
                    self.buf[self.len] = b;
                    self.len += 1;
                }
            }
        }

        fn pop(&mut self) -> Option<char> {
            if self.len == 0 { return None; }
            // Walk back over any UTF-8 continuation bytes to find the start
            // of the last code point.
            let mut start = self.len;
            while start > 0 && (self.buf[start - 1] & 0xC0) == 0x80 {
                start -= 1;
            }
            let s = std::str::from_utf8(&self.buf[start..self.len]).ok()?;
            let c = s.chars().next()?;
            self.len = start;
            Some(c)
        }

        fn clear(&mut self) {
            self.len = 0;
        }

        fn as_str(&self) -> &str {
            std::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
        }
    }

    #[test]
    fn test_small_string() {
        // C++: SmallStackString<6> string("Hello"); string = string;
        //      ASSERT_STREQ(string.c_str(), "Hello");
        let mut s: SmallString<16> = SmallString::from_str("Hello");
        s = s; // self-assignment must be safe
        assert_eq!(s.as_str(), "Hello");

        // push
        let mut s: SmallString<16> = SmallString::new();
        s.push('H');
        s.push('i');
        assert_eq!(s.as_str(), "Hi");
        s.push('!');
        assert_eq!(s.as_str(), "Hi!");

        // pop
        assert_eq!(s.pop(), Some('!'));
        assert_eq!(s.as_str(), "Hi");
        assert_eq!(s.pop(), Some('i'));
        assert_eq!(s.as_str(), "H");

        // clear
        s.clear();
        assert_eq!(s.as_str(), "");
        assert_eq!(s.len(), 0);
        assert_eq!(s.pop(), None);

        // multi-byte character handling
        let mut s: SmallString<16> = SmallString::new();
        s.push('🙃');
        assert_eq!(s.as_str(), "🙃");
        assert_eq!(s.pop(), Some('🙃'));
        assert_eq!(s.as_str(), "");
    }

    // =================================================================
    // String util
    // =================================================================

    fn stricmp(a: &str, b: &str) -> Ordering {
        a.to_lowercase().cmp(&b.to_lowercase())
    }

    fn to_lower(s: &str) -> String {
        s.to_lowercase()
    }

    fn to_chars<T: Display>(v: T) -> String {
        v.to_string()
    }

    fn ellipsise(s: &str, max_len: usize, ellipsis: &str) -> String {
        if s.len() <= max_len { return s.to_string(); }
        if max_len <= ellipsis.len() {
            return ellipsis.chars().take(max_len).collect();
        }
        let cut = max_len - ellipsis.len();
        format!("{}{}", &s[..cut], ellipsis)
    }

    fn ellipsise_in_place(s: &mut String, max_len: usize, ellipsis: &str) {
        *s = ellipsise(s, max_len, ellipsis);
    }

    #[test]
    fn test_string_util() {
        // Stricmp
        assert_eq!(stricmp("Hello", "HELLO"), Ordering::Equal);
        assert_eq!(stricmp("hello", "world"), Ordering::Less);
        assert_eq!(stricmp("world", "hello"), Ordering::Greater);
        assert_eq!(stricmp("abc", "abcd"), Ordering::Less);

        // to_lower
        assert_eq!(to_lower("Hello World"), "hello world");
        assert_eq!(to_lower("ABC123"), "abc123");
        assert_eq!(to_lower(""), "");

        // ToChars
        assert_eq!(to_chars(false), "false");
        assert_eq!(to_chars(true), "true");
        assert_eq!(to_chars(0), "0");
        assert_eq!(to_chars(-1337), "-1337");
        assert_eq!(to_chars(1337), "1337");
        assert_eq!(to_chars(1337u32), "1337");
        assert_eq!(to_chars(13.37f32), "13.37");
        assert_eq!(to_chars(255i32), "ff"); // mimics StringUtil::ToChars(255, 16) with hex formatting - here shown as decimal

        // Ellipsise
        assert_eq!(ellipsise("HelloWorld", 6, "..."), "Hel...");
        assert_eq!(ellipsise("HelloWorld", 7, ".."), "Hello..");
        assert_eq!(ellipsise("HelloWorld", 20, ".."), "HelloWorld");
        assert_eq!(ellipsise("", 20, "..."), "");
        assert_eq!(ellipsise("Hello", 10, "..."), "Hello");

        // EllipsiseInPlace
        let mut s = String::from("HelloWorld");
        ellipsise_in_place(&mut s, 6, "...");
        assert_eq!(s, "Hel...");
        s = String::from("HelloWorld");
        ellipsise_in_place(&mut s, 7, "..");
        assert_eq!(s, "Hello..");
        s = String::from("HelloWorld");
        ellipsise_in_place(&mut s, 20, "..");
        assert_eq!(s, "HelloWorld");
    }

    // =================================================================
    // GS swizzle
    // =================================================================

    fn swizzle_pixels(table: &[u8], dst: &mut [u8], src: &[u8], bpp: usize, deswizzle: bool) {
        let pxbytes = bpp / 8;
        if pxbytes == 0 { return; }
        for i in 0..(256 / pxbytes) {
            let soff = if deswizzle { table[i] as usize } else { i } * pxbytes;
            let doff = if deswizzle { i } else { table[i] as usize } * pxbytes;
            if doff + pxbytes <= dst.len() && soff + pxbytes <= src.len() {
                dst[doff..doff + pxbytes].copy_from_slice(&src[soff..soff + pxbytes]);
            }
        }
    }

    fn swizzle_h(table: &[u8], dst: &mut [u32], src: &[u8], bpp: usize, shift: u32) {
        for i in 0..64 {
            let spx: u32 = if bpp == 8 {
                src[i] as u32
            } else {
                ((src[i >> 1] >> ((i & 1) * 4)) & 0xF) as u32
            };
            let spx = spx << shift;
            let idx = table[i] as usize;
            if idx < dst.len() {
                dst[idx] = spx;
            }
        }
    }

    #[test]
    fn test_swizzle() {
        // Identity table: swizzle then deswizzle returns the original buffer.
        let identity: Vec<u8> = (0..256u32).map(|i| (i & 0xFF) as u8).collect();
        let src: Vec<u8> = (0..256u32).map(|i| (i & 0xFF) as u8).collect();
        let mut swizzled = vec![0u8; 256];
        swizzle_pixels(&identity, &mut swizzled, &src, 8, false);
        let mut round_trip = vec![0u8; 256];
        swizzle_pixels(&identity, &mut round_trip, &swizzled, 8, true);
        assert_eq!(round_trip, src);

        // Reversed table: swizzled[i] should equal src[255 - i].
        let reverse: Vec<u8> = (0..256u32).rev().map(|i| (i & 0xFF) as u8).collect();
        let mut swizzled = vec![0u8; 256];
        swizzle_pixels(&reverse, &mut swizzled, &src, 8, false);
        for i in 0..256 {
            assert_eq!(swizzled[i], src[255 - i], "mismatch at {}", i);
        }

        // swizzle_h with an identity table and 8 bpp simply re-packs.
        let table32: Vec<u8> = (0..64u32).map(|i| (i & 0xFF) as u8).collect();
        let src_h: Vec<u8> = (0..64u32).map(|i| (i & 0xFF) as u8).collect();
        let mut dst_h = vec![0u32; 64];
        swizzle_h(&table32, &mut dst_h, &src_h, 8, 24);
        for i in 0..64 {
            assert_eq!(dst_h[i], (i as u32) << 24);
        }
    }

    // =================================================================
    // Memory interface
    // =================================================================

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum MemoryOp {
        Read8 { addr: u32, value: u8 },
        Read16 { addr: u32, value: u16 },
        Read32 { addr: u32, value: u32 },
        Read64 { addr: u32, value: u64 },
        Write8 { addr: u32, value: u8 },
        Write16 { addr: u32, value: u16 },
        Write32 { addr: u32, value: u32 },
        Write64 { addr: u32, value: u64 },
    }

    trait MemoryInterface {
        fn read8(&mut self, addr: u32, valid: &mut bool) -> u8;
        fn read16(&mut self, addr: u32, valid: &mut bool) -> u16;
        fn read32(&mut self, addr: u32, valid: &mut bool) -> u32;
        fn read64(&mut self, addr: u32, valid: &mut bool) -> u64;
        fn write8(&mut self, addr: u32, value: u8) -> bool;
        fn write16(&mut self, addr: u32, value: u16) -> bool;
        fn write32(&mut self, addr: u32, value: u32) -> bool;
        fn write64(&mut self, addr: u32, value: u64) -> bool;
    }

    struct MockMemory {
        log: Vec<MemoryOp>,
        state: HashMap<u32, u64>,
    }

    impl MockMemory {
        fn new() -> Self {
            Self { log: Vec::new(), state: HashMap::new() }
        }
        fn ops(&self) -> &[MemoryOp] { &self.log }
    }

    impl MemoryInterface for MockMemory {
        fn read8(&mut self, addr: u32, _valid: &mut bool) -> u8 {
            let v = self.state.get(&addr).copied().unwrap_or(0) as u8;
            self.log.push(MemoryOp::Read8 { addr, value: v });
            v
        }
        fn read16(&mut self, addr: u32, _valid: &mut bool) -> u16 {
            let v = self.state.get(&addr).copied().unwrap_or(0) as u16;
            self.log.push(MemoryOp::Read16 { addr, value: v });
            v
        }
        fn read32(&mut self, addr: u32, _valid: &mut bool) -> u32 {
            let v = self.state.get(&addr).copied().unwrap_or(0) as u32;
            self.log.push(MemoryOp::Read32 { addr, value: v });
            v
        }
        fn read64(&mut self, addr: u32, _valid: &mut bool) -> u64 {
            let v = self.state.get(&addr).copied().unwrap_or(0);
            self.log.push(MemoryOp::Read64 { addr, value: v });
            v
        }
        fn write8(&mut self, addr: u32, value: u8) -> bool {
            self.log.push(MemoryOp::Write8 { addr, value });
            self.state.insert(addr, value as u64);
            true
        }
        fn write16(&mut self, addr: u32, value: u16) -> bool {
            self.log.push(MemoryOp::Write16 { addr, value });
            self.state.insert(addr, value as u64);
            true
        }
        fn write32(&mut self, addr: u32, value: u32) -> bool {
            self.log.push(MemoryOp::Write32 { addr, value });
            self.state.insert(addr, value as u64);
            true
        }
        fn write64(&mut self, addr: u32, value: u64) -> bool {
            self.log.push(MemoryOp::Write64 { addr, value });
            self.state.insert(addr, value);
            true
        }
    }

    #[test]
    fn test_mock_memory_interface() {
        let mut m = MockMemory::new();
        let mut valid = false;

        // Exercise every trait method through dyn dispatch.
        let _: u8 = m.read8(0x10, &mut valid);
        let _: u16 = m.read16(0x20, &mut valid);
        let _: u32 = m.read32(0x30, &mut valid);
        let _: u64 = m.read64(0x40, &mut valid);
        assert!(m.write8(0x50, 0xab));
        assert!(m.write16(0x60, 0x1234));
        assert!(m.write32(0x70, 0xdeadbeef));
        assert!(m.write64(0x80, 0x1122334455667788));

        // Writes are visible to subsequent reads via the mock state.
        let v8 = m.read8(0x50, &mut valid);
        let v32 = m.read32(0x70, &mut valid);
        assert_eq!(v8, 0xab);
        assert_eq!(v32, 0xdeadbeef);

        // The recorded log captures every operation in order.
        assert_eq!(
            m.ops(),
            &[
                MemoryOp::Read8 { addr: 0x10, value: 0 },
                MemoryOp::Read16 { addr: 0x20, value: 0 },
                MemoryOp::Read32 { addr: 0x30, value: 0 },
                MemoryOp::Read64 { addr: 0x40, value: 0 },
                MemoryOp::Write8 { addr: 0x50, value: 0xab },
                MemoryOp::Write16 { addr: 0x60, value: 0x1234 },
                MemoryOp::Write32 { addr: 0x70, value: 0xdeadbeef },
                MemoryOp::Write64 { addr: 0x80, value: 0x1122334455667788 },
                MemoryOp::Read8 { addr: 0x50, value: 0xab },
                MemoryOp::Read32 { addr: 0x70, value: 0xdeadbeef },
            ]
        );
    }

    // =================================================================
    // Patch engine
    // =================================================================

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PatchPlace {
        OnceOnLoad,
        Continuously,
        Combined01,
        OnLoadOrWhenEnabled,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PatchCpu { Ee, Iop }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PatchType {
        Byte,
        Short,
        Word,
        Double,
        ShortBe,
        WordBe,
        DoubleBe,
        Extended,
    }

    #[derive(Clone, Copy, Debug)]
    struct PatchCommand {
        place: PatchPlace,
        cpu: PatchCpu,
        addr: u32,
        kind: PatchType,
        data: u64,
    }

    fn build_patch(
        place: PatchPlace,
        cpu: PatchCpu,
        addr: u32,
        kind: PatchType,
        data: u64,
    ) -> PatchCommand {
        PatchCommand { place, cpu, addr, kind, data }
    }

    fn apply_patches(
        patches: &[&PatchCommand],
        mode: PatchPlace,
        ee: &mut dyn MemoryInterface,
        iop: &mut dyn MemoryInterface,
    ) {
        for p in patches {
            let include = match mode {
                PatchPlace::Combined01 => {
                    p.place == PatchPlace::OnceOnLoad
                        || p.place == PatchPlace::Continuously
                }
                _ => p.place == mode,
            };
            if !include { continue; }
            let bus: &mut dyn MemoryInterface = match p.cpu {
                PatchCpu::Ee => ee,
                PatchCpu::Iop => iop,
            };
            let mut valid = true;
            match p.kind {
                PatchType::Byte => { bus.write8(p.addr, p.data as u8); }
                PatchType::Short => { bus.write16(p.addr, p.data as u16); }
                PatchType::Word => { bus.write32(p.addr, p.data as u32); }
                PatchType::Double => { bus.write64(p.addr, p.data); }
                PatchType::ShortBe => {
                    bus.write16(p.addr, (p.data as u16).swap_bytes());
                }
                PatchType::WordBe => {
                    bus.write32(p.addr, (p.data as u32).swap_bytes());
                }
                PatchType::DoubleBe => {
                    bus.write64(p.addr, p.data.swap_bytes());
                }
                PatchType::Extended => apply_extended(p, bus, &mut valid),
            }
        }
    }

    fn apply_extended(
        cmd: &PatchCommand,
        bus: &mut dyn MemoryInterface,
        valid: &mut bool,
    ) {
        let top = (cmd.addr >> 24) & 0xFF;
        let sub = cmd.addr & 0x00FF_FFFF;
        match top {
            // Top byte 0x00: simple write, low nibble of (sub>>20) is size.
            0x00 => {
                let sz = (sub >> 20) & 0xF;
                match sz {
                    1 => {
                        let old = bus.read8(cmd.addr, valid);
                        if old != cmd.data as u8 {
                            bus.write8(cmd.addr, cmd.data as u8);
                        }
                    }
                    2 => {
                        let old = bus.read16(cmd.addr, valid);
                        if old != cmd.data as u16 {
                            bus.write16(cmd.addr, cmd.data as u16);
                        }
                    }
                    4 => {
                        let old = bus.read32(cmd.addr, valid);
                        if old != cmd.data as u32 {
                            bus.write32(cmd.addr, cmd.data as u32);
                        }
                    }
                    _ => {}
                }
            }
            // 8-bit increment / decrement.
            0x30 => {
                let cur = bus.read8(cmd.addr, valid);
                bus.write8(cmd.addr, cur.wrapping_add(cmd.data as u8));
            }
            0x31 => {
                let cur = bus.read8(cmd.addr, valid);
                bus.write8(cmd.addr, cur.wrapping_sub(cmd.data as u8));
            }
            // 16-bit increment / decrement.
            0x32 => {
                let cur = bus.read16(cmd.addr, valid);
                bus.write16(cmd.addr, cur.wrapping_add(cmd.data as u16));
            }
            0x33 => {
                let cur = bus.read16(cmd.addr, valid);
                bus.write16(cmd.addr, cur.wrapping_sub(cmd.data as u16));
            }
            // 32-bit increment / decrement.
            0x34 => {
                let cur = bus.read32(cmd.addr, valid);
                bus.write32(cmd.addr, cur.wrapping_add(cmd.data as u32));
            }
            0x35 => {
                let cur = bus.read32(cmd.addr, valid);
                bus.write32(cmd.addr, cur.wrapping_sub(cmd.data as u32));
            }
            // Copy bytes: sub>>20 is the byte count, data is the destination.
            0x50 => {
                let n = (sub >> 20) & 0xF;
                for i in 0..n {
                    let b = bus.read8(cmd.addr + i as u32, valid);
                    bus.write8(cmd.data as u32 + i as u32, b);
                }
            }
            // Bitwise ops: high nibble of (sub>>20) is op (1=or,2=and,4=xor);
            // low nibble of (sub>>20) is size (0=8-bit, 1=16-bit).
            0x70 => {
                let sz = (sub >> 16) & 0xF;
                let op = (sub >> 20) & 0xF;
                if sz == 0 {
                    let cur = bus.read8(cmd.addr, valid);
                    let new = match op {
                        1 => cur | cmd.data as u8,
                        2 => cur & cmd.data as u8,
                        4 => cur ^ cmd.data as u8,
                        _ => cur,
                    };
                    bus.write8(cmd.addr, new);
                } else if sz == 1 {
                    let cur = bus.read16(cmd.addr, valid);
                    let new = match op {
                        1 => cur | cmd.data as u16,
                        2 => cur & cmd.data as u16,
                        4 => cur ^ cmd.data as u16,
                        _ => cur,
                    };
                    bus.write16(cmd.addr, new);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn test_patches() {
        // Plain Byte write (idempotent with old=0, new=0x12).
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x0010_0000, PatchType::Byte, 0x12,
            );
            apply_patches(
                &[&c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[MemoryOp::Write8 { addr: 0x0010_0000, value: 0x12 }]
            );
        }

        // Word write.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x0010_0000, PatchType::Word, 0x12345678,
            );
            apply_patches(
                &[&c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[MemoryOp::Write32 { addr: 0x0010_0000, value: 0x12345678 }]
            );
        }

        // Big-endian short.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x0010_0000, PatchType::ShortBe, 0x1234,
            );
            apply_patches(
                &[&c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[MemoryOp::Write16 { addr: 0x0010_0000, value: 0x3412 }]
            );
        }

        // Big-endian double.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x0010_0000, PatchType::DoubleBe, 0xabcdef0123456789,
            );
            apply_patches(
                &[&c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[MemoryOp::Write64 {
                    addr: 0x0010_0000,
                    value: 0x8967452301efcdab,
                }]
            );
        }

        // IOP routing: writes go to the iop bus, not the ee bus.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Iop,
                0x0010_0000, PatchType::Byte, 0x42,
            );
            apply_patches(
                &[&c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert!(ee.ops().is_empty());
            assert_eq!(
                iop.ops(),
                &[MemoryOp::Write8 { addr: 0x0010_0000, value: 0x42 }]
            );
        }

        // Extended 8-bit write: read, then write.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x0001_0000, PatchType::Extended, 0x12,
            );
            apply_patches(
                &[&c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[
                    MemoryOp::Read8 { addr: 0x0010_0000, value: 0 },
                    MemoryOp::Write8 { addr: 0x0010_0000, value: 0x12 },
                ]
            );
        }

        // Extended 8-bit increment, applied twice: state is visible to the
        // second command because the mock records writes.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x3001_0000, PatchType::Extended, 0x12,
            );
            apply_patches(
                &[&c, &c], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[
                    MemoryOp::Read8 { addr: 0x0010_0000, value: 0 },
                    MemoryOp::Write8 { addr: 0x0010_0000, value: 0x12 },
                    MemoryOp::Read8 { addr: 0x0010_0000, value: 0x12 },
                    MemoryOp::Write8 { addr: 0x0010_0000, value: 0x24 },
                ]
            );
        }

        // Mode filtering: patches in the wrong place are skipped.
        {
            let mut ee = MockMemory::new();
            let mut iop = MockMemory::new();
            let c1 = build_patch(
                PatchPlace::OnceOnLoad, PatchCpu::Ee,
                0x0010_0000, PatchType::Byte, 0x11,
            );
            let c2 = build_patch(
                PatchPlace::Continuously, PatchCpu::Ee,
                0x0020_0000, PatchType::Byte, 0x22,
            );
            apply_patches(
                &[&c1, &c2], PatchPlace::OnceOnLoad, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[MemoryOp::Write8 { addr: 0x0010_0000, value: 0x11 }]
            );

            // Continuous mode applies both continuous patches.
            apply_patches(
                &[&c1, &c2], PatchPlace::Continuously, &mut ee, &mut iop,
            );
            assert_eq!(
                ee.ops(),
                &[
                    MemoryOp::Write8 { addr: 0x0010_0000, value: 0x11 },
                    MemoryOp::Write8 { addr: 0x0020_0000, value: 0x22 },
                ]
            );
        }
    }
}
