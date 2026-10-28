//! The macOS Trash: moving files in, and back out again for undo.

use std::path::{Component, Path, PathBuf};

/// Moves `p` to the Trash and returns where it landed there, so undo can
/// put it back. `home` is unused here; tests use it for a stand-in Trash.
#[cfg(all(target_os = "macos", not(test)))]
pub fn to_trash(p: &Path, _home: &Path) -> Result<PathBuf, String> {
    use objc2::rc::Retained;
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let name = crate::fsx::file_name(p);
    let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
    let mut landed: Option<Retained<NSURL>> = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut landed))
        .map_err(|e| format!("Couldn't move {name} to the Trash: {}", e.localizedDescription()))?;
    landed
        .and_then(|u| u.path())
        .map(|s| PathBuf::from(s.to_string()))
        .ok_or_else(|| format!("Moved {name} to the Trash, but macOS didn't say where."))
}

/// Tests use `<home>/.Trash` inside their temp folder, never the real Trash.
#[cfg(test)]
pub fn to_trash(p: &Path, home: &Path) -> Result<PathBuf, String> {
    let bin = home.join(".Trash");
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let to = crate::fsx::unique_path(&bin, &crate::fsx::file_name(p), &mut Default::default());
    std::fs::rename(p, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

/// True for a path inside a Trash folder (the home Trash, or a volume's).
pub fn is_in_trash(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::Normal(n) if n == ".Trash" || n == ".Trashes"))
}
