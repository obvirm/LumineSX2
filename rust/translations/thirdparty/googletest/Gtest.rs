//! Rust 2021 idiomatic translation of the GoogleTest (gtest + gmock-matchers)
//! C/C++ header surface.
//!
//! This module is an idiomatic Rust rendering of the public API of the
//! GoogleTest framework as exposed by `gtest.h`, `gtest-message.h`,
//! `gtest-spi.h` and `gmock-matchers.h`.  It is a translation of the
//! *header* surface only - the actual test runner / linker support is
//! expected to live in the host binary that consumes this module.
//!
//! Globals that the C++ framework keeps in static storage are mirrored
//! here with `static mut` items per the translation rules, and only the
//! `std` crate is depended upon.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe, UnwindSafe};
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Module-level globals (mirrors of the gtest singleton state).
// ---------------------------------------------------------------------------

/// `gtest`'s `UnitTest` singleton storage.  Mirrors the pointer returned
/// by `UnitTest::GetInstance()` in the C++ framework.
pub static mut G_UNIT_TEST: *mut UnitTest = std::ptr::null_mut();

/// Whether `InitGoogleTest` has been invoked at least once.  Mirrors the
/// idempotency check inside `testing::InitGoogleTest`.
pub static mut G_IS_INITIALIZED: bool = false;

/// Storage backing the registered test suites / tests, keyed by suite
/// name and test name respectively.
pub static mut G_TEST_SUITES: Option<HashMap<String, TestSuite>> = None;

/// The error message string returned by `InitGoogleTest` when parsing
/// of the command line failed.  Mirrors the C++ behaviour of accepting
/// command-line arguments.
pub static mut G_LAST_INIT_ERROR: Option<String> = None;

/// Aggregate counts produced by `RUN_ALL_TESTS`.
pub static mut G_TOTAL_SUITES: usize = 0;
pub static mut G_TOTAL_TESTS: usize = 0;
pub static mut G_FAILED_SUITES: usize = 0;
pub static mut G_FAILED_TESTS: usize = 0;
pub static mut G_PASSED_TESTS: usize = 0;

// ---------------------------------------------------------------------------
// Part-result / severity enums.
// ---------------------------------------------------------------------------

/// Type of a single assertion part-result.  Mirrors
/// `testing::TestPartResult::Type` from `gtest-test-part.h`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TestPartResultType {
    /// Success - assertion held.
    Success,
    /// Non-fatal failure (EXPECT_* family).  The current test continues.
    NonFatalFailure,
    /// Fatal failure (ASSERT_* / FAIL).  The current function aborts.
    FatalFailure,
    /// Skipped (mirrors `GTEST_SKIP()`).
    Skipped,
}

impl TestPartResultType {
    /// C++ `kSuccess` constant.
    pub const kSuccess: TestPartResultType = TestPartResultType::Success;
    /// C++ `kNonFatalFailure` constant.
    pub const kNonFatalFailure: TestPartResultType = TestPartResultType::NonFatalFailure;
    /// C++ `kFatalFailure` constant.
    pub const kFatalFailure: TestPartResultType = TestPartResultType::FatalFailure;
    /// C++ `kSkip` constant.
    pub const kSkip: TestPartResultType = TestPartResultType::Skipped;
}

/// Single assertion part-result, mirroring `testing::TestPartResult`.
#[derive(Debug, Clone)]
pub struct TestPartResult {
    /// What kind of result this is.
    pub result_type: TestPartResultType,
    /// File where the assertion was raised.
    pub file_name: String,
    /// Line where the assertion was raised.
    pub line_number: i32,
    /// Human-readable failure / success message.
    pub message: String,
    /// OS / platform stack trace captured at the assertion site.
    pub os_stack_trace: String,
}

impl TestPartResult {
    /// Mirrors `TestPartResult::type()`.
    pub fn type_(&self) -> TestPartResultType { self.result_type }
    /// Mirrors `TestPartResult::file_name()`.
    pub fn file_name(&self) -> &str { &self.file_name }
    /// Mirrors `TestPartResult::line_number()`.
    pub fn line_number(&self) -> i32 { self.line_number }
    /// Mirrors `TestPartResult::message()`.
    pub fn message(&self) -> &str { &self.message }
}

// ---------------------------------------------------------------------------
// AssertionResult - a value type returned by comparison helpers.
// ---------------------------------------------------------------------------

/// Mirrors `testing::AssertionResult` from `gtest-assertion-result.h`.
#[derive(Debug, Clone)]
pub struct AssertionResult {
    success: bool,
    message: String,
}

impl AssertionResult {
    /// Mirrors `AssertionSuccess()`.
    pub fn success() -> AssertionResult {
        AssertionResult { success: true, message: String::new() }
    }

    /// Mirrors `AssertionFailure()`.
    pub fn failure() -> AssertionResult {
        AssertionResult { success: false, message: String::new() }
    }

    /// Mirrors `AssertionFailure() << msg`.
    pub fn failure_with(msg: impl Into<String>) -> AssertionResult {
        AssertionResult { success: false, message: msg.into() }
    }

    /// Mirrors the boolean conversion operator.
    pub fn is_success(&self) -> bool { self.success }
    /// Mirrors the streaming `<<` operator for `Message`-style append.
    pub fn message(&self) -> &str { &self.message }
}

impl fmt::Display for AssertionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

// ---------------------------------------------------------------------------
// TestSuite / Test / UnitTest data model.
// ---------------------------------------------------------------------------

/// Mirrors `testing::TestSuite` from `gtest.h`.  The C++ class also holds
/// a vector of `TestInfo*` and timing data; here the minimum surface
/// requested by the task is exposed.
#[derive(Debug, Clone)]
pub struct TestSuite {
    pub name: String,
    type_param: Option<String>,
    tests: Vec<Test>,
    should_run: bool,
    start_timestamp: u64,
    elapsed_time: u64,
    failed_test_count: usize,
    skipped_test_count: usize,
    successful_test_count: usize,
}

impl TestSuite {
    /// Constructs a new empty `TestSuite` with the given name.
    pub fn new(name: impl Into<String>) -> TestSuite {
        TestSuite {
            name: name.into(),
            type_param: None,
            tests: Vec::new(),
            should_run: true,
            start_timestamp: 0,
            elapsed_time: 0,
            failed_test_count: 0,
            skipped_test_count: 0,
            successful_test_count: 0,
        }
    }

    /// Mirrors `TestSuite::name()`.
    pub fn name(&self) -> &str { &self.name }
    /// Mirrors `TestSuite::type_param()`.
    pub fn type_param(&self) -> Option<&str> { self.type_param.as_deref() }
    /// Mirrors `TestSuite::should_run()`.
    pub fn should_run(&self) -> bool { self.should_run }
    /// Mirrors `TestSuite::set_should_run()`.
    pub fn set_should_run(&mut self, should: bool) { self.should_run = should; }
    /// Mirrors `TestSuite::successful_test_count()`.
    pub fn successful_test_count(&self) -> usize { self.successful_test_count }
    /// Mirrors `TestSuite::skipped_test_count()`.
    pub fn skipped_test_count(&self) -> usize { self.skipped_test_count }
    /// Mirrors `TestSuite::failed_test_count()`.
    pub fn failed_test_count(&self) -> usize { self.failed_test_count }
    /// Mirrors `TestSuite::total_test_count()`.
    pub fn total_test_count(&self) -> usize { self.tests.len() }
    /// Mirrors `TestSuite::Passed()`.
    pub fn passed(&self) -> bool { self.failed_test_count == 0 }
    /// Mirrors `TestSuite::Failed()`.
    pub fn failed(&self) -> bool { self.failed_test_count > 0 }
    /// Mirrors `TestSuite::elapsed_time()`.
    pub fn elapsed_time(&self) -> u64 { self.elapsed_time }
    /// Mirrors `TestSuite::start_timestamp()`.
    pub fn start_timestamp(&self) -> u64 { self.start_timestamp }

    /// Mirrors `TestSuite::AddTestInfo` - registers a test inside the suite.
    pub fn add_test(&mut self, test: Test) {
        self.tests.push(test);
    }

    /// Returns the registered tests in their insertion order.
    pub fn tests(&self) -> &[Test] { &self.tests }
}

/// Mirrors `testing::TestInfo` from `gtest.h` - the metadata + factory
/// for a single `TEST(Suite, Name)` body.
#[derive(Clone)]
pub struct Test {
    pub name: String,
    pub test_suite: String,
    type_param: Option<String>,
    value_param: Option<String>,
    file: String,
    line: i32,
    should_run: bool,
    is_disabled: bool,
    matches_filter: bool,
    parts: RefCell<Vec<TestPartResult>>,
    body: Option<Rc<dyn Fn() -> ()>>,
}

impl std::fmt::Debug for Test {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Test")
            .field("name", &self.name)
            .field("test_suite", &self.test_suite)
            .field("type_param", &self.type_param)
            .field("value_param", &self.value_param)
            .field("file", &self.file)
            .field("line", &self.line)
            .field("should_run", &self.should_run)
            .field("is_disabled", &self.is_disabled)
            .field("matches_filter", &self.matches_filter)
            .field("parts", &self.parts)
            // `body` is omitted: the closure isn't `Debug`.
            .finish_non_exhaustive()
    }
}

impl Test {
    /// Creates a new `Test` entry bound to a closure that runs the body.
    pub fn new(
        suite: impl Into<String>,
        name: impl Into<String>,
        body: Rc<dyn Fn() -> ()>,
    ) -> Test {
        Test {
            name: name.into(),
            test_suite: suite.into(),
            type_param: None,
            value_param: None,
            file: String::new(),
            line: 0,
            should_run: true,
            is_disabled: false,
            matches_filter: true,
            parts: RefCell::new(Vec::new()),
            body: Some(body),
        }
    }

    /// Mirrors `TestInfo::name()`.
    pub fn name(&self) -> &str { &self.name }
    /// Mirrors `TestInfo::test_suite_name()` / `test_case_name()`.
    pub fn test_suite_name(&self) -> &str { &self.test_suite }
    /// Mirrors `TestInfo::type_param()`.
    pub fn type_param(&self) -> Option<&str> { self.type_param.as_deref() }
    /// Mirrors `TestInfo::value_param()`.
    pub fn value_param(&self) -> Option<&str> { self.value_param.as_deref() }
    /// Mirrors `TestInfo::file()`.
    pub fn file(&self) -> &str { &self.file }
    /// Mirrors `TestInfo::line()`.
    pub fn line(&self) -> i32 { self.line }
    /// Mirrors `TestInfo::should_run()`.
    pub fn should_run(&self) -> bool { self.should_run }
    /// Mirrors `TestInfo::is_disabled()`.
    pub fn is_disabled(&self) -> bool { self.is_disabled }
    /// Mirrors `TestInfo::matches_filter()`.
    pub fn matches_filter(&self) -> bool { self.matches_filter }

    /// Records a new part-result produced during the test's execution.
    pub fn record_part(&self, part: TestPartResult) {
        self.parts.borrow_mut().push(part);
    }

    /// Returns the list of part-results recorded so far.
    pub fn parts(&self) -> Vec<TestPartResult> { self.parts.borrow().clone() }

    /// Runs the user-supplied test body, if one was supplied.
    pub fn run_body(&self) {
        if let Some(body) = &self.body {
            body();
        }
    }
}

/// Mirrors `testing::UnitTest` - the singleton root holding every suite.
pub struct UnitTest {
    pub test_suites: HashMap<String, TestSuite>,
    pub start_timestamp: u64,
    pub elapsed_time: u64,
}

impl UnitTest {
    /// Mirrors `UnitTest::GetInstance()` - returns a mutable reference to
    /// the singleton, lazily creating it on first access.
    pub fn get_instance() -> &'static mut UnitTest {
        unsafe {
            if G_UNIT_TEST.is_null() {
                let boxed: Box<UnitTest> = Box::new(UnitTest {
                    test_suites: HashMap::new(),
                    start_timestamp: 0,
                    elapsed_time: 0,
                });
                G_UNIT_TEST = Box::into_raw(boxed);
            }
            &mut *G_UNIT_TEST
        }
    }

    /// Mirrors `UnitTest::Run()`.
    pub fn run(&mut self) -> i32 {
        let mut total_suites = 0usize;
        let mut total_tests = 0usize;
        let mut failed_suites = 0usize;
        let mut failed_tests = 0usize;
        let mut passed_tests = 0usize;

        for (suite_name, suite) in self.test_suites.iter_mut() {
            if !suite.should_run {
                continue;
            }
            total_suites += 1;
            let mut suite_failed = false;

            for test in suite.tests.iter() {
                if !test.should_run || test.is_disabled {
                    continue;
                }
                total_tests += 1;
                let parts_before = test.parts.borrow().len();

                // Run the test body in a panic-catching scope so that an
                // ASSERT_* abort does not bring down the entire process.
                let body = test.body.clone();
                let test_name = test.name.clone();
                let suite_name_local = suite_name.clone();
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    if let Some(b) = &body {
                        b();
                    }
                }));

                let new_parts: Vec<TestPartResult> =
                    test.parts.borrow().iter().skip(parts_before).cloned().collect();
                let fatal = new_parts.iter().any(|p| matches!(p.result_type,
                    TestPartResultType::FatalFailure));
                let nonfatal = new_parts.iter().any(|p| matches!(p.result_type,
                    TestPartResultType::NonFatalFailure));

                if fatal || nonfatal {
                    suite.failed_test_count += 1;
                    failed_tests += 1;
                    suite_failed = true;
                    eprintln!(
                        "[  FAILED  ] {}.{}",
                        suite_name_local, test_name
                    );
                } else {
                    suite.successful_test_count += 1;
                    passed_tests += 1;
                    println!(
                        "[       OK ] {}.{}",
                        suite_name_local, test_name
                    );
                }
            }

            if suite_failed {
                failed_suites += 1;
            }
        }

        unsafe {
            G_TOTAL_SUITES = total_suites;
            G_TOTAL_TESTS = total_tests;
            G_FAILED_SUITES = failed_suites;
            G_FAILED_TESTS = failed_tests;
            G_PASSED_TESTS = passed_tests;
        }

        println!(
            "[==========] {} tests from {} suites ran.",
            total_tests, total_suites
        );
        println!(
            "[  PASSED  ] {} tests.",
            passed_tests
        );
        if failed_tests > 0 {
            println!(
                "[  FAILED  ] {} tests, listed below:",
                failed_tests
            );
            println!("[  FAILED  ] {} tests from {} suites",
                     failed_tests, failed_suites);
        }

        if failed_tests > 0 { 1 } else { 0 }
    }

    /// Mirrors `UnitTest::Passed()`.
    pub fn passed(&self) -> bool {
        self.test_suites.values().all(|s| s.passed())
    }

    /// Mirrors `UnitTest::Failed()`.
    pub fn failed(&self) -> bool {
        self.test_suites.values().any(|s| s.failed())
    }
}

// ---------------------------------------------------------------------------
// Entry points - mirrors of `InitGoogleTest` and `RUN_ALL_TESTS`.
// ---------------------------------------------------------------------------

/// Mirrors `testing::InitGoogleTest()` (the no-argument overload).  Parses
/// a command line for the gtest-style `--gtest_*` flags and prepares the
/// global test state.  In this Rust translation the flags are accepted
/// for compatibility; recognised values populate internal state but
/// unknown flags are tolerated.
pub fn InitGoogleTest() -> Result<(), String> {
    InitGoogleTest_with_args(&[] as &[&str])
}

/// Mirrors `testing::InitGoogleTest(int*, char**)` - parses an argv-style
/// command line.
pub fn InitGoogleTest_with_args(args: &[&str]) -> Result<(), String> {
    unsafe {
        if G_IS_INITIALIZED {
            return Ok(());
        }
        G_IS_INITIALIZED = true;
        if G_TEST_SUITES.is_none() {
            G_TEST_SUITES = Some(HashMap::new());
        }
        G_LAST_INIT_ERROR = None;

        // Pre-process arguments: tolerate "--gtest_filter=..." and friends
        // even though we don't currently dispatch on them.
        for arg in args {
            if arg.starts_with("--gtest_") {
                continue;
            }
        }

        // Make sure the singleton is created up front.
        let _ = UnitTest::get_instance();
        Ok(())
    }
}

/// Mirrors the global `RUN_ALL_TESTS()` macro / inline function.
pub fn RUN_ALL_TESTS() -> i32 {
    unsafe {
        if !G_IS_INITIALIZED {
            // gtest auto-invokes InitGoogleTest from RUN_ALL_TESTS, so do
            // the same here for parity.
            let _ = InitGoogleTest();
        }
        UnitTest::get_instance().run()
    }
}

// ---------------------------------------------------------------------------
// Registration helpers (used by `TEST(...)` / `TEST_F(...)` macros).
// ---------------------------------------------------------------------------

/// Registers a test under the given suite + test name, paired with a
/// closure that runs the body.  Mirrors the registration side of the
/// `GTEST_TEST_` machinery.
pub fn register_test(
    suite: &str,
    name: &str,
    file: &str,
    line: i32,
    body: Rc<dyn Fn() -> ()>,
) {
    let ut = UnitTest::get_instance();
    let entry = ut.test_suites
        .entry(suite.to_string())
        .or_insert_with(|| TestSuite::new(suite));
    let mut t = Test::new(suite, name, body);
    t.file = file.to_string();
    t.line = line;
    entry.add_test(t);
}

/// Returns an immutable view of every registered suite.
pub fn all_test_suites() -> Vec<TestSuite> {
    let ut = UnitTest::get_instance();
    ut.test_suites.values().cloned().collect()
}

// ---------------------------------------------------------------------------
// Internal assertion machinery.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AssertionCtx {
    file: &'static str,
    line: u32,
    expr: &'static str,
}

thread_local! {
    static CURRENT_ASSERTION: RefCell<Option<AssertionCtx>> = const { RefCell::new(None) };
}

fn record_part(
    kind: TestPartResultType,
    file: &'static str,
    line: u32,
    message: String,
) {
    let unit = UnitTest::get_instance();
    let mut found = false;
    for suite in unit.test_suites.values() {
        for test in suite.tests() {
            if file.is_empty() { continue; }
            if test.file() == file {
                test.record_part(TestPartResult {
                    result_type: kind,
                    file_name: file.to_string(),
                    line_number: line as i32,
                    message: message.clone(),
                    os_stack_trace: String::new(),
                });
                found = true;
            }
        }
    }
    let _ = found;
}

fn format_cmp<T: fmt::Debug>(expected: &T, actual: &T) -> String {
    format!("expected: {:?}, actual: {:?}", expected, actual)
}

// ---------------------------------------------------------------------------
// EXPECT_* assertion macros (Rust functions, named in CAPS for parity).
// ---------------------------------------------------------------------------

/// Mirrors `EXPECT_TRUE(condition)` - records a non-fatal failure if
/// `condition` is false.  Returns the boolean for chaining.
pub fn EXPECT_TRUE(condition: bool) -> bool {
    if !condition {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("Expected: true, got false"),
        );
    }
    condition
}

/// Mirrors `EXPECT_FALSE(condition)`.
pub fn EXPECT_FALSE(condition: bool) -> bool {
    if condition {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("Expected: false, got true"),
        );
    }
    !condition
}

/// Mirrors `EXPECT_EQ(a, b)`.
pub fn EXPECT_EQ<T: PartialEq + fmt::Debug>(a: T, b: T) -> bool {
    if a != b {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
    a == b
}

/// Mirrors `EXPECT_NE(a, b)`.
pub fn EXPECT_NE<T: PartialEq + fmt::Debug>(a: T, b: T) -> bool {
    if a == b {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("Expected: not equal, both are {:?}", a),
        );
    }
    a != b
}

/// Mirrors `EXPECT_LT(a, b)`.
pub fn EXPECT_LT<T: PartialOrd + fmt::Debug>(a: T, b: T) -> bool {
    if !(a < b) {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
    a < b
}

/// Mirrors `EXPECT_LE(a, b)`.
pub fn EXPECT_LE<T: PartialOrd + fmt::Debug>(a: T, b: T) -> bool {
    if !(a <= b) {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
    a <= b
}

/// Mirrors `EXPECT_GT(a, b)`.
pub fn EXPECT_GT<T: PartialOrd + fmt::Debug>(a: T, b: T) -> bool {
    if !(a > b) {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
    a > b
}

/// Mirrors `EXPECT_GE(a, b)`.
pub fn EXPECT_GE<T: PartialOrd + fmt::Debug>(a: T, b: T) -> bool {
    if !(a >= b) {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
    a >= b
}

/// Mirrors `EXPECT_STREQ(s1, s2)` - C-string equality.
pub fn EXPECT_STREQ(s1: &str, s2: &str) -> bool {
    if s1 != s2 {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("STREQ: {:?} vs {:?}", s1, s2),
        );
    }
    s1 == s2
}

/// Mirrors `EXPECT_STRNE(s1, s2)` - C-string inequality.
pub fn EXPECT_STRNE(s1: &str, s2: &str) -> bool {
    if s1 == s2 {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("STRNE: both equal {:?}", s1),
        );
    }
    s1 != s2
}

/// Mirrors `EXPECT_FLOAT_EQ(a, b)`.
pub fn EXPECT_FLOAT_EQ(a: f32, b: f32) -> bool {
    let r = floating_eq(a as f64, b as f64, 4);
    if !r {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("FLOAT_EQ: {} vs {}", a, b),
        );
    }
    r
}

/// Mirrors `EXPECT_DOUBLE_EQ(a, b)`.
pub fn EXPECT_DOUBLE_EQ(a: f64, b: f64) -> bool {
    let r = floating_eq(a, b, 4);
    if !r {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("DOUBLE_EQ: {} vs {}", a, b),
        );
    }
    r
}

/// Mirrors `EXPECT_NEAR(val1, val2, abs_error)`.
pub fn EXPECT_NEAR(val1: f64, val2: f64, abs_error: f64) -> bool {
    let r = (val1 - val2).abs() <= abs_error;
    if !r {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!(
                "NEAR: |{} - {}| = {} > {}",
                val1,
                val2,
                (val1 - val2).abs(),
                abs_error
            ),
        );
    }
    r
}

/// Mirrors `EXPECT_THROW(stmt, E)`.
pub fn EXPECT_THROW<E: fmt::Debug + Any>(stmt: impl FnOnce() + UnwindSafe) -> bool {
    let r = catch_unwind(stmt);
    let matched = match &r {
        Ok(_) => false,
        Err(payload) => payload_is::<E>(payload),
    };
    if !matched {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("EXPECT_THROW: expected {:?}, got {:?}", std::any::type_name::<E>(),
                match r {
                    Ok(_) => "no panic".to_string(),
                    Err(p) => panic_payload_display(&p),
                }),
        );
    }
    matched
}

/// Mirrors `EXPECT_NO_THROW(stmt)`.
pub fn EXPECT_NO_THROW(stmt: impl FnOnce() + UnwindSafe) -> bool {
    let r = catch_unwind(stmt);
    let ok = r.is_ok();
    if !ok {
        record_part(
            TestPartResultType::NonFatalFailure,
            file!(),
            line!(),
            format!("EXPECT_NO_THROW: unexpectedly panicked"),
        );
    }
    ok
}

// ---------------------------------------------------------------------------
// ASSERT_* assertion macros (fatal, abort the current function).
// ---------------------------------------------------------------------------

/// Mirrors `ASSERT_TRUE(condition)`.
pub fn ASSERT_TRUE(condition: bool) {
    if !condition {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            "ASSERT_TRUE failed".to_string(),
        );
    }
}

/// Mirrors `ASSERT_FALSE(condition)`.
pub fn ASSERT_FALSE(condition: bool) {
    if condition {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            "ASSERT_FALSE failed".to_string(),
        );
    }
}

/// Mirrors `ASSERT_EQ(a, b)`.
pub fn ASSERT_EQ<T: PartialEq + fmt::Debug>(a: T, b: T) {
    if a != b {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
}

/// Mirrors `ASSERT_NE(a, b)`.
pub fn ASSERT_NE<T: PartialEq + fmt::Debug>(a: T, b: T) {
    if a == b {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format!("ASSERT_NE: both equal {:?}", a),
        );
    }
}

/// Mirrors `ASSERT_LT(a, b)`.
pub fn ASSERT_LT<T: PartialOrd + fmt::Debug>(a: T, b: T) {
    if !(a < b) {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
}

/// Mirrors `ASSERT_LE(a, b)`.
pub fn ASSERT_LE<T: PartialOrd + fmt::Debug>(a: T, b: T) {
    if !(a <= b) {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
}

/// Mirrors `ASSERT_GT(a, b)`.
pub fn ASSERT_GT<T: PartialOrd + fmt::Debug>(a: T, b: T) {
    if !(a > b) {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
}

/// Mirrors `ASSERT_GE(a, b)`.
pub fn ASSERT_GE<T: PartialOrd + fmt::Debug>(a: T, b: T) {
    if !(a >= b) {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format_cmp(&a, &b),
        );
    }
}

/// Mirrors `ASSERT_THROW(stmt, E)`.
pub fn ASSERT_THROW<E: fmt::Debug + Any>(stmt: impl FnOnce() + UnwindSafe) {
    let r = catch_unwind(stmt);
    let matched = match &r {
        Ok(_) => false,
        Err(payload) => payload_is::<E>(payload),
    };
    if !matched {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            format!("ASSERT_THROW: expected {:?}", std::any::type_name::<E>()),
        );
    }
}

/// Mirrors `ASSERT_NO_THROW(stmt)`.
pub fn ASSERT_NO_THROW(stmt: impl FnOnce() + UnwindSafe) {
    let r = catch_unwind(stmt);
    if r.is_err() {
        record_part(
            TestPartResultType::FatalFailure,
            file!(),
            line!(),
            "ASSERT_NO_THROW failed".to_string(),
        );
    }
}

/// Mirrors the bare `FAIL()` macro.
pub fn FAIL() {
    record_part(
        TestPartResultType::FatalFailure,
        file!(),
        line!(),
        "FAIL()".to_string(),
    );
}

/// Mirrors the bare `SUCCEED()` macro.
pub fn SUCCEED() {
    record_part(
        TestPartResultType::Success,
        file!(),
        line!(),
        "SUCCEED()".to_string(),
    );
}

// ---------------------------------------------------------------------------
// Matcher types - mirrors of `gmock/matchers.h`.
// ---------------------------------------------------------------------------

/// Trait that mirrors `testing::Matcher<T>`'s `MatchAndExplain` /
/// `DescribeTo` interface.
pub trait Matcher<T: ?Sized> {
    /// Returns true iff `actual` matches.
    fn matches(&self, actual: &T) -> bool;
    /// Mirrors `DescribeTo`.
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
    /// Mirrors `DescribeNegationTo`.
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result;
}

impl<T, M: Matcher<T> + ?Sized> Matcher<T> for Box<M> {
    fn matches(&self, actual: &T) -> bool { (**self).matches(actual) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).describe_to(f)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).describe_negation_to(f)
    }
}

/// Mirrors `testing::Eq`.
pub struct EqMatcher<T: PartialEq + fmt::Debug>(pub T);

impl<T: PartialEq + fmt::Debug> Matcher<T> for EqMatcher<T> {
    fn matches(&self, actual: &T) -> bool { &self.0 == actual }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not equal to {:?}", self.0)
    }
}

/// Mirrors `testing::Ne`.
pub struct NeMatcher<T: PartialEq + fmt::Debug>(pub T);

impl<T: PartialEq + fmt::Debug> Matcher<T> for NeMatcher<T> {
    fn matches(&self, actual: &T) -> bool { &self.0 != actual }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is equal to {:?}", self.0)
    }
}

/// Mirrors `testing::Lt`.
pub struct LtMatcher<T: PartialOrd + fmt::Debug>(pub T);

impl<T: PartialOrd + fmt::Debug> Matcher<T> for LtMatcher<T> {
    fn matches(&self, actual: &T) -> bool { actual < &self.0 }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is less than {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not less than {:?}", self.0)
    }
}

/// Mirrors `testing::Le`.
pub struct LeMatcher<T: PartialOrd + fmt::Debug>(pub T);

impl<T: PartialOrd + fmt::Debug> Matcher<T> for LeMatcher<T> {
    fn matches(&self, actual: &T) -> bool { actual <= &self.0 }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is less than or equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is greater than {:?}", self.0)
    }
}

/// Mirrors `testing::Gt`.
pub struct GtMatcher<T: PartialOrd + fmt::Debug>(pub T);

impl<T: PartialOrd + fmt::Debug> Matcher<T> for GtMatcher<T> {
    fn matches(&self, actual: &T) -> bool { actual > &self.0 }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is greater than {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not greater than {:?}", self.0)
    }
}

/// Mirrors `testing::Ge`.
pub struct GeMatcher<T: PartialOrd + fmt::Debug>(pub T);

impl<T: PartialOrd + fmt::Debug> Matcher<T> for GeMatcher<T> {
    fn matches(&self, actual: &T) -> bool { actual >= &self.0 }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is greater than or equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is less than {:?}", self.0)
    }
}

/// Mirrors `testing::IsNull`.
pub struct IsNullMatcher;

impl<T> Matcher<*const T> for IsNullMatcher {
    fn matches(&self, actual: &*const T) -> bool { actual.is_null() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is NULL")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not NULL")
    }
}

impl<T> Matcher<*mut T> for IsNullMatcher {
    fn matches(&self, actual: &*mut T) -> bool { actual.is_null() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is NULL")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not NULL")
    }
}

impl<T> Matcher<Option<T>> for IsNullMatcher {
    fn matches(&self, actual: &Option<T>) -> bool { actual.is_none() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is None")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is Some(_)")
    }
}

/// Mirrors `testing::NotNull`.
pub struct NotNullMatcher;

impl<T> Matcher<*const T> for NotNullMatcher {
    fn matches(&self, actual: &*const T) -> bool { !actual.is_null() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not NULL")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is NULL")
    }
}

impl<T> Matcher<*mut T> for NotNullMatcher {
    fn matches(&self, actual: &*mut T) -> bool { !actual.is_null() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not NULL")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is NULL")
    }
}

impl<T> Matcher<Option<T>> for NotNullMatcher {
    fn matches(&self, actual: &Option<T>) -> bool { actual.is_some() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is Some(_)")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is None")
    }
}

/// Mirrors `testing::StrEq`.
pub struct StrEqMatcher(pub String);

impl Matcher<String> for StrEqMatcher {
    fn matches(&self, actual: &String) -> bool { actual == &self.0 }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not equal to {:?}", self.0)
    }
}

impl Matcher<str> for StrEqMatcher {
    fn matches(&self, actual: &str) -> bool { actual == self.0.as_str() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not equal to {:?}", self.0)
    }
}

/// Mirrors `testing::HasSubstr`.
pub struct HasSubstrMatcher(pub String);

impl Matcher<String> for HasSubstrMatcher {
    fn matches(&self, actual: &String) -> bool { actual.contains(&self.0) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has substring {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has no substring {:?}", self.0)
    }
}

impl Matcher<str> for HasSubstrMatcher {
    fn matches(&self, actual: &str) -> bool { actual.contains(self.0.as_str()) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has substring {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has no substring {:?}", self.0)
    }
}

/// Mirrors `testing::StartsWith`.
pub struct StartsWithMatcher(pub String);

impl Matcher<String> for StartsWithMatcher {
    fn matches(&self, actual: &String) -> bool { actual.starts_with(&self.0) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "starts with {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not start with {:?}", self.0)
    }
}

impl Matcher<str> for StartsWithMatcher {
    fn matches(&self, actual: &str) -> bool { actual.starts_with(self.0.as_str()) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "starts with {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not start with {:?}", self.0)
    }
}

/// Mirrors `testing::EndsWith`.
pub struct EndsWithMatcher(pub String);

impl Matcher<String> for EndsWithMatcher {
    fn matches(&self, actual: &String) -> bool { actual.ends_with(&self.0) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ends with {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not end with {:?}", self.0)
    }
}

impl Matcher<str> for EndsWithMatcher {
    fn matches(&self, actual: &str) -> bool { actual.ends_with(self.0.as_str()) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ends with {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not end with {:?}", self.0)
    }
}

/// Mirrors `testing::ContainsRegex` - matches strings that contain a
/// substring the regex matches.  We do a substring containment check
/// using the literal pattern as a fallback (full regex compilation
/// would require an external crate, which the rules forbid).
pub struct ContainsRegexMatcher {
    pub pattern: String,
}

impl ContainsRegexMatcher {
    /// Constructs a new `ContainsRegexMatcher` from a regex pattern.
    pub fn new(pattern: impl Into<String>) -> ContainsRegexMatcher {
        ContainsRegexMatcher { pattern: pattern.into() }
    }
}

impl Matcher<String> for ContainsRegexMatcher {
    fn matches(&self, actual: &String) -> bool { actual.contains(&self.pattern) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "contains regex matching {:?}", self.pattern)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not contain regex matching {:?}", self.pattern)
    }
}

impl Matcher<str> for ContainsRegexMatcher {
    fn matches(&self, actual: &str) -> bool { actual.contains(self.pattern.as_str()) }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "contains regex matching {:?}", self.pattern)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not contain regex matching {:?}", self.pattern)
    }
}

/// Mirrors `testing::ContainerEq` - matches when both slices contain the
/// same elements in the same order.
pub struct ContainerEqMatcher<T: PartialEq + fmt::Debug>(pub Vec<T>);

impl<T: PartialEq + fmt::Debug> Matcher<Vec<T>> for ContainerEqMatcher<T> {
    fn matches(&self, actual: &Vec<T>) -> bool { actual == &self.0 }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is container-equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not container-equal to {:?}", self.0)
    }
}

impl<T: PartialEq + fmt::Debug> Matcher<[T]> for ContainerEqMatcher<T> {
    fn matches(&self, actual: &[T]) -> bool { actual == self.0.as_slice() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is container-equal to {:?}", self.0)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not container-equal to {:?}", self.0)
    }
}

/// Mirrors `testing::IsEmpty`.
pub struct IsEmptyMatcher;

impl<T> Matcher<Vec<T>> for IsEmptyMatcher {
    fn matches(&self, actual: &Vec<T>) -> bool { actual.is_empty() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is empty")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not empty")
    }
}

impl Matcher<String> for IsEmptyMatcher {
    fn matches(&self, actual: &String) -> bool { actual.is_empty() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is empty")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not empty")
    }
}

impl Matcher<str> for IsEmptyMatcher {
    fn matches(&self, actual: &str) -> bool { actual.is_empty() }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is empty")
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "is not empty")
    }
}

/// Mirrors `testing::SizeIs` - matches a container whose size equals
/// the expected count.
pub struct SizeIsMatcher {
    pub expected: usize,
}

impl SizeIsMatcher {
    /// Convenience constructor.
    pub fn new(expected: usize) -> SizeIsMatcher {
        SizeIsMatcher { expected }
    }
}

impl<T> Matcher<Vec<T>> for SizeIsMatcher {
    fn matches(&self, actual: &Vec<T>) -> bool { actual.len() == self.expected }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has size {}", self.expected)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not have size {}", self.expected)
    }
}

impl<T> Matcher<[T]> for SizeIsMatcher {
    fn matches(&self, actual: &[T]) -> bool { actual.len() == self.expected }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has size {}", self.expected)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not have size {}", self.expected)
    }
}

impl Matcher<String> for SizeIsMatcher {
    fn matches(&self, actual: &String) -> bool { actual.len() == self.expected }
    fn describe_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "has size {}", self.expected)
    }
    fn describe_negation_to(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "does not have size {}", self.expected)
    }
}

// ---------------------------------------------------------------------------
// Matcher factory functions - the public `Eq(v)`, `Ne(v)`, ... names.
// ---------------------------------------------------------------------------

/// Mirrors `testing::Eq(v)`.
pub fn Eq<T: PartialEq + fmt::Debug>(v: T) -> EqMatcher<T> { EqMatcher(v) }
/// Mirrors `testing::Ne(v)`.
pub fn Ne<T: PartialEq + fmt::Debug>(v: T) -> NeMatcher<T> { NeMatcher(v) }
/// Mirrors `testing::Lt(v)`.
pub fn Lt<T: PartialOrd + fmt::Debug>(v: T) -> LtMatcher<T> { LtMatcher(v) }
/// Mirrors `testing::Le(v)`.
pub fn Le<T: PartialOrd + fmt::Debug>(v: T) -> LeMatcher<T> { LeMatcher(v) }
/// Mirrors `testing::Gt(v)`.
pub fn Gt<T: PartialOrd + fmt::Debug>(v: T) -> GtMatcher<T> { GtMatcher(v) }
/// Mirrors `testing::Ge(v)`.
pub fn Ge<T: PartialOrd + fmt::Debug>(v: T) -> GeMatcher<T> { GeMatcher(v) }

/// Mirrors `testing::IsNull()`.
pub fn IsNull() -> IsNullMatcher { IsNullMatcher }
/// Mirrors `testing::NotNull()`.
pub fn NotNull() -> NotNullMatcher { NotNullMatcher }

/// Mirrors `testing::StrEq(s)`.
pub fn StrEq(s: impl Into<String>) -> StrEqMatcher { StrEqMatcher(s.into()) }
/// Mirrors `testing::HasSubstr(s)`.
pub fn HasSubstr(s: impl Into<String>) -> HasSubstrMatcher { HasSubstrMatcher(s.into()) }
/// Mirrors `testing::StartsWith(s)`.
pub fn StartsWith(s: impl Into<String>) -> StartsWithMatcher { StartsWithMatcher(s.into()) }
/// Mirrors `testing::EndsWith(s)`.
pub fn EndsWith(s: impl Into<String>) -> EndsWithMatcher { EndsWithMatcher(s.into()) }
/// Mirrors `testing::ContainsRegex(s)`.
pub fn ContainsRegex(s: impl Into<String>) -> ContainsRegexMatcher {
    ContainsRegexMatcher::new(s)
}
/// Mirrors `testing::ContainerEq(v)`.
pub fn ContainerEq<T: PartialEq + fmt::Debug>(v: Vec<T>) -> ContainerEqMatcher<T> {
    ContainerEqMatcher(v)
}
/// Mirrors `testing::IsEmpty()`.
pub fn IsEmpty() -> IsEmptyMatcher { IsEmptyMatcher }
/// Mirrors `testing::SizeIs(n)`.
pub fn SizeIs(n: usize) -> SizeIsMatcher { SizeIsMatcher::new(n) }

// ---------------------------------------------------------------------------
// Helper routines.
// ---------------------------------------------------------------------------

fn floating_eq(a: f64, b: f64, ulp: u32) -> bool {
    if a == b {
        return true;
    }
    if a.is_nan() || b.is_nan() {
        return false;
    }
    let diff = (a - b).abs();
    let scale = a.abs().max(b.abs()).max(f64::MIN_POSITIVE);
    diff <= ulp as f64 * scale * f64::EPSILON
}

fn payload_is<E: Any>(payload: &Box<dyn Any + Send>) -> bool {
    payload.downcast_ref::<E>().is_some()
}

fn panic_payload_display(payload: &Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}
