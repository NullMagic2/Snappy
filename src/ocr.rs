//! Module: ocr. Purpose: optional, offline text recognition from captured pixels.
//! Created: 2026-11-06. Architecture: workers reuse screenshot encoding and bounded child ownership.
use crate::{
    files::{self, Result},
    image_export::{self, Snapshot},
};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone)]
pub struct Engine {
    executable: PathBuf,
}

#[derive(Clone)]
struct Model {
    code: String,
    file: PathBuf,
}

/// One resolved file per recognition model, with local Snappy packs preferred
/// over shared packs and the engine's native data. Each model is loaded once.
#[derive(Clone)]
pub struct Models {
    models: Vec<Model>,
}

impl Models {
    pub fn languages(&self) -> impl Iterator<Item = &str> + '_ {
        self.models.iter().map(|model| model.code.as_str())
    }

    /// None selects multilingual recognition. Temporary links let one child read
    /// models from different stores without copying or changing any installation.
    fn prepare(&self, directory: &Path, language: Option<&str>) -> Result<String> {
        let mut selected: Vec<_> = self
            .models
            .iter()
            .filter(|installed| language.is_none_or(|code| code == installed.code))
            .collect();
        // An English-first pass can discard accents before other models decode.
        selected.sort_by_key(|installed| {
            (
                match installed.code.as_str() {
                    "eng" => 3,
                    "spa" => 2,
                    "ell" => 1,
                    _ => 0,
                },
                installed.code.as_str(),
            )
        });
        let mut codes = Vec::new();
        for installed in selected {
            let code = installed.code.as_str();
            let target = directory.join(format!("{code}.traineddata"));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Text recognition failed: {error}"))?;
            }
            symlink(&installed.file, target)
                .map_err(|error| format!("Text recognition failed: {error}"))?;
            codes.push(code);
        }
        if codes.is_empty() {
            return Err("No supported OCR languages installed".into());
        }
        Ok(codes.join("+"))
    }
}

impl Engine {
    /// Discovery does not start a process or add work to ordinary screen capture.
    pub fn find() -> Option<Self> {
        glib::find_program_in_path("tesseract").map(|executable| Self { executable })
    }

    /// Resolve installed models on a worker, using Tesseract's effective native
    /// data directory as fallback. Detection-only data is not a text language.
    pub fn models(&self, cancelled: &AtomicBool) -> Result<Models> {
        let scratch = scratch()?;
        let mut command = Command::new(&self.executable);
        command.arg("--list-langs");
        let output = run(command, scratch.path(), Duration::from_secs(10), cancelled);
        if cancelled.load(Ordering::Relaxed) {
            return Err("Text recognition cancelled".into());
        }
        let native = output.as_ref().ok().and_then(|text| native_directory(text));
        let mut codes: BTreeSet<&str> = crate::ocr_packs::Pack::ALL
            .iter()
            .map(|pack| pack.code)
            .collect();
        if let Ok(text) = &output {
            codes.extend(
                text.lines()
                    .map(str::trim)
                    .filter(|code| valid_model_code(code)),
            );
        }
        let mut models = Vec::new();
        for code in codes
            .into_iter()
            .filter(|code| !["osd", "equ"].contains(code))
        {
            let directory = crate::ocr_packs::model_directory(code).or_else(|| {
                native
                    .as_ref()
                    .filter(|_| {
                        output
                            .as_ref()
                            .is_ok_and(|text| text.lines().any(|line| line.trim() == code))
                    })
                    .cloned()
            });
            if let Some(file) = directory
                .and_then(|directory| {
                    fs::canonicalize(directory.join(format!("{code}.traineddata"))).ok()
                })
                .filter(|file| file.is_file())
            {
                models.push(Model {
                    code: code.to_owned(),
                    file,
                });
            }
        }
        if models.is_empty() {
            output?;
        }
        Ok(Models { models })
    }

    /// Uses the original cropped pixels, independently of the image export preset.
    /// None recognizes all discovered languages together. Sparse-text segmentation
    /// accommodates scattered labels in screenshots; cancellation reaps the child.
    pub fn recognize(
        &self,
        snapshot: Snapshot,
        models: &Models,
        language: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<String> {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Text recognition cancelled".into());
        }
        let scratch = scratch()?;
        let codes = models.prepare(scratch.path(), language)?;
        let input = scratch.path().join("image.png");
        fs::write(&input, image_export::encode(snapshot, 0)?)
            .map_err(|error| format!("Text recognition failed: {error}"))?;
        let mut command = Command::new(&self.executable);
        command
            .arg(&input)
            .args(["stdout", "-l", &codes, "--psm", "11", "--tessdata-dir"])
            .arg(scratch.path())
            .env("OMP_THREAD_LIMIT", "2");
        run(command, scratch.path(), Duration::from_secs(60), cancelled)
            .map(|text| text.trim().to_owned())
    }
}

/// Tesseract reports its effective data directory before the model names.
/// The child runs in the C locale, and the final quote permits quotes in paths.
fn native_directory(listing: &str) -> Option<PathBuf> {
    listing.lines().find_map(|line| {
        let rest = line.strip_prefix("List of available languages in \"")?;
        let (directory, _) = rest.rsplit_once("\" (")?;
        Some(PathBuf::from(directory))
    })
}

/// Model identifiers may contain one script namespace, never arbitrary paths.
pub(crate) fn valid_model_code(code: &str) -> bool {
    let name = code.strip_prefix("script/").unwrap_or(code);
    !name.is_empty()
        && name.len() <= 80
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn scratch() -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("snappy-ocr-")
        .tempdir()
        .map_err(|error| format!("Text recognition failed: {error}"))
}

/// Private files avoid pipe deadlocks and bound diagnostics and returned text.
/// Closing the text dialog cancels the worker and reaps its Tesseract process.
fn run(
    command: Command,
    scratch: &Path,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<String> {
    files::command_output(command, Stdio::null(), scratch, timeout, || {
        cancelled.load(Ordering::Relaxed)
    })
    .map_err(|error| match error.kind() {
        std::io::ErrorKind::TimedOut => "Text recognition timed out".into(),
        std::io::ErrorKind::Interrupted => "Text recognition cancelled".into(),
        _ => format!("Text recognition failed: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_and_manual_recognition_share_isolated_models() {
        let source = tempfile::tempdir().unwrap();
        let mut models = Models { models: Vec::new() };
        for code in ["eng", "ell", "por", "script/Latin"] {
            let store = source.path().join(code.replace('/', "_"));
            fs::create_dir(&store).unwrap();
            let file = store.join(format!("{}.traineddata", code.replace('/', "_")));
            fs::write(&file, code).unwrap();
            models.models.push(Model {
                code: code.to_owned(),
                file,
            });
        }
        let automatic = tempfile::tempdir().unwrap();
        assert_eq!(
            models.prepare(automatic.path(), None).unwrap(),
            "por+script/Latin+ell+eng"
        );
        let manual = tempfile::tempdir().unwrap();
        assert_eq!(models.prepare(manual.path(), Some("por")).unwrap(), "por");
        assert_eq!(fs::read_dir(manual.path()).unwrap().count(), 1);
        assert_eq!(models.languages().count(), 4);
        for installed in models.models {
            let code = installed.code;
            let link = automatic.path().join(format!("{code}.traineddata"));
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read_to_string(&link).unwrap(), code);
            assert_eq!(fs::read_to_string(&installed.file).unwrap(), code);
        }
        assert!(!valid_model_code("../eng"));
        assert!(!valid_model_code("script/../../eng"));
        let directory = source.path().join("native \"data\"");
        let listing = format!(
            "List of available languages in \"{}\" (1):\neng\n",
            directory.display()
        );
        assert_eq!(native_directory(&listing), Some(directory));
    }
}
