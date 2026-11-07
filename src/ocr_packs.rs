//! Module: ocr_packs. Purpose: verified language packs in Snappy-owned local and shared stores.
//! Created: 2026-11-06. Architecture: unprivileged downloads feed a narrowly scoped privileged installer.
use crate::{files, files::Result, i18n::Language, ocr};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const REVISION: &str = "87416418657359cb625c412a48b6e1d6d41c29bd";
const MAX_PACK_BYTES: u64 = 20 * 1024 * 1024;
const SYSTEM_DIRECTORY: &str = "/var/lib/snappy/ocr/tessdata";
const LICENSE: &[u8] = include_bytes!("../assets/ocr-models-license.txt");

#[derive(Clone, Copy)]
pub struct Pack {
    pub language: Language,
    sha256: &'static str,
}

impl Pack {
    pub const ALL: [Self; 4] = [
        Self {
            language: Language::English,
            sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
        },
        Self {
            language: Language::Spanish,
            sha256: "6f2e04d02774a18f01bed44b1111f2cd7f3ba7ac9dc4373cd3f898a40ea6b464",
        },
        Self {
            language: Language::Greek,
            sha256: "4fba8a0b461038d51f1c20d043d4f2ac38c4e778f1b90830847f7bd8fa3ba726",
        },
        Self {
            language: Language::PortugueseBrazil,
            sha256: "c4932b937207a9514b7514d518b931a99938c02a28a5a5a553f8599ed58b7deb",
        },
    ];

    fn code(self) -> &'static str {
        ocr::model(self.language)
    }
    fn filename(self) -> String {
        format!("{}.traineddata", self.code())
    }

    fn verify(self, bytes: &[u8]) -> Result<()> {
        let digest = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, bytes);
        if digest.as_deref() != Some(self.sha256) {
            return Err("Language pack checksum mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Local,
    System,
}

impl Scope {
    pub fn directory(self) -> PathBuf {
        match self {
            Self::Local => glib::user_data_dir().join("snappy/ocr/tessdata"),
            Self::System => PathBuf::from(SYSTEM_DIRECTORY),
        }
    }
}

pub fn installed(pack: Pack, scope: Scope) -> bool {
    scope.directory().join(pack.filename()).is_file()
}

/// Only the child command receives this override. Global TESSDATA_PREFIX and
/// Tesseract's own directories are never modified. Local packs take precedence.
pub fn model_directory(language: Language) -> Option<PathBuf> {
    [Scope::Local.directory(), Scope::System.directory()]
        .into_iter()
        .find(|directory| {
            directory
                .join(format!("{}.traineddata", ocr::model(language)))
                .is_file()
        })
}

/// Probe only on a worker. Older system packages must not be launched as root:
/// their startup predates the privileged entry point and may initialize GTK.
pub fn can_install_system() -> bool {
    if glib::find_program_in_path("pkexec").is_none()
        || !fs::metadata("/usr/bin/snappy").is_ok_and(|metadata| {
            metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0
        })
    {
        return false;
    }
    let Ok(scratch) = tempfile::Builder::new()
        .prefix("snappy-pack-probe-")
        .tempdir()
    else {
        return false;
    };
    let mut command = Command::new("/usr/bin/snappy");
    command.arg("--help");
    files::command_output(
        command,
        Stdio::null(),
        scratch.path(),
        Duration::from_secs(5),
        || false,
    )
    .is_ok_and(|text| text.contains("--install-ocr-pack MODEL"))
}

/// Downloads occur as the desktop user. Only verified model bytes are offered to
/// the installed, root-owned Snappy executable through stdin after authorization.
pub fn install(
    pack: Pack,
    scope: Scope,
    cancelled: &AtomicBool,
    authorizing: impl Fn(),
) -> Result<()> {
    let operation = || -> Result<()> {
        let scratch = tempfile::Builder::new()
            .prefix("snappy-pack-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let input = scratch.path().join(pack.filename());
        let curl = glib::find_program_in_path("curl")
            .ok_or("curl is required to download language packs")?;
        let mut download = Command::new(curl);
        download
            .args([
                "--disable",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "10",
                "--max-time",
                "60",
                "--max-filesize",
                "20971520",
                "--output",
            ])
            .arg(&input)
            .arg(format!(
                "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{REVISION}/{}",
                pack.filename()
            ));
        files::command_output(
            download,
            Stdio::null(),
            scratch.path(),
            Duration::from_secs(65),
            || cancelled.load(Ordering::Relaxed),
        )
        .map_err(command_error)?;
        let bytes = read_pack(File::open(&input).map_err(|e| e.to_string())?)?;
        pack.verify(&bytes)?;
        if cancelled.load(Ordering::Relaxed) {
            return Err("Language pack installation cancelled".into());
        }
        match scope {
            Scope::Local => install_verified(&Scope::Local.directory(), pack, &bytes, false),
            Scope::System => {
                if !can_install_system() {
                    return Err(
                        "Install the Snappy package to enable system-wide language packs".into(),
                    );
                }
                authorizing();
                let pkexec = glib::find_program_in_path("pkexec")
                    .ok_or("Administrator authentication is unavailable")?;
                let mut helper = Command::new(pkexec);
                helper.args(["/usr/bin/snappy", "--install-ocr-pack", pack.code()]);
                files::command_output(
                    helper,
                    File::open(input).map_err(|e| e.to_string())?.into(),
                    scratch.path(),
                    Duration::from_secs(120),
                    || cancelled.load(Ordering::Relaxed),
                )
                .map(|_| ())
                .map_err(command_error)
            }
        }
    };
    operation().map_err(|error| format!("Language pack installation failed: {error}"))
}

fn command_error(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::TimedOut => "Language pack installation timed out".into(),
        std::io::ErrorKind::Interrupted => "Language pack installation cancelled".into(),
        _ => error.to_string(),
    }
}

fn read_pack(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_PACK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PACK_BYTES {
        return Err("Language pack exceeds the size limit".into());
    }
    Ok(bytes)
}

/// Privileged CLI boundary: fixed model whitelist, pinned checksum, fixed destination.
/// This path runs before GTK or user preferences are loaded and accepts no file paths.
pub fn install_system_from_stdin(code: &str) -> Result<()> {
    // SAFETY: geteuid reads the process identity and has no pointer arguments.
    if unsafe { libc::geteuid() } != 0 {
        return Err("Administrator privileges are required".into());
    }
    let pack = Pack::ALL
        .into_iter()
        .find(|pack| pack.code() == code)
        .ok_or("Unsupported OCR language")?;
    let bytes = read_pack(std::io::stdin().lock())?;
    install_verified(Path::new(SYSTEM_DIRECTORY), pack, &bytes, true)
}

fn install_verified(directory: &Path, pack: Pack, bytes: &[u8], system: bool) -> Result<()> {
    pack.verify(bytes)?;
    // Validate each app-owned component rather than following an existing link
    // into a different installation. Shared directories must be root-owned.
    let app = directory
        .parent()
        .and_then(Path::parent)
        .ok_or("Invalid language pack directory")?;
    if !system {
        fs::create_dir_all(app.parent().ok_or("Invalid language pack directory")?)
            .map_err(|e| e.to_string())?;
    }
    for path in [app.to_owned(), app.join("ocr"), directory.to_owned()] {
        match fs::DirBuilder::new().mode(0o755).create(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.to_string()),
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || (system && (metadata.uid() != 0 || metadata.mode() & 0o022 != 0))
        {
            return Err("Unsafe language pack directory".into());
        }
    }
    write_new(&directory.join("LICENSE"), LICENSE)?;
    write_new(&directory.join(pack.filename()), bytes)
}

/// Atomic, readable by all users for the shared store; never overwrites existing
/// files or symlink targets, even if another Snappy instance installs the same pack.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("Invalid language pack directory")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o644))
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                && fs::read(path).map_err(|e| e.to_string())? == bytes
            {
                Ok(())
            } else {
                Err("An existing language pack was left unchanged".into())
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_bytes_and_never_overwrites_existing_files_or_links() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("snappy/ocr/tessdata");
        assert!(install_verified(&directory, Pack::ALL[0], b"invalid model", false).is_err());
        assert!(!directory.exists());
        let other = root.path().join("other-installation");
        fs::create_dir(&other).unwrap();
        let target = other.join("eng.traineddata");
        fs::write(&target, b"existing model").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_new(&link, b"replacement").is_err());
        assert!(write_new(&target, b"replacement").is_err());
        assert_eq!(fs::read(target).unwrap(), b"existing model");
        assert!(write_new(&root.path().join("new-model"), b"model").is_ok());
        assert!(write_new(&root.path().join("new-model"), b"model").is_ok());
    }
}
