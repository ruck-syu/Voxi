//! Shared data types: filters, parsed intents, concrete actions, and what the
//! window receives back (plans, read-only results, errors).

use chrono::NaiveDate;
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

/// Extensions that make up each file type, lowercase and without the dot. The
/// parser and the planner both read these, so they agree on what a type is.
pub const IMAGE_EXTS: &[&str] =
    &["png", "jpg", "jpeg", "heic", "heif", "gif", "webp", "tiff", "tif", "bmp", "svg", "raw", "cr2", "nef", "arw", "dng"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "m4v", "avi", "mkv", "webm", "wmv", "flv"];
pub const DOC_EXTS: &[&str] =
    &["doc", "docx", "txt", "rtf", "md", "pages", "odt", "xls", "xlsx", "csv", "numbers", "ppt", "pptx", "key", "odp", "epub"];
pub const ARCHIVE_EXTS: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"];
pub const INSTALLER_EXTS: &[&str] = &["dmg", "pkg", "mpkg"];
pub const AUDIO_EXTS: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "aiff", "ogg"];

impl FileType {
    /// Whether a file with this extension is of this type. Screenshots count as
    /// images here; the planner also checks their names.
    pub fn accepts_ext(self, ext: &str) -> bool {
        match self {
            FileType::Files => true,
            FileType::Screenshots | FileType::Images => IMAGE_EXTS.contains(&ext),
            FileType::Pdfs => ext == "pdf",
            FileType::Videos => VIDEO_EXTS.contains(&ext),
            FileType::Documents => DOC_EXTS.contains(&ext) || ext == "pdf",
            FileType::Archives => ARCHIVE_EXTS.contains(&ext),
            FileType::Installers => INSTALLER_EXTS.contains(&ext),
            FileType::Audio => AUDIO_EXTS.contains(&ext),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayRef {
    Today,
    Yesterday,
}

/// Which date a date filter reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DateField {
    /// When the file was created, or modified if the creation date is unknown.
    Created,
    /// When the file was last modified: "not modified in a year".
    Modified,
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
    /// A calendar month, 1 to 12 ("in August").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month: Option<u32>,
    /// The month's year; without one, the most recent such month.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    /// Created on or after this day: "since March 2026", "after July 2026"
    /// (which starts the day after July ends).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<NaiveDate>,
    /// Created before this day: "before July 2026", "until March 15" (which
    /// stops the day after March 15).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<NaiveDate>,
    /// "Including subfolders": moves, copies and deletes also reach files in
    /// the named folder's subfolders, instead of only those directly in it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_subfolders: bool,
    /// "From subfolders": with `include_subfolders`, reach only files in the
    /// named folder's subfolders, leaving those directly in it alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub only_subfolders: bool,
    /// The date the date filters read. None reads the creation date, like
    /// `Created`, but records that the command didn't say which.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_field: Option<DateField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub larger_than_mb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smaller_than_mb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_contains: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_starts_with: Option<String>,
}

impl Filter {
    /// True when anything narrows the set beyond "files in a folder".
    pub fn is_narrowing(&self) -> bool {
        !matches!(self.file_type, None | Some(FileType::Files))
            || self.ext.is_some()
            || self.older_than_days.is_some()
            || self.newer_than_days.is_some()
            || self.day.is_some()
            || self.month.is_some()
            || self.since.is_some()
            || self.before.is_some()
            || self.larger_than_mb.is_some()
            || self.smaller_than_mb.is_some()
            || self.name_contains.is_some()
            || self.name_starts_with.is_some()
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

/// A path or a folder inside an existing parent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Destination {
    Path(String),
    InFolder {
        folder: String,
        parent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        literal: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        existing_only: bool,
    },
}

impl Destination {
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Path(path) => Some(path),
            Self::InFolder { .. } => None,
        }
    }
}

impl From<String> for Destination {
    fn from(path: String) -> Self {
        Self::Path(path)
    }
}

impl From<&str> for Destination {
    fn from(path: &str) -> Self {
        path.to_string().into()
    }
}

/// The parser's output: what the user asked for, with names still unresolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    Chain {
        commands: Vec<Intent>,
    },
    Move {
        src: Source,
        to: Destination,
    },
    Copy {
        src: Source,
        to: Destination,
    },
    Rename {
        src: Source,
        new_name: String,
    },
    AddPrefix {
        src: Source,
        prefix: String,
    },
    AddDatePrefix {
        src: Source,
    },
    ReplaceInNames {
        src: Source,
        find: String,
        replace: String,
    },
    Number {
        src: Source,
        base: String,
        width: usize,
    },
    Lowercase {
        src: Source,
    },
    CreateFolders {
        names: Vec<String>,
        parent: Option<String>,
    },
    SortByType {
        folder: Destination,
        /// Only "including subfolders" reaches into nested folders; by default
        /// a sort leaves the user's existing subfolders untouched.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        include_subfolders: bool,
    },
    SortByMonth {
        folder: Destination,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        include_subfolders: bool,
    },
    ConvertImage {
        src: Source,
        format: ImageFormat,
    },
    ResizeImage {
        src: Source,
        width: u32,
    },
    MergePdf {
        src: Source,
        output: String,
    },
    SplitPdf {
        src: Source,
    },
    ExtractPages {
        src: Source,
        from: u32,
        to: u32,
        output: Option<String>,
    },
    ImagesToPdf {
        src: Source,
        output: Option<String>,
    },
    Zip {
        src: Source,
        output: Option<String>,
    },
    Unzip {
        src: Source,
        destination: Option<String>,
    },
    Trash {
        src: Source,
    },
    List {
        src: Source,
    },
    Size {
        folder: String,
    },
    Count {
        src: Source,
    },
    Duplicates {
        folder: String,
    },
    Reveal {
        target: String,
    },
    Undo,
    RecentActions,
}

impl Intent {
    pub fn source(&self) -> Option<&Source> {
        match self {
            Self::Move { src, .. }
            | Self::Copy { src, .. }
            | Self::Rename { src, .. }
            | Self::AddPrefix { src, .. }
            | Self::AddDatePrefix { src }
            | Self::ReplaceInNames { src, .. }
            | Self::Number { src, .. }
            | Self::Lowercase { src }
            | Self::ConvertImage { src, .. }
            | Self::ResizeImage { src, .. }
            | Self::MergePdf { src, .. }
            | Self::SplitPdf { src }
            | Self::ExtractPages { src, .. }
            | Self::ImagesToPdf { src, .. }
            | Self::Zip { src, .. }
            | Self::Unzip { src, .. }
            | Self::Trash { src }
            | Self::List { src }
            | Self::Count { src } => Some(src),
            _ => None,
        }
    }
    pub fn replace_source(&mut self, source: Source) {
        match self {
            Self::Move { src, .. }
            | Self::Copy { src, .. }
            | Self::Rename { src, .. }
            | Self::AddPrefix { src, .. }
            | Self::AddDatePrefix { src }
            | Self::ReplaceInNames { src, .. }
            | Self::Number { src, .. }
            | Self::Lowercase { src }
            | Self::ConvertImage { src, .. }
            | Self::ResizeImage { src, .. }
            | Self::MergePdf { src, .. }
            | Self::SplitPdf { src }
            | Self::ExtractPages { src, .. }
            | Self::ImagesToPdf { src, .. }
            | Self::Zip { src, .. }
            | Self::Unzip { src, .. }
            | Self::Trash { src }
            | Self::List { src }
            | Self::Count { src } => *src = source,
            _ => {}
        }
    }
}

/// A concrete, fully resolved step. Every path is absolute and was checked
/// against the allowed folders when the plan was built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Action {
    Move {
        from: PathBuf,
        to: PathBuf,
    },
    Copy {
        from: PathBuf,
        to: PathBuf,
    },
    Rename {
        path: PathBuf,
        new_name: String,
    },
    /// Never a hard delete: moves `path` to the macOS Trash.
    Trash {
        path: PathBuf,
    },
    CreateFolder {
        path: PathBuf,
    },
    ConvertImage {
        path: PathBuf,
        format: ImageFormat,
        output: PathBuf,
    },
    /// Resizes in place; the original is first copied to `backup`.
    ResizeImage {
        path: PathBuf,
        max_width_px: u32,
        backup: PathBuf,
    },
    ImagesToPdf {
        paths: Vec<PathBuf>,
        output: PathBuf,
    },
    MergePdf {
        paths: Vec<PathBuf>,
        output: PathBuf,
    },
    /// Writes one PDF per page into `output_dir`.
    SplitPdf {
        path: PathBuf,
        output_dir: PathBuf,
    },
    ExtractPages {
        path: PathBuf,
        from: u32,
        to: u32,
        output: PathBuf,
    },
    Zip {
        path: PathBuf,
        output: PathBuf,
    },
    Unzip {
        path: PathBuf,
        destination: PathBuf,
    },
    OpenInFinder {
        path: PathBuf,
        reveal: bool,
    },
    /// Undo-only: removes a folder Voxi created, and only if it is empty.
    RemoveEmptyFolder {
        path: PathBuf,
    },
    /// Undo-only: takes a trashed item at `from` back out to `to`.
    RestoreFromTrash {
        from: PathBuf,
        to: PathBuf,
    },
    /// Undo-only: puts a backed-up original back over a modified file.
    Restore {
        from: PathBuf,
        to: PathBuf,
    },
}

impl Action {
    /// Every path a step reads or writes.
    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Move { from, to } | Self::Copy { from, to } => vec![from.clone(), to.clone()],
            Self::Rename { path, new_name } => vec![path.clone(), path.with_file_name(new_name)],
            Self::Trash { path } | Self::CreateFolder { path } | Self::OpenInFinder { path, .. } | Self::RemoveEmptyFolder { path } => {
                vec![path.clone()]
            }
            Self::ConvertImage { path, output, .. }
            | Self::ExtractPages { path, output, .. }
            | Self::Zip { path, output }
            | Self::ResizeImage { path, backup: output, .. } => vec![path.clone(), output.clone()],
            Self::SplitPdf { path, output_dir } => vec![path.clone(), output_dir.clone()],
            Self::Unzip { path, destination } => vec![path.clone(), destination.clone()],
            Self::ImagesToPdf { paths, output } | Self::MergePdf { paths, output } => {
                paths.iter().cloned().chain(std::iter::once(output.clone())).collect()
            }
            Self::RestoreFromTrash { from, to } | Self::Restore { from, to } => vec![from.clone(), to.clone()],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub kind: String,
    pub primary: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// True when the user may leave this step out before confirming.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub skippable: bool,
}

/// A preview step loaded after the first page, with its 0-based position.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    pub step: usize,
    #[serde(flatten)]
    pub item: PreviewItem,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoRow {
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityReason {
    MultipleMatches,
    SuggestedMatch,
    IncompleteSearch,
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
        /// The first 20 steps, for the preview. The rest load page by page.
        items: Vec<PreviewItem>,
        /// How many steps carry a note, such as a name changed to avoid a clash.
        noted: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        scope_notice: Option<String>,
        warnings: Vec<String>,
        model_interpreted: bool,
        is_undo: bool,
        scheduled_for: Option<chrono::DateTime<chrono::Local>>,
        first_run: crate::scheduler::FirstRunPolicy,
        /// How often a repeating task runs, such as “every 3 days”.
        repeats: Option<String>,
    },
    /// A name needs a choice before the command can be planned again.
    /// `folders` is true when the pick can be saved as a folder nickname.
    Ambiguous {
        name: String,
        candidates: Vec<String>,
        folders: bool,
        reason: AmbiguityReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// Read-only result; nothing changed.
    Info {
        title: String,
        summary: String,
        rows: Vec<InfoRow>,
        total: usize,
        numbered: bool,
    },
    /// The command was understood, but there is nothing to do or it can't be done.
    Problem {
        message: String,
    },
    NotUnderstood {
        text: String,
    },
}

#[derive(Debug)]
pub(crate) enum RunOutcome {
    Completed,
    Cancelled,
    Failed { error: String, partial: bool },
    Scheduled,
    NeedsReview { error: String },
}

impl RunOutcome {
    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } | Self::NeedsReview { error } => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Cancelled,
    Failed,
    PartiallyFailed,
    Scheduled,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub status: RunStatus,
    pub entry_id: Option<u64>,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub cancelled: bool,
    pub undoable: bool,
    pub summary: String,
}

impl RunResult {
    pub(crate) fn new(terminal: RunOutcome, done: usize, total: usize, entry_id: Option<u64>, undoable: bool, summary: String) -> Self {
        let status = match terminal {
            RunOutcome::Completed => RunStatus::Completed,
            RunOutcome::Cancelled => RunStatus::Cancelled,
            RunOutcome::Failed { partial: false, .. } => RunStatus::Failed,
            RunOutcome::Failed { partial: true, .. } => RunStatus::PartiallyFailed,
            RunOutcome::Scheduled => RunStatus::Scheduled,
            RunOutcome::NeedsReview { .. } => RunStatus::NeedsReview,
        };
        Self {
            status,
            entry_id,
            done,
            total,
            error: terminal.error().map(str::to_owned),
            cancelled: matches!(status, RunStatus::Cancelled),
            undoable,
            summary,
        }
    }
}
