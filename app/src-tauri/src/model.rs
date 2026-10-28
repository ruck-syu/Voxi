//! Shared data types: filters, parsed intents, concrete actions, and what the
//! window receives back (plans, read-only results, errors).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileType {
    Screenshots,
    Images,
    Pdfs,
    Videos,
    Documents,
    Archives,
    Installers,
    Audio,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayRef {
    Today,
    Yesterday,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub file_type: Option<FileType>,
    /// Lowercase extensions without the dot, e.g. ["heic"].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub older_than_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newer_than_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<DayRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub larger_than_mb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smaller_than_mb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_contains: Option<String>,
}

impl Filter {
    /// True when anything narrows the set beyond "files in a folder".
    pub fn is_narrowing(&self) -> bool {
        !matches!(self.file_type, None | Some(FileType::Files))
            || self.ext.is_some()
            || self.older_than_days.is_some()
            || self.newer_than_days.is_some()
            || self.day.is_some()
            || self.larger_than_mb.is_some()
            || self.smaller_than_mb.is_some()
            || self.name_contains.is_some()
    }
}

/// What a command refers to, before it is resolved against the disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A single file or folder by name, optionally inside a named folder.
    Item { name: String, folder: Option<String> },
    /// Every file in a folder (or the default folders) matching a filter.
    Set { folder: Option<String>, filter: Filter },
}

impl Source {
    pub fn item(name: &str) -> Self {
        Source::Item { name: name.to_string(), folder: None }
    }
    #[cfg(test)]
    pub fn set(folder: Option<&str>, filter: Filter) -> Self {
        Source::Set { folder: folder.map(str::to_string), filter }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpg,
    Webp,
    Heic,
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Heic => "heic",
        }
    }
    /// The name `sips -s format` expects.
    pub fn sips_name(self) -> &'static str {
        match self {
            ImageFormat::Jpg => "jpeg",
            other => other.ext(),
        }
    }
}

/// The parser's output: what the user asked for, with names still unresolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    Move { src: Source, to: String },
    Copy { src: Source, to: String },
    Rename { src: Source, new_name: String },
    AddPrefix { src: Source, prefix: String },
    AddDatePrefix { src: Source },
    ReplaceInNames { src: Source, find: String, replace: String },
    Number { src: Source, base: String, width: usize },
    Lowercase { src: Source },
    CreateFolders { names: Vec<String>, parent: Option<String> },
    SortByType { folder: String },
    SortByMonth { folder: String },
    ConvertImage { src: Source, format: ImageFormat },
    ResizeImage { src: Source, width: u32 },
    MergePdf { src: Source, output: String },
    SplitPdf { src: Source },
    ExtractPages { src: Source, from: u32, to: u32, output: Option<String> },
    ImagesToPdf { src: Source, output: Option<String> },
    Zip { src: Source, output: Option<String> },
    Unzip { src: Source, destination: Option<String> },
    Trash { src: Source },
    List { src: Source },
    Size { folder: String },
    Count { src: Source },
    Duplicates { folder: String },
    Reveal { target: String },
    Undo,
    RecentActions,
}

/// A concrete, fully resolved step. Every path is absolute and was checked
/// against the allowed folders when the plan was built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Action {
    Move { from: PathBuf, to: PathBuf },
    Copy { from: PathBuf, to: PathBuf },
    Rename { path: PathBuf, new_name: String },
    /// Never a hard delete: moves `path` to the macOS Trash.
    Trash { path: PathBuf },
    CreateFolder { path: PathBuf },
    ConvertImage { path: PathBuf, format: ImageFormat, output: PathBuf },
    /// Resizes in place; the original is first copied to `backup`.
    ResizeImage { path: PathBuf, max_width_px: u32, backup: PathBuf },
    ImagesToPdf { paths: Vec<PathBuf>, output: PathBuf },
    MergePdf { paths: Vec<PathBuf>, output: PathBuf },
    /// Writes one PDF per page into `output_dir`.
    SplitPdf { path: PathBuf, output_dir: PathBuf },
    ExtractPages { path: PathBuf, from: u32, to: u32, output: PathBuf },
    Zip { path: PathBuf, output: PathBuf },
    Unzip { path: PathBuf, destination: PathBuf },
    OpenInFinder { path: PathBuf, reveal: bool },
    /// Undo-only: removes a folder Voxi created, and only if it is empty.
    RemoveEmptyFolder { path: PathBuf },
    /// Undo-only: takes a trashed item at `from` back out to `to`.
    RestoreFromTrash { from: PathBuf, to: PathBuf },
    /// Undo-only: puts a backed-up original back over a modified file.
    Restore { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub kind: String,
    pub primary: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoRow {
    pub name: String,
    pub detail: String,
}

/// What the window gets back after Enter.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Outcome {
    /// Needs confirmation before anything changes.
    Plan {
        id: u64,
        title: String,
        total: usize,
        /// The first 20 steps, for the preview.
        items: Vec<PreviewItem>,
        warnings: Vec<String>,
        is_undo: bool,
    },
    /// A name matched more than one file. The user picks one and the
    /// command is planned again with that choice.
    Ambiguous { name: String, candidates: Vec<String> },
    /// Read-only result; nothing changed.
    Info { title: String, summary: String, rows: Vec<InfoRow>, total: usize },
    /// The command was understood, but there is nothing to do or it can't be done.
    Problem { message: String },
    NotUnderstood { text: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub undoable: bool,
    pub summary: String,
}
