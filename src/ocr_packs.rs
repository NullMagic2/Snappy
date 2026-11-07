//! Module: ocr_packs. Purpose: verified language packs in Snappy-owned local and shared stores.
//! Created: 2026-11-06. Architecture: unprivileged downloads feed a narrowly scoped privileged installer.
use crate::{files, files::Result, i18n};
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
const SYSTEM_DIRECTORY: &str = "/var/lib/snappy/ocr/tessdata";
const LICENSE: &[u8] = include_bytes!("../assets/ocr-models-license.txt");

#[derive(Clone, Copy)]
pub struct Pack {
    pub code: &'static str,
    names: [&'static str; 5],
    size: u64,
    sha256: &'static str,
}

impl Pack {
    pub const ALL: &'static [Self] = include!("ocr_catalog.rs");

    pub fn find(code: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|pack| pack.code == code)
    }
    pub fn name(self) -> &'static str {
        self.names[i18n::language() as usize]
    }
    pub fn matches(self, query: &str) -> bool {
        self.code.to_lowercase().contains(query)
            || self
                .names
                .iter()
                .any(|name| name.to_lowercase().contains(query))
    }
    fn filename(self) -> String {
        format!("{}.traineddata", self.code)
    }

    fn verify(self, bytes: &[u8]) -> Result<()> {
        let digest = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, bytes);
        if bytes.len() as u64 != self.size || digest.as_deref() != Some(self.sha256) {
            return Err("Language pack checksum mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Remove,
}

impl Action {
    fn flag(self) -> &'static str {
        match self {
            Self::Install => "--install-ocr-pack",
            Self::Remove => "--remove-ocr-pack",
        }
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
pub fn model_directory(code: &str) -> Option<PathBuf> {
    [Scope::Local.directory(), Scope::System.directory()]
        .into_iter()
        .find(|directory| directory.join(format!("{code}.traineddata")).is_file())
}

/// Probe only on a worker. Older system packages must not be launched as root:
/// their startup predates the privileged entry point and may initialize GTK.
pub fn can_manage_system() -> bool {
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
    .is_ok_and(|text| {
        text.contains("--install-ocr-pack MODEL") && text.contains("--remove-ocr-pack MODEL")
    })
}

/// Downloads occur as the desktop user. Only verified model bytes are offered to
/// the installed, root-owned Snappy executable through stdin after authorization.
pub fn change(
    pack: Pack,
    scope: Scope,
    action: Action,
    cancelled: &AtomicBool,
    authorizing: impl Fn(),
) -> Result<()> {
    let operation = || -> Result<()> {
        let scratch = tempfile::Builder::new()
            .prefix("snappy-pack-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let input = scratch.path().join(pack.filename());
        if let Some(parent) = input.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let bytes = if action == Action::Install {
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
                    "180",
                    "--max-filesize",
                ])
                .arg(pack.size.to_string())
                .args(["--output"])
                .arg(&input)
                .arg(format!(
                    "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{REVISION}/{}",
                    pack.filename()
                ));
            files::command_output(
                download,
                Stdio::null(),
                scratch.path(),
                Duration::from_secs(185),
                || cancelled.load(Ordering::Relaxed),
            )
            .map_err(command_error)?;
            let bytes = read_pack(File::open(&input).map_err(|e| e.to_string())?, pack.size)?;
            pack.verify(&bytes)?;
            bytes
        } else {
            Vec::new()
        };
        if cancelled.load(Ordering::Relaxed) {
            return Err("Language pack operation cancelled".into());
        }
        match scope {
            Scope::Local => match action {
                Action::Install => install_verified(&scope.directory(), pack, &bytes, false),
                Action::Remove => remove_pack(&scope.directory(), pack, false),
            },
            Scope::System => {
                drop(bytes);
                if !can_manage_system() {
                    return Err(
                        "Install the Snappy package to enable system-wide language packs".into(),
                    );
                }
                authorizing();
                let pkexec = glib::find_program_in_path("pkexec")
                    .ok_or("Administrator authentication is unavailable")?;
                let mut helper = Command::new(pkexec);
                helper.args(["/usr/bin/snappy", action.flag(), pack.code]);
                let stdin = match action {
                    Action::Install => File::open(input).map_err(|e| e.to_string())?.into(),
                    Action::Remove => Stdio::null(),
                };
                files::command_output(
                    helper,
                    stdin,
                    scratch.path(),
                    Duration::from_secs(120),
                    || cancelled.load(Ordering::Relaxed),
                )
                .map(|_| ())
                .map_err(command_error)
            }
        }
    };
    operation().map_err(|error| match action {
        Action::Install => format!("Language pack installation failed: {error}"),
        Action::Remove => format!("Language pack removal failed: {error}"),
    })
}

fn command_error(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::TimedOut => "Language pack operation timed out".into(),
        std::io::ErrorKind::Interrupted => "Language pack operation cancelled".into(),
        _ => error.to_string(),
    }
}

fn read_pack(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Language pack exceeds the size limit".into());
    }
    Ok(bytes)
}

/// Privileged CLI boundary: fixed model whitelist, pinned checksum, fixed destination.
/// This path runs before GTK or user preferences are loaded and accepts no file paths.
pub fn change_system_from_stdin(code: &str, action: Action) -> Result<()> {
    // SAFETY: geteuid reads the process identity and has no pointer arguments.
    if unsafe { libc::geteuid() } != 0 {
        return Err("Administrator privileges are required".into());
    }
    let pack = Pack::find(code).ok_or("Unsupported OCR language")?;
    if action == Action::Remove {
        return remove_pack(Path::new(SYSTEM_DIRECTORY), pack, true);
    }
    let bytes = read_pack(std::io::stdin().lock(), pack.size)?;
    install_verified(Path::new(SYSTEM_DIRECTORY), pack, &bytes, true)
}

fn install_verified(directory: &Path, pack: Pack, bytes: &[u8], system: bool) -> Result<()> {
    pack.verify(bytes)?;
    validate_directory(directory, pack, system, true)?;
    write_new(&directory.join("LICENSE"), LICENSE)?;
    write_new(&directory.join(pack.filename()), bytes)
}

fn validate_directory(directory: &Path, pack: Pack, system: bool, create: bool) -> Result<()> {
    // Validate each app-owned component rather than following an existing link
    // into a different installation. Shared directories must be root-owned.
    let app = directory
        .parent()
        .and_then(Path::parent)
        .ok_or("Invalid language pack directory")?;
    if create && !system {
        fs::create_dir_all(app.parent().ok_or("Invalid language pack directory")?)
            .map_err(|e| e.to_string())?;
    }
    let mut paths = vec![app.to_owned(), app.join("ocr"), directory.to_owned()];
    if pack.code.starts_with("script/") {
        paths.push(directory.join("script"));
    }
    for path in paths {
        if create {
            match fs::DirBuilder::new().mode(0o755).create(&path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error.to_string()),
            }
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || (system && (metadata.uid() != 0 || metadata.mode() & 0o022 != 0))
        {
            return Err("Unsafe language pack directory".into());
        }
    }
    Ok(())
}

/// Only regular files in the selected Snappy store can be removed. Native
/// Tesseract data, the other scope and shared license files are never touched.
fn remove_pack(directory: &Path, pack: Pack, system: bool) -> Result<()> {
    validate_directory(directory, pack, system, false)?;
    let path = directory.join(pack.filename());
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || (system && metadata.uid() != 0) {
        return Err("Unsafe language pack file".into());
    }
    fs::remove_file(path).map_err(|e| e.to_string())
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
    fn removal_is_scoped_and_rejects_links() {
        let root = tempfile::tempdir().unwrap();
        let pack = Pack::find("fra").unwrap();
        let local = root.path().join("local/snappy/ocr/tessdata");
        let shared = root.path().join("shared/snappy/ocr/tessdata");
        for directory in [&local, &shared] {
            validate_directory(directory, pack, false, true).unwrap();
            fs::write(directory.join(pack.filename()), b"model").unwrap();
        }
        remove_pack(&local, pack, false).unwrap();
        assert!(!local.join(pack.filename()).exists());
        assert!(shared.join(pack.filename()).is_file());
        let link = local.join(pack.filename());
        std::os::unix::fs::symlink(shared.join(pack.filename()), &link).unwrap();
        assert!(remove_pack(&local, pack, false).is_err());
        assert_eq!(fs::read(shared.join(pack.filename())).unwrap(), b"model");
        assert!(Pack::find("../eng").is_none());
        assert!(Pack::find("script/../../eng").is_none());
        let mut codes = std::collections::BTreeSet::new();
        for pack in Pack::ALL {
            assert!(crate::ocr::valid_model_code(pack.code));
            assert!(codes.insert(pack.code));
            assert_eq!(pack.sha256.len(), 64);
            assert!(pack.size > 0 && pack.names.iter().all(|name| !name.is_empty()));
        }
    }

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
