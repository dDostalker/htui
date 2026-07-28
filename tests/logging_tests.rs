use std::fs;
use std::io::Read;
use std::path::PathBuf;

use htui::logging;

fn unique_log_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "htui-logging-test-{}-{name}.log",
        std::process::id(),
    ))
}

fn read_log(path: &PathBuf) -> String {
    let mut s = String::new();
    fs::File::open(path)
        .expect("log file should exist")
        .read_to_string(&mut s)
        .expect("log file should be readable");
    s
}

fn try_init(path: &PathBuf) -> bool {
    // try_init returns Err if the global logger is already set by another
    // test in the same process. The first test to call it wins; others skip.
    logging::init_file_logger(path, None).is_ok()
}

#[test]
fn file_logger_writes_lines() {
    let path = unique_log_path("writes");
    let _ = fs::remove_file(&path);

    if !try_init(&path) {
        eprintln!("skipping: global logger already initialized");
        return;
    }

    log::info!("hello-info");
    log::warn!("hello-warn");

    let content = read_log(&path);
    assert!(content.contains("hello-info"), "content: {content}");
    assert!(content.contains("hello-warn"), "content: {content}");
    assert!(
        content.contains("INFO") || content.contains("info"),
        "content: {content}"
    );
    assert!(
        content.contains("WARN") || content.contains("warn"),
        "content: {content}"
    );
}

#[test]
fn file_logger_respects_custom_level() {
    let path = unique_log_path("level");
    let _ = fs::remove_file(&path);

    if logging::init_file_logger(&path, Some(log::LevelFilter::Error)).is_err() {
        eprintln!("skipping: global logger already initialized");
        return;
    }

    log::info!("should-not-appear");
    log::error!("should-appear");

    let content = read_log(&path);
    assert!(!content.contains("should-not-appear"), "content: {content}");
    assert!(content.contains("should-appear"), "content: {content}");
}

#[test]
fn file_logger_creates_parent_directory() {
    let dir =
        std::env::temp_dir().join(format!("htui-logging-test-{}-nested", std::process::id(),));
    let path = dir.join("subdir").join("test.log");
    let _ = fs::remove_dir_all(&dir);

    if logging::init_file_logger(&path, None).is_err() {
        eprintln!("skipping: global logger already initialized");
        return;
    }

    log::error!("nested-dir-test");
    assert!(
        path.exists(),
        "log file should be created at {}",
        path.display()
    );
}

#[test]
fn init_file_logger_rejects_double_init() {
    let path_a = unique_log_path("double-a");
    let path_b = unique_log_path("double-b");
    let _ = fs::remove_file(&path_a);
    let _ = fs::remove_file(&path_b);

    match logging::init_file_logger(&path_a, None) {
        Ok(()) => {
            // First call won; the second must fail.
            let result = logging::init_file_logger(&path_b, None);
            assert!(result.is_err(), "second init must fail");
        }
        Err(e) => {
            // Another test already initialized the logger. That itself
            // demonstrates idempotent rejection.
            eprintln!("first init rejected (expected in some test orders): {e}");
        }
    }
}

#[test]
fn default_level_matches_build_profile() {
    let level = logging::default_level();
    if cfg!(debug_assertions) {
        assert_eq!(level, log::LevelFilter::Debug);
    } else {
        assert_eq!(level, log::LevelFilter::Warn);
    }
}
