//! Module: capture. Purpose: capture preparation, recording lifetime and export.
//! Created: 2026-10-19. Architecture: GTK-free workers; one process owner per recording.
use crate::{
    files::{self, Result},
    gnome, portal,
    preset::Preset,
};
use std::{
    fs::File,
    io::Write,
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Area {
    pub fn from_points(
        start: (f64, f64),
        end: (f64, f64),
        width: i32,
        height: i32,
    ) -> Option<Self> {
        let x = start.0.min(end.0).floor().clamp(0.0, f64::from(width)) as i32;
        let y = start.1.min(end.1).floor().clamp(0.0, f64::from(height)) as i32;
        let right = start.0.max(end.0).ceil().clamp(0.0, f64::from(width)) as i32;
        let bottom = start.1.max(end.1).ceil().clamp(0.0, f64::from(height)) as i32;
        (right - x >= 2 && bottom - y >= 2).then_some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

pub enum Source {
    X11 { display: String },
    Wayland(portal::Stream),
    Gnome(gnome::Stream),
    Screenshot,
}

pub struct Prepared {
    pub source: Source,
    pub image: PathBuf,
    pub _scratch: tempfile::TempDir,
}

/// Captures a frozen preview. Wayland recording previews come from the exact selected stream.
pub fn prepare(wayland: bool, video: bool) -> Result<Prepared> {
    let started = Instant::now();
    let scratch = tempfile::Builder::new()
        .prefix("snappy-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let image = scratch.path().join("preview.png");
    let native = wayland && portal::runtime().block_on(gnome::available())?;
    let source = if wayland && !video {
        if native {
            portal::runtime().block_on(gnome::screenshot(&image))?;
        } else {
            let path = portal::runtime().block_on(portal::screenshot())?;
            std::fs::copy(&path, &image)
                .map_err(|e| format!("Cannot read portal screenshot: {e}"))?;
        }
        Source::Screenshot
    } else if wayland {
        let source = if native {
            Source::Gnome(portal::runtime().block_on(gnome::Stream::request())?)
        } else {
            Source::Wayland(portal::runtime().block_on(portal::Stream::request())?)
        };
        let (node, remote) = stream_connection(&source)?;
        let mut command = pipewire_command(node, remote.as_ref());
        command
            .args([
                "num-buffers=1",
                "!",
                "videoconvert",
                "!",
                "pngenc",
                "!",
                "filesink",
            ])
            .arg(gst_property("location", &image.to_string_lossy()));
        run_image_command(command, scratch.path(), Duration::from_secs(20))?;
        source
    } else {
        let (display, width, height) = x11_geometry()?;
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "x11grab",
                "-draw_mouse",
                "0",
                "-video_size",
            ])
            .arg(format!("{width}x{height}"))
            .args(["-i", &display, "-frames:v", "1"])
            .arg(&image);
        run_image_command(command, scratch.path(), Duration::from_secs(10))?;
        Source::X11 { display }
    };
    eprintln!(
        "Snappy capture: {} in {} ms",
        if native {
            "gnome"
        } else if wayland {
            "portal"
        } else {
            "x11"
        },
        started.elapsed().as_millis()
    );
    Ok(Prepared {
        source,
        image,
        _scratch: scratch,
    })
}

fn x11_geometry() -> Result<(String, i32, i32)> {
    let name = std::env::var("DISPLAY")
        .map_err(|_| "No X11 display. Start in a graphical session.".to_string())?;
    let xlib = x11_dl::xlib::Xlib::open().map_err(|e| e.to_string())?;
    // SAFETY: null selects DISPLAY. All queries use this live connection, closed once below.
    let (width, height) = unsafe {
        let display = (xlib.XOpenDisplay)(std::ptr::null());
        if display.is_null() {
            return Err("Cannot open the X11 display".into());
        }
        let screen = (xlib.XDefaultScreen)(display);
        let dimensions = (
            (xlib.XDisplayWidth)(display, screen),
            (xlib.XDisplayHeight)(display, screen),
        );
        (xlib.XCloseDisplay)(display);
        dimensions
    };
    Ok((name, width, height))
}

/// Arguments are passed directly, never through a shell. GStreamer property quoting
/// prevents paths or device names from becoming pipeline tokens.
fn gst_property(key: &str, value: &str) -> String {
    format!(
        "{key}=\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn stream_connection(source: &Source) -> Result<(u32, Option<OwnedFd>)> {
    match source {
        Source::Wayland(stream) => Ok((
            stream.node,
            Some(portal::runtime().block_on(stream.remote())?),
        )),
        Source::Gnome(stream) => Ok((stream.node, None)),
        _ => Err("Capture source is not a native stream".into()),
    }
}

fn pipewire_command(node: u32, remote: Option<&OwnedFd>) -> Command {
    let mut command = Command::new("gst-launch-1.0");
    command
        .args(["-q", "-e", "pipewiresrc"])
        .arg(format!("path={node}"))
        .arg("do-timestamp=true");
    if let Some(remote) = remote {
        let descriptor = remote.as_raw_fd();
        command.arg(format!("fd={descriptor}"));
        // SAFETY: the parent holds remote through spawn. Only async-signal-safe fcntl
        // runs after fork; clearing CLOEXEC exposes this one portal FD to this child.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(descriptor, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command
}

pub(crate) fn run_image_command(
    mut command: Command,
    scratch: &Path,
    timeout: Duration,
) -> Result<()> {
    let log = scratch.join("preview.log");
    let stderr = File::create(&log).map_err(|e| e.to_string())?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("Cannot start capture backend: {e}"))?;
    match wait_bounded(&mut child, timeout) {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(format!("Capture failed: {}", files::log_tail(&log))),
        Err(error) => Err(error),
    }
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Capture backend did not finish in time".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn desktop_monitor() -> String {
    // PulseAudio resolves this special source to the default output monitor,
    // including when served by PipeWire-Pulse. It never selects the microphone.
    "@DEFAULT_MONITOR@".into()
}

pub struct Recording {
    child: Child,
    prepared: Prepared,
    stage: Option<tempfile::TempDir>,
    raw: PathBuf,
    log: PathBuf,
    destination: PathBuf,
    pub preset: Preset,
    stopped: bool,
    started: Instant,
}

impl Recording {
    /// Records lossless RGB + FLAC first, so changing an export profile never compounds loss.
    /// Records at frame_rate frames per second, normally the display's refresh rate,
    /// so motion is as smooth in the file as it was on screen.
    pub fn start(
        prepared: Prepared,
        area: Area,
        bounds: Area,
        sound: bool,
        preset: Preset,
        folder: &Path,
        frame_rate: u32,
    ) -> Result<Self> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let stage = tempfile::Builder::new()
            .prefix(".snappy-")
            .tempdir_in(folder)
            .map_err(|e| e.to_string())?;
        let raw = stage.path().join("recording.mkv");
        let log = stage.path().join("capture.log");
        let monitor = if sound { Some(desktop_monitor()) } else { None };
        let connection = match &prepared.source {
            Source::Wayland(_) | Source::Gnome(_) => Some(stream_connection(&prepared.source)?),
            _ => None,
        };
        let mut command = match &prepared.source {
            Source::X11 { display } => {
                let mut command = Command::new("ffmpeg");
                command
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-y",
                        "-thread_queue_size",
                        "512",
                        "-f",
                        "x11grab",
                        "-framerate",
                    ])
                    .arg(frame_rate.to_string())
                    .arg("-video_size")
                    .arg(format!("{}x{}", area.width, area.height))
                    .arg("-i")
                    .arg(format!("{display}+{},{}", area.x, area.y));
                if let Some(monitor) = &monitor {
                    command.args(["-thread_queue_size", "512", "-f", "pulse", "-i", monitor]);
                }
                command
                    .args([
                        "-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgr0", "-c:a", "flac",
                    ])
                    .arg(&raw);
                command
            }
            Source::Wayland(_) | Source::Gnome(_) => {
                let (node, remote) = connection.as_ref().expect("native stream connection");
                let mut command = pipewire_command(*node, remote.as_ref());
                command
                    .args(["!", "videoconvert", "!", "videocrop"])
                    .arg(format!("left={}", area.x))
                    .arg(format!("top={}", area.y))
                    .arg(format!("right={}", bounds.width - area.x - area.width))
                    .arg(format!("bottom={}", bounds.height - area.y - area.height))
                    // A bounded queue keeps the PipeWire thread free while frames encode.
                    // gst-libav's FFV1 encoder runs on one thread unless slices, the slice
                    // thread type and threads=auto are all set; together they keep 4K at
                    // the display rate within real time on a desktop CPU.
                    .args([
                        "!",
                        "queue",
                        "max-size-buffers=8",
                        "max-size-bytes=0",
                        "max-size-time=0",
                        "!",
                        "videorate",
                        "!",
                    ])
                    .arg(format!("video/x-raw,format=BGRx,framerate={frame_rate}/1"))
                    .args([
                        "!",
                        "avenc_ffv1",
                        "slices=16",
                        "thread-type=slice",
                        "threads=auto",
                        "!",
                        "queue",
                        "!",
                        "mux.",
                        "matroskamux",
                        "name=mux",
                        "!",
                        "filesink",
                    ])
                    .arg(gst_property("location", &raw.to_string_lossy()));
                if let Some(monitor) = &monitor {
                    command
                        .arg("pulsesrc")
                        .arg(gst_property("device", monitor))
                        .args([
                            "do-timestamp=true",
                            "!",
                            "queue",
                            "!",
                            "audioconvert",
                            "!",
                            "audioresample",
                            "!",
                            "flacenc",
                            "!",
                            "queue",
                            "!",
                            "mux.",
                        ]);
                }
                command
            }
            Source::Screenshot => return Err("A screenshot is not a recording source".into()),
        };
        let stderr = File::create(&log).map_err(|e| e.to_string())?;
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("Cannot start recorder: {e}"))?;
        drop(connection);
        let destination = match files::reserve(folder, preset.extension()) {
            Ok(path) => path,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            prepared,
            stage: Some(stage),
            raw,
            log,
            destination,
            preset,
            stopped: false,
            started: Instant::now(),
        })
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Detects encoder errors and compositor-revoked screen sharing without blocking GTK.
    pub fn exited(&mut self) -> Result<bool> {
        self.child
            .try_wait()
            .map(|s| s.is_some())
            .map_err(|e| e.to_string())
    }

    /// Finalizes the container, then exports. The worker owns the recording until complete.
    pub fn finish(mut self) -> Result<PathBuf> {
        self.stopped = true;
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            match &self.prepared.source {
                Source::X11 { .. } => {
                    if let Some(input) = self.child.stdin.as_mut() {
                        let _ = input.write_all(b"q\n");
                    }
                }
                _ => {
                    // SAFETY: this is our unreaped child PID; SIGINT asks gst-launch -e to send EOS.
                    unsafe {
                        libc::kill(self.child.id() as i32, libc::SIGINT);
                    }
                }
            }
        }
        let status = wait_bounded(&mut self.child, Duration::from_secs(20));
        if status
            .as_ref()
            .map(|s| !s.success() && s.code() != Some(130))
            .unwrap_or(true)
        {
            return Err(self.recover(format!("Recording failed: {}", files::log_tail(&self.log))));
        }
        if self.preset == Preset::Lossless {
            if let Err(error) = std::fs::copy(&self.raw, &self.destination) {
                return Err(self.recover(format!("Cannot save recording: {error}")));
            }
        } else if let Err(error) = self.export() {
            return Err(self.recover(error));
        }
        Ok(self.destination.clone())
    }

    fn export(&mut self) -> Result<()> {
        let (video, audio, height) = self.preset.encoding();
        let stderr = File::create(&self.log).map_err(|e| e.to_string())?;
        // Escape the comma for FFmpeg's filter parser. Pad odd selections for H.264.
        // The recorded frame rate is kept; only the resolution is limited.
        let filter = format!(
            "scale=-2:'min({height}\\,ih)':force_original_aspect_ratio=decrease,pad=ceil(iw/2)*2:ceil(ih/2)*2"
        );
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&self.raw)
            .args([
                "-map", "0:v:0", "-map", "0:a?", "-vf", &filter, "-c:v", "libx264", "-preset",
                "medium", "-b:v",
            ])
            .arg(format!("{video}k"))
            .arg("-maxrate")
            .arg(format!("{}k", video * 2))
            .arg("-bufsize")
            .arg(format!("{}k", video * 4))
            .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a"])
            .arg(format!("{audio}k"))
            .args(["-movflags", "+faststart"])
            .arg(&self.destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr);
        self.child = command.spawn().map_err(|e| format!("Cannot export: {e}"))?;
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("Export failed: {}", files::log_tail(&self.log)));
        }
        Ok(())
    }

    fn recover(&mut self, error: String) -> String {
        let _ = std::fs::remove_file(&self.destination);
        let path = self.stage.take().map(|dir| dir.keep()).unwrap_or_default();
        format!("{error}\nOriginal recording kept in {}", path.display())
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if !self.stopped {
            let _ = std::fs::remove_file(&self.destination);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_normalizes_reverse_drag_and_clamps_edges() {
        assert_eq!(
            Area::from_points((100.0, 80.0), (-20.0, 10.0), 200, 100),
            Some(Area {
                x: 0,
                y: 10,
                width: 100,
                height: 70
            })
        );
        assert!(Area::from_points((0.0, 0.0), (0.2, 0.2), 100, 100).is_none());
    }
}
