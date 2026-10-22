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
        vec!["--check"],
        vec!["--sample-rate", "8000"],
    ] {
        let output = run(&dir, &args);
        assert!(!output.status.success(), "{args:?}");
    }
}
