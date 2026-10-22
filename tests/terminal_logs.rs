#![cfg(unix)]

use std::{
    fs::{self, File},
    io::{self, Read},
    os::fd::FromRawFd,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

fn run_in_terminal(directory: &TempDir, args: &[&str], style: &str) -> (ExitStatus, String) {
    let mut master_fd = -1;
    let mut slave_fd = -1;
    // The descriptor pointers are writable; null selects the default optional PTY settings.
    let result = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0, "openpty: {}", io::Error::last_os_error());
    // Successful openpty returns two newly owned descriptors, each transferred exactly once.
    let (mut master, slave) =
        unsafe { (File::from_raw_fd(master_fd), File::from_raw_fd(slave_fd)) };
    let reader = thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut captured = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            match master.read(&mut buffer) {
                Ok(0) => return Ok(captured),
                Ok(size) => captured.extend_from_slice(&buffer[..size]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // Linux PTY masters signal a closed slave with EIO instead of EOF.
                Err(error) if error.raw_os_error() == Some(libc::EIO) => return Ok(captured),
                Err(error) => return Err(error),
            }
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_bytebeat"))
        .current_dir(directory.path())
        .args(args)
        .env("TERM", "xterm-256color")
        .env("RUST_LOG", "info")
        .env("RUST_LOG_STYLE", style)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(slave)
        .spawn()
        .expect("launch checker with PTY stderr");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll checker") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop timed-out checker");
            child.wait().expect("reap timed-out checker");
            panic!("checker exceeded the PTY regression test deadline");
        }
        thread::sleep(Duration::from_millis(5));
    };
    let captured = reader.join().expect("join PTY reader").expect("read PTY");
    let text = String::from_utf8(captured).expect("UTF-8 terminal output");
    (status, text.replace("\r\n", "\n"))
}

#[test]
fn terminal_diagnostics_escape_untrusted_controls_but_preserve_text_and_report_data() {
    let directory = TempDir::new().unwrap();
    let controls = "first line\n\x1b]52;c;QQ==\x07\x1b[2J\r\x08\t\0\x7f\u{9b}\u{9d}last line";
    let escaped = r"first line
\u{1b}]52;c;QQ==\u{7}\u{1b}[2J\r\u{8}\t\u{0}\u{7f}\u{9b}\u{9d}last line";
    let recovered = format!("RECOVER-{controls}");
    let fatal = format!("FATAL-{controls}");
    let factory = format!("FACTORY-{controls}");
    let title = format!("TITLE-{controls}");
    let author = format!("AUTHOR-{controls}");
    fs::write(
        directory.path().join("recover.js"),
        format!(
            "t === 0 ? (() => {{ throw {}; }})() : 128",
            serde_json::to_string(&recovered).unwrap(),
        ),
    )
    .unwrap();
    fs::write(
        directory.path().join("fatal.js"),
        format!(
            "(() => {{ throw Error({}); }})()",
            serde_json::to_string(&fatal).unwrap(),
        ),
    )
    .unwrap();
    fs::write(
        directory.path().join("factory.js"),
        format!("throw {};", serde_json::to_string(&factory).unwrap()),
    )
    .unwrap();
    fs::write(
        directory.path().join("catalog.json"),
        json!({
            "version": 1,
            "songs": [
                {"file":"recover.js","title":title,"author":author,"mode":"Bytebeat","rate":8000},
                {"file":"fatal.js","title":title,"author":author,"mode":"Bytebeat","rate":8000},
                {"file":"factory.js","title":title,"author":author,"mode":"Funcbeat","rate":8000}
            ]
        })
        .to_string(),
    )
    .unwrap();
    for style in ["auto", "always", "never"] {
        let (status, text) = run_in_terminal(
            &directory,
            &[
                "--catalog",
                "catalog.json",
                "--check",
                "--seconds",
                "0.01",
                "--report",
                "report.json",
            ],
            style,
        );
        assert_eq!(status.code(), Some(1), "{style}: {text:?}");
        if style == "always" {
            assert!(
                text.contains("\x1b["),
                "trusted level colors missing: {text:?}"
            );
        }
        assert!(!text.contains("\x1b]"), "raw OSC sequence: {text:?}");
        assert!(
            !text.contains("\x1b[2J"),
            "raw screen-clear command: {text:?}"
        );
        assert!(
            !text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\x1b'),
            "raw non-formatting controls: {text:?}",
        );
        for label in ["RECOVER", "FATAL", "FACTORY", "TITLE", "AUTHOR"] {
            assert!(text.contains(&format!("{label}-{escaped}")), "{text:?}");
        }
        let report: Value = serde_json::from_str(
            &fs::read_to_string(directory.path().join("report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["passed"], 1);
        assert_eq!(report["failed"], 2);
        assert_eq!(report["results"][0]["thrown_texts"], 1);
        assert_eq!(report["results"][0]["title"], title);
        assert_eq!(report["results"][0]["author"], author);
        assert!(
            report["results"][1]["error"]
                .as_str()
                .unwrap()
                .contains(&fatal)
        );
        assert!(
            report["results"][2]["error"]
                .as_str()
                .unwrap()
                .contains(&factory)
        );
    }
}
