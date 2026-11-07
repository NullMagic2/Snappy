//! Module: files. Purpose: private scratch files and collision-free capture destinations.
//! Created: 2026-10-19. Architecture: shared filesystem policy for both backends.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, String>;

/// Runs a worker command with private output files and bounded returned diagnostics.
/// Explicit stdin lets the privileged pack helper receive bytes, never a user-supplied path.
pub fn command_output(
    mut command: Command,
    input: Stdio,
    scratch: &Path,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> std::io::Result<String> {
    if cancelled() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "Command cancelled",
        ));
    }
    let output = scratch.join("stdout.log");
    let log = scratch.join("stderr.log");
    let mut child = command
        .env("LC_ALL", "C")
        .stdin(input)
        .stdout(File::create(&output)?)
        .stderr(File::create(&log)?)
        .spawn()?;
    let status = wait_bounded(&mut child, timeout, cancelled)?;
    if !status.success() {
        let details = log_tail(&log);
        return Err(std::io::Error::other(if details.is_empty() {
            status.to_string()
        } else {
            details
        }));
    }
    let mut text = String::new();
    const MAX_TEXT_BYTES: u64 = 1024 * 1024;
    File::open(output)?
        .take(MAX_TEXT_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_TEXT_BYTES {
        return Err(std::io::Error::other("Command output exceeds 1 MiB"));
    }
    Ok(text)
}

/// Waits on a worker, reaping the owned child on cancellation, timeout or I/O failure.
pub fn wait_bounded(
    child: &mut Child,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => (),
            Err(error) => break Err(error),
        }
        if cancelled() {
            break Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Command cancelled",
            ));
        }
        if Instant::now() >= deadline {
            break Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Command did not finish in time",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Local capture timestamp, taken when a capture happens rather than when it is saved.
pub fn stamp() -> Result<String> {
    glib::DateTime::now_local()
        .and_then(|d| d.format("%Y-%m-%d_%H-%M-%S"))
        .map(|stamp| stamp.to_string())
        .map_err(|e| e.to_string())
}

/// The capture name shared by screenshots and recordings. Counter 0 is the plain
/// name an export dialog can show before saving; later counters resolve collisions.
pub fn name(stamp: &str, counter: u32, extension: &str) -> String {
    if counter == 0 {
        format!("Capture_{stamp}.{extension}")
    } else {
        format!("Capture_{stamp}_{counter}.{extension}")
    }
}

/// Reserves a new path, stamped now, without overwriting any previous capture.
pub fn reserve(folder: &Path, extension: &str) -> Result<PathBuf> {
    reserve_stamped(folder, &stamp()?, extension)
}

/// Reserves a new path for an earlier timestamp without overwriting any previous capture.
pub fn reserve_stamped(folder: &Path, stamp: &str, extension: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(folder).map_err(|e| format!("Cannot create output folder: {e}"))?;
    for counter in 0..1000 {
        let path = folder.join(name(stamp, counter, extension));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot save capture: {e}")),
        }
    }
    Err("Too many captures with the same timestamp".into())
}

/// Keeps only a small diagnostic tail, even for a long recording.
pub fn log_tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(size.saturating_sub(8192)));
    let mut text = String::new();
    let _ = file.read_to_string(&mut text);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_wait_reaps_cancelled_and_timed_out_children() {
        for (timeout, cancel, expected) in [
            (
                Duration::from_secs(5),
                true,
                std::io::ErrorKind::Interrupted,
            ),
            (Duration::ZERO, false, std::io::ErrorKind::TimedOut),
        ] {
            let mut child = std::process::Command::new("sleep")
                .arg("5")
                .spawn()
                .unwrap();
            let error = wait_bounded(&mut child, timeout, || cancel).unwrap_err();
            assert_eq!(error.kind(), expected);
            assert!(child.try_wait().unwrap().is_some());
        }
    }

    #[test]
    fn previewed_name_is_reserved_first_and_collisions_add_a_counter() {
        let temp = tempfile::tempdir().unwrap();
        let stamp = "2026-10-20_12-00-00";
        let first = reserve_stamped(temp.path(), stamp, "png").unwrap();
        assert_eq!(first, temp.path().join(name(stamp, 0, "png")));
        let second = reserve_stamped(temp.path(), stamp, "png").unwrap();
        assert_eq!(
            second.file_name().unwrap(),
            "Capture_2026-10-20_12-00-00_1.png"
        );
    }
}
