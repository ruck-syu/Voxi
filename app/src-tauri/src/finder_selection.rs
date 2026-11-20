//! "Organize with Voxi" in Finder: a macOS Service that hands Voxi the
//! selected files. They become the current results list, so "move these to
//! Receipts" acts on exactly that selection, checked again before it runs.

use crate::fsx::{self, Roots};
use std::path::PathBuf;

/// What the window says about a selection it couldn't take in full.
pub const LABEL: &str = "your Finder selection";

/// Keeps the selected files Voxi may touch. Folders and anything outside the
/// allowed folders (or inside an excluded one) are left out and named in the
/// returned notice; with nothing left, the notice is the error.
pub fn accept(roots: &Roots, selected: Vec<PathBuf>) -> Result<(Vec<PathBuf>, Option<String>), String> {
    let (mut files, mut folders, mut outside, mut links) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for p in selected {
        let name = fsx::file_name(&p);
        // A link would be swapped for the file it points to, which the user
        // didn't select: leave it out and say so.
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            links.push(name);
        } else if !roots.is_allowed(&p) {
            outside.push(name);
        } else if p.is_dir() {
            folders.push(name);
        } else if p.is_file() {
            let c = fsx::canonical(&p);
            if !files.contains(&c) {
                files.push(c);
            }
        } else {
            outside.push(name);
        }
    }
    let mut notes = Vec::new();
    if !outside.is_empty() {
        notes.push(format!("{} outside the folders Voxi may use, left out: {}.", count(outside.len()), names(&outside)));
    }
    if !folders.is_empty() {
        notes.push(format!("{} left out, since only files can be selected: {}.", folder_count(folders.len()), names(&folders)));
    }
    if !links.is_empty() {
        let what = if links.len() == 1 { "1 symbolic link".to_string() } else { format!("{} symbolic links", links.len()) };
        notes.push(format!("{what} left out; select the original instead: {}.", names(&links)));
    }
    let notice = (!notes.is_empty()).then(|| notes.join(" "));
    if files.is_empty() {
        return Err(notice.unwrap_or_else(|| "Nothing was selected in Finder.".into()));
    }
    Ok((files, notice))
}

fn count(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

fn folder_count(n: usize) -> String {
    if n == 1 { "1 folder".into() } else { format!("{n} folders") }
}

/// Up to three names, then how many more.
fn names(all: &[String]) -> String {
    let shown: Vec<String> = all.iter().take(3).map(|n| format!("“{n}”")).collect();
    match all.len().saturating_sub(3) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

#[cfg(target_os = "macos")]
pub mod service {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSPasteboard, NSPasteboardTypeFileURL, NSUpdateDynamicServices};
    use objc2_foundation::{NSString, NSURL};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use tauri::AppHandle;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    thread_local! {
        static PROVIDER: RefCell<Option<Retained<Provider>>> = const { RefCell::new(None) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Provider
        // doesn't implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "VoxiServiceProvider"]
        struct Provider;

        impl Provider {
            /// Named by NSMessage in Info.plist.
            #[unsafe(method(organizeWithVoxi:userData:error:))]
            fn organize(&self, pboard: &NSPasteboard, _data: Option<&NSString>, _error: *mut *mut NSString) {
                let paths = file_paths(pboard);
                if let Some(app) = APP.get() {
                    crate::use_finder_selection(app, paths);
                }
            }
        }
    );

    fn file_paths(pboard: &NSPasteboard) -> Vec<PathBuf> {
        let Some(items) = pboard.pasteboardItems() else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| {
                // SAFETY: an AppKit constant, read on the main thread.
                let url = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
                let url = NSURL::URLWithString(&url)?;
                url.isFileURL().then(|| url.path())?.map(|p| PathBuf::from(p.to_string()))
            })
            .collect()
    }

    /// Registers the provider. Finder lists the service once Voxi is
    /// installed as an app bundle, from the NSServices entry in Info.plist.
    pub fn install(app: &AppHandle) -> Result<(), &'static str> {
        let mtm = MainThreadMarker::new().ok_or("The Finder service must be installed on the main thread")?;
        let _ = APP.set(app.clone());
        let provider: Retained<Provider> = unsafe { msg_send![Provider::alloc(), init] };
        let ns_app = NSApplication::sharedApplication(mtm);
        // SAFETY: Provider implements the selector Info.plist names.
        unsafe { ns_app.setServicesProvider(Some(&*(Retained::as_ptr(&provider) as *const AnyObject))) };
        NSUpdateDynamicServices();
        PROVIDER.with(|slot| *slot.borrow_mut() = Some(provider));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn keeps_allowed_files_and_names_what_it_leaves_out() {
        let dir = tempfile::tempdir().unwrap();
        let h = fsx::canonical(dir.path());
        for d in ["Desktop", "Downloads", "Documents", "Pictures", "Documents/Archive", "Elsewhere"] {
            fs::create_dir_all(h.join(d)).unwrap();
        }
        for f in ["Downloads/a.pdf", "Downloads/b.pdf", "Documents/Archive/old.pdf", "Elsewhere/x.txt"] {
            fs::write(h.join(f), b"x").unwrap();
        }
        let roots = Roots::new(&h, None, &[]).with_exclusions(&[h.join("Documents/Archive")]);
        let (files, notice) = accept(
            &roots,
            vec![
                h.join("Downloads/a.pdf"),
                h.join("Downloads/b.pdf"),
                h.join("Downloads/a.pdf"),
                h.join("Documents/Archive/old.pdf"),
                h.join("Elsewhere/x.txt"),
                h.join("Desktop"),
            ],
        )
        .unwrap();
        assert_eq!(files, [h.join("Downloads/a.pdf"), h.join("Downloads/b.pdf")]);
        let notice = notice.unwrap();
        assert!(notice.contains("2 items outside the folders") && notice.contains("“old.pdf”") && notice.contains("“x.txt”"), "{notice}");
        assert!(notice.contains("1 folder left out") && notice.contains("“Desktop”"), "{notice}");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(h.join("Documents/Archive/old.pdf"), h.join("Downloads/link.pdf")).unwrap();
            let err = accept(&roots, vec![h.join("Downloads/link.pdf")]).unwrap_err();
            assert!(err.contains("1 symbolic link left out") && err.contains("“link.pdf”"), "{err}");
        }

        let err = accept(&roots, vec![h.join("Elsewhere/x.txt")]).unwrap_err();
        assert!(err.contains("outside the folders"), "{err}");
        assert_eq!(accept(&roots, vec![]).unwrap_err(), "Nothing was selected in Finder.");
    }

    #[test]
    fn long_lists_of_names_are_cut_short() {
        let all: Vec<String> = (1..=5).map(|i| format!("{i}.pdf")).collect();
        assert_eq!(names(&all), "“1.pdf”, “2.pdf”, “3.pdf” and 2 more");
    }
}
