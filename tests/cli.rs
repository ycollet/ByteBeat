use std::{
    fs,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

fn run(directory: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bytebeat"))
        .current_dir(directory.path())
        .args(args)
        .output()
        .expect("launch bytebeat checker")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn headless_checks_do_not_contact_nsm() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("song.js"), "t").unwrap();
    let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bytebeat"))
        .current_dir(dir.path())
        .env(
            "NSM_URL",
            format!("osc.udp://{}/", server.local_addr().unwrap()),
        )
        .args(["--check", "--file", "song.js", "--seconds", "0.01"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let error = server.recv_from(&mut [0; 4096]).unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
}

#[test]
fn checks_stereo_file_using_sidecar_and_writes_report() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("song.js"), "[sin(t), cos(t)];").unwrap();
    fs::write(
        dir.path().join("song.json"),
        json!({
            "title": "Stereo test", "author": "Test",
            "mode": "Floatbeat", "rate": 35498.66666
        })
        .to_string(),
    )
    .unwrap();
    let output = run(
        &dir,
        &[
            "--file",
            "song.js",
            "--check",
            "--seconds",
            "0.01",
            "--report",
            "report.json",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("PASS song.js"));
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["failed"], 0);
    assert_eq!(report["results"][0]["rate"], 35498.66666);
    assert_eq!(report["results"][0]["mode"], "Floatbeat");
    assert_eq!(report["results"][0]["samples_checked"], 355);
    let performance = &report["results"][0]["performance"];
    assert_eq!(performance["output_frames"], 480);
    assert_eq!(performance["blocks_measured"], 4);
    let render_ms = performance["render_ms"].as_f64().unwrap();
    assert!(render_ms >= 0.0);
    assert!((performance["realtime_factor"].as_f64().unwrap() - render_ms / 10.0).abs() < 1e-9);
    assert!((performance["block_budget_ms"].as_f64().unwrap() - 8.0 / 3.0).abs() < 1e-12);
    assert!(performance["blocks_over_budget"].as_u64().unwrap() <= 4);
    let mut previous = 0.0;
    for key in [
        "block_p50_ms",
        "block_p95_ms",
        "block_p99_ms",
        "block_max_ms",
    ] {
        let current = performance[key].as_f64().unwrap();
        assert!(current >= previous, "{performance}");
        previous = current;
    }
    assert!(stderr(&output).contains("Performance song.js"));
}

#[test]
fn nan_output_passes_check_and_warns_only_once_across_blocks() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("nan.js"), "[NaN, NaN]").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_bytebeat"))
        .current_dir(dir.path())
        .env("RUST_LOG", "warn")
        .args([
            "--file",
            "nan.js",
            "--check",
            "--seconds",
            "0.1",
            "--report",
            "report.json",
        ])
        .output()
        .expect("launch bytebeat checker");
    assert!(output.status.success(), "{}", stderr(&output));
    let log = stderr(&output);
    assert_eq!(log.matches("NaN output at sample").count(), 1, "{log}");
    assert!(log.contains("NaN output at sample 0"), "{log}");
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["failed"], 0);
}

#[test]
fn reports_runtime_error_with_filename_and_sample_position() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("broken.js"),
        "t === 150 ? (()=>{throw Error('diagnostic marker')})() : 128",
    )
    .unwrap();
    let output = run(
        &dir,
        &["--file", "broken.js", "--check", "--seconds", "0.1"],
    );
    assert_eq!(output.status.code(), Some(1));
    let log = stderr(&output);
    assert!(log.contains("FAIL broken.js"), "{log}");
    assert!(log.contains("sample 150"), "{log}");
    assert!(log.contains("diagnostic marker"), "{log}");
}

#[test]
fn catalog_resolves_relative_paths_and_continues_after_failure() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("library")).unwrap();
    fs::write(dir.path().join("library/good.js"), "t").unwrap();
    fs::write(dir.path().join("library/bad.js"), "t +").unwrap();
    fs::write(
        dir.path().join("library/catalog.json"),
        json!({
            "version": 1, "songs": [
                {"file":"bad.js", "title":"Bad", "author":"Test", "mode":"Bytebeat", "rate":8000},
                {"file":"good.js", "title":"Good", "author":"Test", "mode":"Bytebeat", "rate":8000}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let output = run(
        &dir,
        &[
            "--catalog",
            "library/catalog.json",
            "--check",
            "--seconds",
            "0.01",
            "--report",
            "report.json",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("1 passed, 1 failed"));
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["results"][0]["stage"], "compile");
    assert!(report["results"][0]["performance"].is_null());
    assert_eq!(report["results"][1]["stage"], "ok");
}

#[test]
fn overrides_metadata_for_funcbeat() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("song.js"), "return t => 0.25 * sin(t);").unwrap();
    let output = run(
        &dir,
        &[
            "--file",
            "song.js",
            "--mode",
            "funcbeat",
            "--sample-rate",
            "44100",
            "--check",
            "--seconds",
            "0.01",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn rejects_missing_files_bad_metadata_and_invalid_arguments() {
    let dir = TempDir::new().unwrap();
    let missing = run(&dir, &["--file", "missing.js", "--check"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).contains("missing.js"));
    fs::write(dir.path().join("song.json"), "{not json").unwrap();
    let bad_metadata = run(&dir, &["--file", "song.js", "--check"]);
    assert_eq!(bad_metadata.status.code(), Some(1));
    assert!(stderr(&bad_metadata).contains("song.json"));
    for args in [
        vec!["--check", "--seconds", "NaN"],
        vec!["--check", "--sample-rate", "0"],
        vec!["--check", "--catalog", "missing-catalog.json"],
        vec!["--sample-rate", "8000"],
    ] {
        let output = run(&dir, &args);
        assert!(!output.status.success(), "{args:?}");
    }
}

#[test]
fn logs_readable_thrown_text_and_continues_with_diagnostics_in_report() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("display.js"),
        "t === 0 || t === 10 ? (() => { throw 'first line\\nsecond line'; })() : 192",
    )
    .unwrap();
    let output = run(
        &dir,
        &[
            "--file",
            "display.js",
            "--check",
            "--seconds",
            "0.01",
            "--report",
            "report.json",
        ],
    );
    let log = stderr(&output);
    assert!(output.status.success(), "{log}");
    assert!(log.contains("Thrown text at sample 0"), "{log}");
    assert!(log.contains("Thrown text at sample 10"), "{log}");
    assert!(
        log.contains("first line") && log.contains("second line"),
        "{log}"
    );
    assert!(!log.contains("String(Ok"), "{log}");
    assert!(!log.contains("\\nsecond line"), "{log}");
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(report["results"][0]["thrown_texts"], 2);
    assert_eq!(report["results"][0]["samples_checked"], 80);
    assert_eq!(report["results"][0]["peak"], 0.5);
}

#[test]
fn cli_heap_limit_changes_available_memory_and_is_recorded() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("memory.js"),
        "(t || (globalThis.buffer = new Uint8Array(2 * 1024 * 1024)), 128)",
    )
    .unwrap();
    let small = run(
        &dir,
        &[
            "--file",
            "memory.js",
            "--check",
            "--seconds",
            "0.01",
            "--memory-limit-mb",
            "1",
        ],
    );
    assert_eq!(small.status.code(), Some(1));
    assert!(stderr(&small).contains("out of memory"));
    let large = run(
        &dir,
        &[
            "--file",
            "memory.js",
            "--check",
            "--seconds",
            "0.01",
            "--memory-limit-mb",
            "8",
            "--report",
            "report.json",
        ],
    );
    assert!(large.status.success(), "{}", stderr(&large));
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["memory_limit_mb"], 8);
    for limit in ["0", "-1", "not-a-number"] {
        let output = run(&dir, &["--check", "--memory-limit-mb", limit]);
        assert!(!output.status.success(), "{limit}");
    }
}

#[test]
fn later_fatal_errors_keep_earlier_thrown_text_counts() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("display.js"),
            "t === 150 ? (() => { throw 'display'; })() : t === 200 ? (() => { throw Error('fatal'); })() : 128").unwrap();
    let output = run(
        &dir,
        &[
            "--file",
            "display.js",
            "--check",
            "--seconds",
            "0.1",
            "--report",
            "report.json",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("sample 200"));
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["results"][0]["thrown_texts"], 1);
    assert_eq!(report["results"][0]["success"], false);
    assert!(report["results"][0]["performance"].is_null());
}

#[test]
fn slow_rendering_warns_without_failing_compatibility() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("slow.js"),
        "(t === 0 && (() => { const end = Date.now() + 20; while (Date.now() < end) {} })(), 128)",
    )
    .unwrap();
    let output = run(
        &dir,
        &[
            "--file",
            "slow.js",
            "--check",
            "--seconds",
            "0.01",
            "--report",
            "report.json",
        ],
    );
    let log = stderr(&output);
    assert!(output.status.success(), "{log}");
    assert!(log.contains("PASS slow.js"), "{log}");
    assert!(log.contains("real-time playback may underrun"), "{log}");
    let report: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["passed"], 1);
    assert_eq!(
        report["results"][0]["performance"]["over_realtime_budget"],
        true
    );
    assert!(
        report["results"][0]["performance"]["realtime_factor"]
            .as_f64()
            .unwrap()
            > 1.0
    );
}

#[test]
fn jack_is_advertised_but_conflicts_with_headless_checking() {
    let dir = TempDir::new().unwrap();
    let help = run(&dir, &["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--jack"));
    let output = run(&dir, &["--jack", "--check"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("--jack") && stderr(&output).contains("--check"));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn jack_is_explicitly_rejected_outside_linux_before_starting_the_gui() {
    let dir = TempDir::new().unwrap();
    let output = run(&dir, &["--jack"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("--jack is only supported on Linux"));
}

#[test]
fn checks_the_local_catalog_without_a_catalog_argument() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("data")).unwrap();
    fs::write(dir.path().join("data/song.js"), "t").unwrap();
    fs::write(dir.path().join("data/catalog.json"), json!({
        "version":1,
        "songs":[{"file":"song.js","title":"Discovered","author":"Test","mode":"Bytebeat","rate":8000}]
    }).to_string()).unwrap();
    let output = run(&dir, &["--check", "--seconds", "0.01"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("Discovered"));
    assert!(!stderr(&output).contains("Downloading"));
}
