//! Module: ocr. Purpose: optional, offline text recognition from captured pixels.
//! Created: 2026-11-06. Architecture: workers reuse screenshot encoding and bounded child ownership.
use crate::{
    files::{self, Result},
    i18n::Language,
    image_export::{self, Snapshot},
};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Clone)]
pub struct Engine {
    executable: PathBuf,
}

impl Engine {
    /// Discovery does not start a process or add work to ordinary screen capture.
    pub fn find() -> Option<Self> {
        glib::find_program_in_path("tesseract").map(|executable| Self { executable })
    }

    /// Only languages with locally installed models are offered. Both Portuguese
    /// interface variants use Tesseract's shared Portuguese recognition model.
    pub fn languages(&self, cancelled: &AtomicBool) -> Result<Vec<Language>> {
        let scratch = scratch()?;
        let mut command = Command::new(&self.executable);
        command.arg("--list-langs");
        let output = run(command, scratch.path(), Duration::from_secs(10), cancelled);
        let installed = |language| crate::ocr_packs::model_directory(language).is_some();
        if output.is_err() && !Language::ALL.into_iter().any(installed) {
            return output.map(|_| Vec::new());
        }
        Ok(Language::ALL
            .into_iter()
            .filter(|language| {
                installed(*language)
                    || output
                        .as_ref()
                        .is_ok_and(|text| text.lines().any(|line| line.trim() == model(*language)))
            })
            .collect())
    }

    /// Uses the original cropped pixels, independently of the image export preset.
    /// Sparse-text segmentation accommodates scattered labels in screenshots.
    pub fn recognize(
        &self,
        snapshot: Snapshot,
        language: Language,
        cancelled: &AtomicBool,
    ) -> Result<String> {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Text recognition cancelled".into());
        }
        let scratch = scratch()?;
        let input = scratch.path().join("image.png");
        std::fs::write(&input, image_export::encode(snapshot, 0)?)
            .map_err(|error| format!("Text recognition failed: {error}"))?;
        let mut command = Command::new(&self.executable);
        command
            .arg(&input)
            .args(["stdout", "-l", model(language), "--psm", "11"])
            .env("OMP_THREAD_LIMIT", "2");
        if let Some(directory) = crate::ocr_packs::model_directory(language) {
            command.arg("--tessdata-dir").arg(directory);
        }
        run(command, scratch.path(), Duration::from_secs(60), cancelled)
            .map(|text| text.trim().to_owned())
    }
}

pub(crate) fn model(language: Language) -> &'static str {
    match language {
        Language::English => "eng",
        Language::Spanish => "spa",
        Language::Greek => "ell",
        Language::PortugueseBrazil | Language::PortuguesePortugal => "por",
    }
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
