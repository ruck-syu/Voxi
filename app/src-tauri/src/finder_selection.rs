//! Files handed over by macOS become the current results list, so "move
//! these to Receipts" acts on exactly that selection.

use crate::fsx::{self, Roots};
use std::collections::HashSet;
use std::path::PathBuf;
use tauri::Url;

/// What the window says about a selection it couldn't take in full.
pub const LABEL: &str = "your Finder selection";

/// Keep local file paths in their original order. Existence and access are
/// checked by `accept` so the window can explain why an item was left out.
pub fn paths_from_urls(urls: Vec<Url>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    urls.into_iter().filter_map(path_from_url).filter(|path| seen.insert(path.clone())).collect()
}

fn path_from_url(url: Url) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let url = if url.scheme() == "file" && url.path().starts_with("/.file/id=") {
        // Finder can supply a file reference URL. Ask Foundation for its
        // current path before checking it against the allowed folders.
        use objc2_foundation::{NSString, NSURL};
        let reference = NSURL::URLWithString(&NSString::from_str(url.as_str()))?;
        Url::parse(&reference.filePathURL()?.absoluteString()?.to_string()).ok()?
    } else {
        url
    };
    url.to_file_path().ok()
}

/// Keeps the selected files Voxi may touch. Folders and anything outside the
/// allowed folders (or inside an excluded one) are left out and named in the
/// returned notice; with nothing left, the notice is the error.
pub fn accept(roots: &Roots, selected: Vec<PathBuf>) -> Result<(Vec<PathBuf>, Option<String>), String> {
    let (mut files, mut folders, mut outside, mut links, mut missing) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut seen = HashSet::new();
    for p in selected {
        let name = fsx::file_name(&p);
        // A link would be swapped for the file it points to, which the user
        // didn't select: leave it out and say so.
        match std::fs::symlink_metadata(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push(name),
            Ok(m) if m.is_symlink() => links.push(name),
            Ok(m) if roots.is_allowed(&p) => {
                if m.is_dir() {
                    folders.push(name);
                } else if m.is_file() {
                    let c = fsx::canonical(&p);
                    if seen.insert(c.clone()) {
                        files.push(c);
                    }
                } else {
                    outside.push(name);
                }
            }
            _ => outside.push(name),
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
    if !missing.is_empty() {
        notes.push(format!("{} could no longer be found: {}.", count(missing.len()), names(&missing)));
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
    use objc2::{AnyThread, ClassType, MainThreadMarker, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSPasteboard, NSPasteboardURLReadingFileURLsOnlyKey, NSUpdateDynamicServices};
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL};
    use std::cell::RefCell;
    use std::sync::OnceLock;
    use tauri::{AppHandle, Url};

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
                if let Some(app) = APP.get() {
                    crate::use_finder_selection(app, file_urls(pboard));
                }
            }
        }
    );

    fn file_urls(pboard: &NSPasteboard) -> Vec<Url> {
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let file_only: &AnyObject = &NSNumber::new_bool(true);
        // SAFETY: The AppKit key maps to a boolean, and NSURL implements
        // NSPasteboardReading.
        let items = unsafe {
            let options = NSDictionary::from_slices(&[NSPasteboardURLReadingFileURLsOnlyKey], &[file_only]);
            pboard.readObjectsForClasses_options(&classes, Some(&options))
        };
        let Some(items) = items else { return Vec::new() };
        items
            .iter()
            .filter_map(|item| {
                let url = item.downcast_ref::<NSURL>()?.absoluteString()?;
                Url::parse(&url.to_string()).ok()
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
    #[cfg(target_os = "macos")]
    fn resolves_finders_file_reference_urls() {
        use objc2_foundation::{NSString, NSURL};
        let dir = tempfile::tempdir().unwrap();
        let path = fsx::canonical(dir.path()).join("report #1.pdf");
        fs::write(&path, b"x").unwrap();
        let ns_url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let reference = ns_url.fileReferenceURL().unwrap();
        assert!(reference.isFileReferenceURL());
        let url = Url::parse(&reference.absoluteString().unwrap().to_string()).unwrap();
        assert_eq!(paths_from_urls(vec![url, Url::from_file_path(&path).unwrap()]), [path]);
    }

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
