use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_boxxy")
}

fn temp_home(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "boxxy-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap()
}

#[test]
fn help_and_version_do_not_need_a_config() {
    let home = temp_home("help");
    let help = run(&home, &["--help"]);
    assert!(help.status.success());
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert!(stdout.contains("--generate-config"), "{stdout}");

    let version = run(&home, &["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("boxxy 0.1.0"));
    let _ = fs::remove_dir_all(home);
}

#[test]
fn generate_config_refuses_to_clobber() {
    let home = temp_home("generate");
    let first = run(&home, &["--generate-config"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let path = home.join(".config/boxxycli/boxxy.toml");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        boxxy::config::DEFAULT_CONFIG
    );

    let second = run(&home, &["--generate-config"]);
    assert_eq!(second.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("Configuration already exists"), "{stderr}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        boxxy::config::DEFAULT_CONFIG
    );

    fs::write(&path, "stale\n").unwrap();
    let forced = run(&home, &["--generate-config", "--force"]);
    assert!(
        forced.status.success(),
        "{}",
        String::from_utf8_lossy(&forced.stderr)
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        boxxy::config::DEFAULT_CONFIG
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn missing_and_invalid_config_exit_before_the_terminal() {
    let home = temp_home("missing");
    let missing = run(&home, &[]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "No configuration found.\nRun 'boxxy --generate-config' to create one.\n"
    );

    let path = home.join(".config/boxxycli/boxxy.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "[frame]\nthickness = 1\nradius = \"rounded\"\ncolor = \"#89DCEB\"\npadding = 5\n\n[brand]\ntext = \"BOXXY\"\n",
    )
    .unwrap();
    let invalid = run(&home, &[]);
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&invalid.stderr),
        "Error:\nUnknown configuration key: frame.padding\n"
    );

    fs::write(&path, boxxy::config::DEFAULT_CONFIG).unwrap();
    let not_a_tty = run(&home, &[]);
    assert_eq!(not_a_tty.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&not_a_tty.stderr).contains("must be run in a terminal"),
        "{}",
        String::from_utf8_lossy(&not_a_tty.stderr)
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn xdg_config_home_is_not_consulted() {
    let home = temp_home("xdg");
    let xdg = temp_home("xdg-decoy");
    fs::create_dir_all(xdg.join("boxxycli")).unwrap();
    fs::write(
        xdg.join("boxxycli/boxxy.toml"),
        boxxy::config::DEFAULT_CONFIG,
    )
    .unwrap();

    let output = Command::new(bin())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "No configuration found.\nRun 'boxxy --generate-config' to create one.\n"
    );
    let _ = fs::remove_dir_all(home);
    let _ = fs::remove_dir_all(xdg);
}

#[test]
fn framed_shell_shows_output_and_the_border() {
    let home = temp_home("pty");
    fs::create_dir_all(home.join(".config/boxxycli")).unwrap();
    fs::write(
        home.join(".config/boxxycli/boxxy.toml"),
        boxxy::config::DEFAULT_CONFIG,
    )
    .unwrap();
    let script = home.join("shell.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '\\033[2J\\033[HFRAME_OK_9f3a\\n'\nexit 0\n",
    )
    .unwrap();
    make_executable(&script);

    let output = run_in_pty(
        &home,
        &script,
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
    );
    let text = String::from_utf8_lossy(&output.bytes);
    let visible = strip_ansi(&text);
    assert!(
        output.success,
        "status failed\nraw:\n{text}\nvisible:\n{visible}"
    );
    assert!(
        visible.contains("FRAME_OK_9f3a"),
        "marker missing\nvisible:\n{visible}"
    );
    assert!(visible.contains('╭'), "frame missing\nvisible:\n{visible}");
    let _ = fs::remove_dir_all(home);
}

#[test]
fn resize_reframes_the_shell_without_restarting_it() {
    let home = temp_home("resize");
    fs::create_dir_all(home.join(".config/boxxycli")).unwrap();
    fs::write(
        home.join(".config/boxxycli/boxxy.toml"),
        boxxy::config::DEFAULT_CONFIG,
    )
    .unwrap();
    let script = home.join("shell.py");
    fs::write(
        &script,
        r#"#!/usr/bin/env python3
import fcntl, signal, struct, sys, termios

def report(*_args):
    packed = fcntl.ioctl(1, termios.TIOCGWINSZ, struct.pack("HHHH", 0, 0, 0, 0))
    rows, cols, _, _ = struct.unpack("HHHH", packed)
    sys.stdout.write(f"SIZE {rows} {cols}\n")
    sys.stdout.flush()

signal.signal(signal.SIGWINCH, report)
sys.stdout.write("READY\n")
sys.stdout.flush()
signal.pause()
"#,
    )
    .unwrap();
    make_executable(&script);

    let system = native_pty_system();
    let pair = system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut command = CommandBuilder::new(bin());
    command.env("HOME", home.as_os_str());
    command.env("SHELL", script.as_os_str());
    let mut child = pair.slave.spawn_command(command).expect("spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let master = pair.master;
    let captured = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let slot = Arc::clone(&captured);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(count) => slot
                    .lock()
                    .expect("capture")
                    .extend_from_slice(&buf[..count]),
            }
        }
    });

    let done = Arc::new(AtomicBool::new(false));
    if let Some(pid) = child.process_id() {
        let flag = Arc::clone(&done);
        std::thread::spawn(move || {
            for _ in 0..50 {
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        });
    }

    let snapshot = || String::from_utf8_lossy(&captured.lock().expect("capture")).into_owned();
    let wait_until = |label: &str, ready: &mut dyn FnMut(&str) -> bool| {
        let started = std::time::Instant::now();
        loop {
            let text = snapshot();
            if ready(&strip_ansi(&text)) {
                return text;
            }
            if started.elapsed() > Duration::from_secs(3) {
                done.store(true, Ordering::SeqCst);
                panic!("timed out waiting for {label}: {text}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    wait_until("READY", &mut |visible| visible.contains("READY"));
    master
        .resize(PtySize {
            rows: 12,
            cols: 30,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("resize");
    let text = wait_until("the inner 10x28 size", &mut |visible| {
        let Some(at) = visible.find("SIZE") else {
            return false;
        };
        let rest = &visible[at + 4..];
        match (rest.find("10"), rest.find("28")) {
            (Some(rows), Some(cols)) => rows < cols,
            _ => false,
        }
    });

    let status = child.wait().expect("wait");
    done.store(true, Ordering::SeqCst);
    assert!(status.success(), "{text}");
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_tiny_terminal_is_rejected() {
    let home = temp_home("tiny");
    fs::create_dir_all(home.join(".config/boxxycli")).unwrap();
    fs::write(
        home.join(".config/boxxycli/boxxy.toml"),
        boxxy::config::DEFAULT_CONFIG,
    )
    .unwrap();
    let script = home.join("shell.sh");
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    make_executable(&script);

    let output = run_in_pty(
        &home,
        &script,
        PtySize {
            rows: 2,
            cols: 20,
            pixel_width: 0,
            pixel_height: 0,
        },
    );
    let text = String::from_utf8_lossy(&output.bytes);
    assert!(!output.success, "{text}");
    assert!(text.contains("too small"), "{text}");
    let _ = fs::remove_dir_all(home);
}

struct PtyOutput {
    success: bool,
    bytes: Vec<u8>,
}

fn run_in_pty(home: &Path, shell: &Path, size: PtySize) -> PtyOutput {
    let system = native_pty_system();
    let pair = system.openpty(size).expect("openpty");
    let mut command = CommandBuilder::new(bin());
    command.env("HOME", home.as_os_str());
    command.env("SHELL", shell.as_os_str());
    command.env("XDG_CONFIG_HOME", "/tmp/boxxy-does-not-read-xdg");
    let mut child = pair.slave.spawn_command(command).expect("spawn boxxy");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(count) => bytes.extend_from_slice(&buf[..count]),
            }
        }
        let _ = tx.send(bytes);
    });

    let done = Arc::new(AtomicBool::new(false));
    if let Some(pid) = child.process_id() {
        let flag = Arc::clone(&done);
        std::thread::spawn(move || {
            for _ in 0..50 {
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        });
    }

    let status = child.wait().expect("wait");
    done.store(true, Ordering::SeqCst);
    let bytes = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    PtyOutput {
        success: status.success(),
        bytes,
    }
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }
        out.push(ch);
    }
    out
}
