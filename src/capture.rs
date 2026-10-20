//! Module: capture. Purpose: capture preparation, recording lifetime and export.
//! Created: 2026-10-19. Architecture: GTK-free workers; one process owner per recording.
use crate::{
    files::{self, Result},
    gnome, portal,
    preset::Preset,
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::OnceLock,
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
    // The one-time encoder probe runs beside the preview so Record rarely waits for
    // it; X11 prepares before the user chooses between screenshot and video.
    thread::spawn(|| {
        hardware_encoders();
    });
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
        let mut command = pipewire_command(node, remote.as_ref(), false);
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

/// A verbose launch prints property notifications on stdout, which a recording
/// redirects to its statistics file to collect the videorate counters.
fn pipewire_command(node: u32, remote: Option<&OwnedFd>, verbose: bool) -> Command {
    let mut command = Command::new("gst-launch-1.0");
    command
        .args([if verbose { "-v" } else { "-q" }, "-e", "pipewiresrc"])
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

/// The VA-API H.264 encoders that a short synthetic encode proved to work, probed
/// once per process. GStreamer's va plugin binds to its default DRM render node;
/// FFmpeg needs an explicit node, so the render nodes are tried in name order.
pub struct HardwareEncoders {
    pub gstreamer: bool,
    pub ffmpeg_device: Option<PathBuf>,
}

/// VA-API encoders refuse frames smaller than this; smaller outputs use software.
const HARDWARE_MINIMUM_SIZE: i32 = 128;

static HARDWARE: OnceLock<HardwareEncoders> = OnceLock::new();

/// Finds the hardware encoders once; later calls return the cached result. Setting
/// SNAPPY_SOFTWARE_ENCODING=1 disables them for comparison or diagnosis.
pub fn hardware_encoders() -> &'static HardwareEncoders {
    HARDWARE.get_or_init(|| {
        let disabled =
            std::env::var_os("SNAPPY_SOFTWARE_ENCODING").is_some_and(|value| value == "1");
        let found = if disabled {
            HardwareEncoders {
                gstreamer: false,
                ffmpeg_device: None,
            }
        } else {
            probe_hardware_encoders()
        };
        eprintln!(
            "Snappy encoders: GStreamer VA-API H.264 {}, FFmpeg VA-API H.264 {}",
            if found.gstreamer {
                "available"
            } else {
                "unavailable"
            },
            found
                .ffmpeg_device
                .as_ref()
                .map(|node| node.display().to_string())
                .unwrap_or_else(|| "unavailable".into())
        );
        found
    })
}

/// Runs the exact element and filter chains of the direct recording path on two
/// tiny synthetic frames, so a missing plugin, driver or device falls back honestly.
fn probe_hardware_encoders() -> HardwareEncoders {
    let none = HardwareEncoders {
        gstreamer: false,
        ffmpeg_device: None,
    };
    let Ok(scratch) = tempfile::Builder::new().prefix("snappy-probe-").tempdir() else {
        return none;
    };
    let timeout = Duration::from_secs(15);
    let (video, audio, _) = Preset::Discord.encoding();
    let (width, height) = (2 * HARDWARE_MINIMUM_SIZE, HARDWARE_MINIMUM_SIZE);
    let mut command = Command::new("gst-launch-1.0");
    command
        .args(["-q", "videotestsrc", "num-buffers=2", "!"])
        .arg(format!(
            "video/x-raw,format=BGRx,width={width},height={height},framerate=60/1"
        ))
        .arg("!")
        .args(gst_hardware_video(video, width, height))
        .args([
            "!",
            "queue",
            "!",
            "mux.",
            "audiotestsrc",
            "num-buffers=2",
            "!",
            "audio/x-raw,rate=48000,channels=2",
            "!",
            "audioconvert",
            "!",
        ])
        .args(gst_aac(audio))
        .args([
            "!", "queue", "!", "mux.", "mp4mux", "name=mux", "!", "fakesink",
        ]);
    let gstreamer = run_image_command(command, scratch.path(), timeout).is_ok();
    let ffmpeg_device = render_nodes().into_iter().find(|node| {
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-loglevel", "error", "-vaapi_device"])
            .arg(node)
            .args(["-f", "lavfi", "-i"])
            .arg(format!(
                "testsrc2=size={width}x{height}:rate=60,format=bgr0"
            ))
            .args(["-frames:v", "2", "-vf"])
            .arg(ffmpeg_hardware_filter(width, height))
            .args(ffmpeg_h264(video, true))
            .args(["-f", "null", "-"]);
        run_image_command(command, scratch.path(), timeout).is_ok()
    });
    HardwareEncoders {
        gstreamer,
        ffmpeg_device,
    }
}

/// DRM render nodes in name order, normally /dev/dri/renderD128 first.
fn render_nodes() -> Vec<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("renderD"))
                })
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}

/// H.264 rate control shared by every encoder, in kilobits: the preset rate is the
/// target, twice the rate the ceiling and four times the rate the decoder buffer.
struct RateControl {
    target: u32,
    maximum: u32,
    buffer: u32,
}

impl RateControl {
    fn new(video_kbps: u32) -> Self {
        Self {
            target: video_kbps,
            maximum: video_kbps * 2,
            buffer: video_kbps * 4,
        }
    }
}

/// Colorimetry pinned on the GPU converter's output: limited range, BT.709 matrix
/// and primaries, and the sRGB transfer of the captured desktop pixels. Left to
/// negotiation, the converter picked a BT.709 or gamma 2.2 transfer and the AMD
/// driver then re-curved every pixel (black 16 came out as 23–27), which is what
/// made recordings look different from the screen; keeping the transfer makes the
/// decoded frame match the capture exactly, with the same tags libx264 wrote.
const GST_H264_COLORIMETRY: &str = "2:3:7:1";

/// GStreamer video elements of the direct path: the GPU scales and converts each
/// frame and encodes H.264 through VA-API; the parser reframes the byte stream for MP4.
fn gst_hardware_video(video_kbps: u32, width: i32, height: i32) -> Vec<String> {
    let rate = RateControl::new(video_kbps);
    vec![
        "vapostproc".into(),
        "!".into(),
        format!(
            "video/x-raw(memory:VAMemory),format=NV12,width={width},height={height},colorimetry={GST_H264_COLORIMETRY}"
        ),
        "!".into(),
        "vah264enc".into(),
        "rate-control=vbr".into(),
        format!("bitrate={}", rate.maximum),
        format!("target-percentage={}", rate.target * 100 / rate.maximum),
        format!("cpb-size={}", rate.buffer),
        "!".into(),
        "h264parse".into(),
    ]
}

/// GStreamer AAC encoder at the preset's audio rate; the libav property is in bits.
fn gst_aac(audio_kbps: u32) -> Vec<String> {
    vec!["avenc_aac".into(), format!("bitrate={}", audio_kbps * 1000)]
}

/// FFmpeg filter of the direct path: raw frames are uploaded as they are and scaled
/// and converted to NV12 on the GPU, which costs a fraction of scaling them first.
/// The grabbed RGB is declared full-range sRGB before the upload and the GPU output
/// limited-range BT.709, the same colorimetry GST_H264_COLORIMETRY pins: without the
/// declaration the driver guessed the input range and tagged the MP4 as full-range
/// GBR, and an output transfer requested here instead would re-curve the pixels.
fn ffmpeg_hardware_filter(width: i32, height: i32) -> String {
    format!(
        "setparams=range=pc:color_trc=iec61966-2-1:color_primaries=bt709,hwupload,scale_vaapi=w={width}:h={height}:format=nv12:out_range=limited:out_color_matrix=bt709"
    )
}

/// FFmpeg H.264 encoder arguments: h264_vaapi for direct recording, otherwise libx264.
fn ffmpeg_h264(video_kbps: u32, hardware: bool) -> Vec<String> {
    let rate = RateControl::new(video_kbps);
    let encoder: &[&str] = if hardware {
        &["-c:v", "h264_vaapi", "-rc_mode", "VBR"]
    } else {
        &[
            "-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p",
        ]
    };
    let mut arguments: Vec<String> = encoder.iter().map(|text| text.to_string()).collect();
    arguments.extend([
        "-b:v".into(),
        format!("{}k", rate.target),
        "-maxrate".into(),
        format!("{}k", rate.maximum),
        "-bufsize".into(),
        format!("{}k", rate.buffer),
    ]);
    arguments
}

/// FFmpeg AAC encoder at the preset's audio rate plus the MP4 layout for playback
/// that starts before the download ends.
fn ffmpeg_aac_mp4(audio_kbps: u32) -> Vec<String> {
    vec![
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        format!("{audio_kbps}k"),
        "-movflags".into(),
        "+faststart".into(),
    ]
}

/// Frames a finished recording holds at its frame rate, and how many of those slots
/// the recorder had to fill with a repeated frame because no new frame was captured
/// in time. Zero dropped frames means the recording is as smooth as the screen was.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStatistics {
    pub expected: u64,
    pub dropped: u64,
}

impl FrameStatistics {
    /// Expected frames come from the finished container's duration at the recorded
    /// rate. FFmpeg counts the frames it encoded, duplicates included; GStreamer's
    /// videorate only reports the repeats it inserted into its constant-rate output.
    fn new(counters: &RecorderCounters, duration_seconds: f64, frame_rate: u32) -> Self {
        let expected = (duration_seconds * f64::from(frame_rate)).round().max(0.0) as u64;
        let captured = counters
            .encoded
            .unwrap_or(expected)
            .saturating_sub(counters.duplicated);
        Self {
            expected,
            dropped: expected.saturating_sub(captured),
        }
    }
}

/// A finished recording at its final path, with its frame completeness.
pub struct Saved {
    pub path: PathBuf,
    pub frames: FrameStatistics,
}

/// Cumulative counters a recorder prints about its own frames while it runs.
#[derive(Debug, Default, PartialEq, Eq)]
struct RecorderCounters {
    /// Frames FFmpeg encoded, when it reported progress; GStreamer reports none.
    encoded: Option<u64>,
    /// Frames fabricated by repeating the previous one: FFmpeg's dup_frames or the
    /// duplicate count of a GStreamer videorate element.
    duplicated: u64,
}

/// Reads recorder counters from the statistics a recorder wrote. FFmpeg's progress
/// file repeats key=value blocks, and gst-launch -v prints a videorate element's
/// "duplicate = N" notifications; later lines supersede earlier ones. The text is
/// tool output, so only these two shapes with a plain decimal count are accepted and
/// every other line is ignored. Memory stays constant however long the recording ran.
fn recorder_counters(reader: impl BufRead) -> RecorderCounters {
    let mut counters = RecorderCounters::default();
    for line in reader.lines().map_while(|line| line.ok()) {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("frame=") {
            if let Ok(encoded) = value.trim().parse() {
                counters.encoded = Some(encoded);
            }
        } else if let Some(value) = line.strip_prefix("dup_frames=") {
            if let Ok(duplicated) = value.trim().parse() {
                counters.duplicated = duplicated;
            }
        } else if let Some((element, notification)) = line.split_once(": ") {
            if element.contains("GstVideoRate") {
                if let Some(duplicated) = notification
                    .strip_prefix("duplicate = ")
                    .and_then(|value| value.trim().parse().ok())
                {
                    counters.duplicated = duplicated;
                }
            }
        }
    }
    counters
}

/// Duration of a finished container in seconds, read from its header by ffprobe so a
/// long lossless recording is never decoded again. None when it cannot be read.
fn container_duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()
        .filter(|seconds: &f64| seconds.is_finite() && *seconds >= 0.0)
}

pub struct Recording {
    child: Child,
    prepared: Prepared,
    stage: Option<tempfile::TempDir>,
    raw: PathBuf,
    log: PathBuf,
    /// Frame counters the recorder writes while running: FFmpeg's progress file, or
    /// the stdout of a verbose gst-launch.
    statistics: PathBuf,
    frame_rate: u32,
    destination: PathBuf,
    pub preset: Preset,
    /// Recorded frame size from the preset policy; the export scale is a no-op.
    size: (i32, i32),
    /// The recorder writes the final MP4 itself; Stop only finalizes and moves it.
    direct: bool,
    stopped: bool,
    started: Instant,
}

impl Recording {
    /// Records at frame_rate frames per second, normally the display's refresh rate,
    /// so motion is as smooth in the file as it was on screen. Compressed presets are
    /// scaled to the preset size while recording. With a working VA-API encoder they
    /// are encoded straight into the final H.264/AAC MP4; otherwise they are recorded
    /// as lossless FFV1 at that size and exported afterwards. Full quality always
    /// records native RGB with FFV1 and FLAC, so nothing lossy touches its output.
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
        let log = stage.path().join("capture.log");
        let statistics = stage.path().join("frames.log");
        let monitor = if sound { Some(desktop_monitor()) } else { None };
        let (video, audio, _) = preset.encoding();
        let size = preset.output_size(area.width, area.height);
        let compressed = preset != Preset::Lossless;
        let hardware = hardware_encoders();
        let x11 = matches!(prepared.source, Source::X11 { .. });
        let direct = compressed
            && size.0 >= HARDWARE_MINIMUM_SIZE
            && size.1 >= HARDWARE_MINIMUM_SIZE
            && if x11 {
                hardware.ffmpeg_device.is_some()
            } else {
                hardware.gstreamer
            };
        let raw = stage.path().join(if direct {
            "recording.mp4"
        } else {
            "recording.mkv"
        });
        let connection = match &prepared.source {
            Source::Wayland(_) | Source::Gnome(_) => Some(stream_connection(&prepared.source)?),
            _ => None,
        };
        let mut command = match &prepared.source {
            Source::X11 { display } => {
                let mut command = Command::new("ffmpeg");
                command
                    .args(["-hide_banner", "-loglevel", "error", "-y", "-progress"])
                    .arg(&statistics);
                if direct {
                    command
                        .arg("-vaapi_device")
                        .arg(hardware.ffmpeg_device.as_ref().expect("probed render node"));
                }
                command
                    .args(["-thread_queue_size", "512", "-f", "x11grab", "-framerate"])
                    .arg(frame_rate.to_string())
                    .arg("-video_size")
                    .arg(format!("{}x{}", area.width, area.height))
                    .arg("-i")
                    .arg(format!("{display}+{},{}", area.x, area.y));
                if let Some(monitor) = &monitor {
                    // A deep input queue keeps audio flowing while video frames are encoded.
                    command.args(["-thread_queue_size", "4096", "-f", "pulse", "-i", monitor]);
                }
                if direct {
                    command
                        .args(["-vf", &ffmpeg_hardware_filter(size.0, size.1)])
                        .args(ffmpeg_h264(video, true))
                        .args(ffmpeg_aac_mp4(audio));
                } else {
                    if compressed {
                        command.args(["-vf", &format!("scale={}:{}", size.0, size.1)]);
                    }
                    command.args([
                        "-c:v", "ffv1", "-level", "3", "-pix_fmt", "bgr0", "-c:a", "flac",
                    ]);
                }
                command.arg(&raw);
                command
            }
            Source::Wayland(_) | Source::Gnome(_) => {
                let (node, remote) = connection.as_ref().expect("native stream connection");
                let mut command = pipewire_command(*node, remote.as_ref(), true);
                // The compositor only sends a frame when the screen changed, so the
                // source repeats its last frame every half period: videorate then
                // repeats a frame only when this pipeline itself failed to take the
                // frames in time, and that count is what the saved popup reports.
                command
                    .arg(format!("keepalive-time={}", (500 / frame_rate).max(1)))
                    .args(["!", "videoconvert", "!", "videocrop"])
                    .arg(format!("left={}", area.x))
                    .arg(format!("top={}", area.y))
                    .arg(format!("right={}", bounds.width - area.x - area.width))
                    .arg(format!("bottom={}", bounds.height - area.y - area.height))
                    // A bounded queue keeps the PipeWire thread free while frames encode.
                    .args([
                        "!",
                        "queue",
                        "max-size-buffers=8",
                        "max-size-bytes=0",
                        "max-size-time=0",
                        "!",
                        "videorate",
                        "silent=false",
                        "!",
                    ])
                    .arg(format!("video/x-raw,format=BGRx,framerate={frame_rate}/1"));
                if direct {
                    command
                        .arg("!")
                        .args(gst_hardware_video(video, size.0, size.1));
                } else {
                    if compressed {
                        // Scaling on every core keeps a 4K source at the display rate.
                        command
                            .args(["!", "videoscale", "n-threads=0", "!"])
                            .arg(format!("video/x-raw,width={},height={}", size.0, size.1));
                    }
                    // gst-libav's FFV1 encoder runs on one thread unless slices, the slice
                    // thread type and threads=auto are all set; together they keep 4K at
                    // the display rate within real time on a desktop CPU.
                    command.args([
                        "!",
                        "avenc_ffv1",
                        "slices=16",
                        "thread-type=slice",
                        "threads=auto",
                    ]);
                }
                command.args(["!", "queue", "!", "mux."]);
                if direct {
                    // The faststart copy is staged beside the recording, never in /tmp,
                    // where a long capture could exhaust memory-backed storage.
                    command
                        .args(["mp4mux", "name=mux", "faststart=true"])
                        .arg(gst_property(
                            "faststart-file",
                            &stage.path().join("faststart.tmp").to_string_lossy(),
                        ));
                } else {
                    command.args(["matroskamux", "name=mux"]);
                }
                command
                    .args(["!", "filesink"])
                    .arg(gst_property("location", &raw.to_string_lossy()));
                if let Some(monitor) = &monitor {
                    // Pin the desktop's native stereo 48 kHz format: left to negotiation,
                    // the encoder's template fixates to mono 44.1 kHz. The audio queue is
                    // unbounded (a few hundred KB per second) so the muxer, which waits for
                    // the slower video branch, can never block the PulseAudio reader and
                    // make it drop samples while the screen is static.
                    command
                        .arg("pulsesrc")
                        .arg(gst_property("device", monitor))
                        .args([
                            "do-timestamp=true",
                            "!",
                            "audio/x-raw,rate=48000,channels=2",
                            "!",
                            "queue",
                            "max-size-time=0",
                            "max-size-buffers=0",
                            "max-size-bytes=0",
                            "!",
                            "audioconvert",
                            "!",
                            "audioresample",
                            "!",
                        ]);
                    if direct {
                        command.args(gst_aac(audio));
                    } else {
                        command.arg("flacenc");
                    }
                    command.args(["!", "queue", "!", "mux."]);
                }
                command
            }
            Source::Screenshot => return Err("A screenshot is not a recording source".into()),
        };
        eprintln!(
            "Snappy recording: {} {}x{} at {frame_rate} fps, {}",
            if x11 { "x11" } else { "native" },
            size.0,
            size.1,
            if direct {
                "VA-API H.264 straight to MP4"
            } else if compressed {
                "FFV1 then H.264 export"
            } else {
                "lossless FFV1"
            }
        );
        let stderr = File::create(&log).map_err(|e| e.to_string())?;
        let stdout = File::create(&statistics).map_err(|e| e.to_string())?;
        let child = command
            .stdin(Stdio::piped())
            .stdout(stdout)
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
            statistics,
            frame_rate,
            destination,
            preset,
            size,
            direct,
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

    /// Finalizes the container, then exports or moves it. The worker owns the recording
    /// until complete. The frame statistics are read from the finalized container and
    /// the recorder's counters before any export, so they describe what was captured.
    pub fn finish(mut self) -> Result<Saved> {
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
        let timeout = self.finalization_timeout();
        let status = wait_bounded(&mut self.child, timeout);
        if status
            .as_ref()
            .map(|s| !s.success() && s.code() != Some(130))
            .unwrap_or(true)
        {
            return Err(self.recover(format!("Recording failed: {}", files::log_tail(&self.log))));
        }
        let frames = self.frame_statistics();
        let result = if self.direct || self.preset == Preset::Lossless {
            self.publish()
        } else {
            self.export()
        };
        if let Err(error) = result {
            return Err(self.recover(error));
        }
        Ok(Saved {
            path: self.destination.clone(),
            frames,
        })
    }

    /// Frames the finished recording should hold and how many were not captured.
    /// Missing counters or an unreadable container report a complete recording
    /// rather than inventing a number; the journal names the counts for diagnosis.
    fn frame_statistics(&self) -> FrameStatistics {
        let counters = File::open(&self.statistics)
            .map(|file| recorder_counters(BufReader::new(file)))
            .unwrap_or_default();
        let frames = container_duration(&self.raw)
            .map(|seconds| FrameStatistics::new(&counters, seconds, self.frame_rate))
            .unwrap_or_default();
        eprintln!(
            "Snappy recording: {} of {} frames dropped",
            frames.dropped, frames.expected
        );
        frames
    }

    /// Finalizing an MP4 for early playback rewrites it once, so the wait grows with
    /// the staged size at a conservative 50 MB/s on top of the usual 20 s.
    fn finalization_timeout(&self) -> Duration {
        let staged: u64 = self
            .stage
            .as_ref()
            .and_then(|stage| std::fs::read_dir(stage.path()).ok())
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.metadata().ok())
                    .map(|metadata| metadata.len())
                    .sum()
            })
            .unwrap_or(0);
        Duration::from_secs(20 + staged / 50_000_000)
    }

    /// Moves the finished file to its reserved name; the stage shares the folder's
    /// file system, so this is a rename, with a copy only if that is refused.
    fn publish(&self) -> Result<()> {
        if std::fs::rename(&self.raw, &self.destination).is_ok() {
            return Ok(());
        }
        std::fs::copy(&self.raw, &self.destination)
            .map(|_| ())
            .map_err(|error| format!("Cannot save recording: {error}"))
    }

    /// Software fallback: the staged FFV1 file already has the preset size, so the
    /// shared scale is a no-op and only the H.264/AAC encode remains.
    fn export(&mut self) -> Result<()> {
        let (video, audio, _) = self.preset.encoding();
        let stderr = File::create(&self.log).map_err(|e| e.to_string())?;
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&self.raw)
            .args(["-map", "0:v:0", "-map", "0:a?", "-vf"])
            .arg(format!("scale={}:{}", self.size.0, self.size.1))
            .args(ffmpeg_h264(video, false))
            .args(ffmpeg_aac_mp4(audio))
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

    #[test]
    fn recorder_counters_take_the_last_ffmpeg_block_and_videorate_notification() {
        let ffmpeg = "frame=12\nfps=60\ndup_frames=0\nprogress=continue\nframe=39\n\
            fps=2.37\nout_time_us=5350000\ndup_frames=2\ndrop_frames=7\nprogress=end\n";
        let counters = recorder_counters(std::io::Cursor::new(ffmpeg));
        assert_eq!(
            counters,
            RecorderCounters {
                encoded: Some(39),
                duplicated: 2,
            }
        );
        // Received frames are counted; the file holds 321 slots over 5.35 s at 60 fps.
        assert_eq!(
            FrameStatistics::new(&counters, 5.35, 60),
            FrameStatistics {
                expected: 321,
                dropped: 284,
            }
        );
        let gstreamer = "/GstPipeline:pipeline0/GstPipeWireSrc:pipewiresrc0.GstPad:src: caps = x\n\
            /GstPipeline:pipeline0/GstVideoRate:videorate0: drop = 4\n\
            /GstPipeline:pipeline0/GstVideoRate:videorate0: duplicate = 305\n\
            /GstPipeline:pipeline0/GstVideoRate:videorate0: duplicate = garbage\n\
            frame=notanumber\n";
        let counters = recorder_counters(std::io::Cursor::new(gstreamer));
        assert_eq!(
            counters,
            RecorderCounters {
                encoded: None,
                duplicated: 305,
            }
        );
        assert_eq!(
            FrameStatistics::new(&counters, 5.5, 60),
            FrameStatistics {
                expected: 330,
                dropped: 305,
            }
        );
        assert_eq!(
            recorder_counters(std::io::Cursor::new("")),
            RecorderCounters::default()
        );
    }
}
