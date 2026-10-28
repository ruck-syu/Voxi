//! Runs a confirmed plan step by step and records how to reverse each step.

use crate::fsx::{self, Roots, file_name};
use crate::model::Action;
use crate::{pdf, trash};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Outcome {
    pub done: Vec<Action>,
    /// Reverses `done`, newest step first.
    pub inverse: Vec<Action>,
    pub error: Option<String>,
}

/// Runs `actions` in order and stops at the first failure. Every path is
/// checked against the allowed folders again right before it is touched.
pub fn run(actions: &[Action], roots: &Roots) -> Outcome {
    let mut done = Vec::new();
    let mut inverse = Vec::new();
    for a in actions {
        match step(a, roots) {
            Ok(mut inv) => {
                done.push(a.clone());
                inv.reverse();
                inverse.splice(0..0, inv);
            }
            Err(e) => return Outcome { done, inverse, error: Some(e) },
        }
    }
    Outcome { done, inverse, error: None }
}

fn guard(roots: &Roots, paths: &[&Path]) -> Result<(), String> {
    for p in paths {
        if !roots.is_allowed(p) {
            return Err(format!("Stopped: {} is outside the allowed folders.", roots.display(p)));
        }
    }
    Ok(())
}

fn fresh(p: &Path) -> Result<(), String> {
    if p.symlink_metadata().is_ok() {
        Err(format!("Stopped: {} appeared since the preview. Nothing was overwritten.", p.display()))
    } else {
        Ok(())
    }
}

fn io_err(what: &str, p: &Path) -> impl Fn(io::Error) -> String {
    let msg = format!("Couldn't {what} {}", file_name(p));
    move |e| format!("{msg}: {e}")
}

fn tool(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("Couldn't run {cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{} failed: {}", cmd.rsplit('/').next().unwrap_or(cmd), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Moves without ever replacing an existing file. Falls back to `mv -n`
/// across volumes, where rename can't work.
fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    fresh(to)?;
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(18) => {
            tool("/bin/mv", &["-n".as_ref(), from.as_os_str(), to.as_os_str()])?;
            if from.exists() { Err(format!("Couldn't move {}", file_name(from))) } else { Ok(()) }
        }
        Err(e) => Err(io_err("move", from)(e)),
    }
}

/// Undoing something Voxi created puts it in the Trash, never a hard delete.
fn trash_inverse(p: &Path) -> Action {
    Action::Trash { path: p.to_path_buf() }
}

/// Runs one step and returns the steps that undo it.
fn step(a: &Action, roots: &Roots) -> Result<Vec<Action>, String> {
    match a {
        Action::Move { from, to } => {
            guard(roots, &[from, to])?;
            move_path(from, to)?;
            Ok(vec![Action::Move { from: to.clone(), to: from.clone() }])
        }
        Action::Copy { from, to } => {
            guard(roots, &[from, to])?;
            fresh(to)?;
            tool("/usr/bin/ditto", &[from.as_os_str(), to.as_os_str()])?;
            Ok(vec![trash_inverse(to)])
        }
        Action::Rename { path, new_name } => {
            let to = path.with_file_name(new_name);
            guard(roots, &[path, &to])?;
            if !fsx::same_file(path, &to) {
                fresh(&to)?;
            }
            fs::rename(path, &to).map_err(io_err("rename", path))?;
            Ok(vec![Action::Rename { path: to, new_name: file_name(path) }])
        }
        Action::Trash { path } => {
            guard(roots, &[path])?;
            if roots.is_root(path) {
                return Err(format!("Stopped: Voxi won't move {} itself to the Trash.", file_name(path)));
            }
            let landed = trash::to_trash(path, &roots.home)?;
            Ok(vec![Action::RestoreFromTrash { from: landed, to: path.clone() }])
        }
        Action::RestoreFromTrash { from, to } => {
            guard(roots, &[to])?;
            if !trash::is_in_trash(from) {
                return Err(format!("Stopped: {} isn't in the Trash.", from.display()));
            }
            fresh(to)?;
            fs::rename(from, to).map_err(|e| {
                format!("Couldn't take {} out of the Trash ({e}). Open the Trash in Finder and choose Put Back.", file_name(to))
            })?;
            Ok(vec![Action::Trash { path: to.clone() }])
        }
        Action::CreateFolder { path } => {
            guard(roots, &[path])?;
            match fs::create_dir(path) {
                Ok(()) => Ok(vec![Action::RemoveEmptyFolder { path: path.clone() }]),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(vec![]),
                Err(e) => Err(io_err("create", path)(e)),
            }
        }
        Action::RemoveEmptyFolder { path } => {
            guard(roots, &[path])?;
            // Only an empty folder goes; anything with contents stays put.
            let empty = fs::read_dir(path).map(|mut rd| rd.all(|e| e.is_ok_and(|e| e.file_name() == ".DS_Store"))).unwrap_or(false);
            if empty {
                let _ = fs::remove_file(path.join(".DS_Store"));
                let _ = fs::remove_dir(path);
            }
            Ok(vec![])
        }
        Action::ConvertImage { path, format, output } => {
            guard(roots, &[path, output])?;
            fresh(output)?;
            tool("/usr/bin/sips", &["-s".as_ref(), "format".as_ref(), format.sips_name().as_ref(), path.as_os_str(), "--out".as_ref(), output.as_os_str()])?;
            Ok(vec![trash_inverse(output)])
        }
        Action::ResizeImage { path, max_width_px, backup } => {
            guard(roots, &[path, backup])?;
            if let Some(dir) = backup.parent() {
                fs::create_dir_all(dir).map_err(io_err("create", dir))?;
            }
            fresh(backup)?;
            tool("/usr/bin/ditto", &[path.as_os_str(), backup.as_os_str()])?;
            tool("/usr/bin/sips", &["--resampleWidth".as_ref(), max_width_px.to_string().as_ref(), path.as_os_str()])?;
            Ok(vec![Action::Restore { from: backup.clone(), to: path.clone() }])
        }
        Action::Restore { from, to } => {
            guard(roots, &[from, to])?;
            // Keep the modified version too: it goes to the Trash.
            if to.exists() {
                trash::to_trash(to, &roots.home)?;
            }
            move_path(from, to)?;
            Ok(vec![])
        }
        Action::MergePdf { paths, output } => {
            guard(roots, &[output])?;
            guard(roots, &paths.iter().map(PathBuf::as_path).collect::<Vec<_>>())?;
            fresh(output)?;
            pdf::merge(paths, output)?;
            Ok(vec![trash_inverse(output)])
        }
        Action::ImagesToPdf { paths, output } => {
            guard(roots, &[output])?;
            guard(roots, &paths.iter().map(PathBuf::as_path).collect::<Vec<_>>())?;
            fresh(output)?;
            pdf::images_to_pdf(paths, output)?;
            Ok(vec![trash_inverse(output)])
        }
        Action::SplitPdf { path, output_dir } => {
            guard(roots, &[path, output_dir])?;
            fresh(output_dir)?;
            fs::create_dir(output_dir).map_err(io_err("create", output_dir))?;
            pdf::split(path, output_dir)?;
            Ok(vec![trash_inverse(output_dir)])
        }
        Action::ExtractPages { path, from, to, output } => {
            guard(roots, &[path, output])?;
            fresh(output)?;
            pdf::extract(path, *from, *to, output)?;
            Ok(vec![trash_inverse(output)])
        }
        Action::Zip { path, output } => {
            guard(roots, &[path, output])?;
            fresh(output)?;
            tool("/usr/bin/ditto", &["-c".as_ref(), "-k".as_ref(), "--sequesterRsrc".as_ref(), "--keepParent".as_ref(), path.as_os_str(), output.as_os_str()])?;
            Ok(vec![trash_inverse(output)])
        }
        Action::Unzip { path, destination } => {
            guard(roots, &[path, destination])?;
            fresh(destination)?;
            tool("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), path.as_os_str(), destination.as_os_str()])?;
            Ok(vec![trash_inverse(destination)])
        }
        Action::OpenInFinder { path, reveal } => {
            guard(roots, &[path])?;
            let mut args: Vec<&std::ffi::OsStr> = Vec::new();
            if *reveal {
                args.push("-R".as_ref());
            }
            args.push(path.as_os_str());
            tool("/usr/bin/open", &args)?;
            Ok(vec![])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::parse::parse;
    use crate::plan::{Planned, Planner};
    use std::collections::HashMap;

    fn setup() -> (tempfile::TempDir, Roots) {
        let dir = tempfile::tempdir().unwrap();
        let h = fsx::canonical(dir.path());
        for d in ["Desktop", "Downloads", "Documents", "Pictures"] {
            fs::create_dir(h.join(d)).unwrap();
        }
        let roots = Roots::new(&h, None, &[]);
        (dir, roots)
    }

    fn plan_and_run(roots: &Roots, history: &History, cmd: &str) -> Outcome {
        let choices = HashMap::new();
        let Ok(Planned::Steps { actions, .. }) = Planner::new(roots, &choices).plan(&parse(cmd).unwrap(), history) else {
            panic!("no plan for {cmd}")
        };
        run(&actions, roots)
    }

    #[test]
    fn move_then_undo_restores_everything() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::write(h.join("Downloads/a.pdf"), b"a").unwrap();
        fs::write(h.join("Downloads/b.pdf"), b"b").unwrap();
        let mut history = History::load(h.join("history.json"));

        let out = plan_and_run(&roots, &history, "move PDFs from Downloads to Archive");
        assert!(out.error.is_none());
        assert!(h.join("Downloads/Archive/a.pdf").exists());
        history.push(crate::history::Entry {
            id: 1,
            time: chrono::Local::now(),
            command: "move".into(),
            done: out.done,
            inverse: out.inverse,
            undone: false,
            undo_of: None,
        });

        let undo = plan_and_run(&roots, &history, "undo last");
        assert!(undo.error.is_none(), "{:?}", undo.error);
        assert!(h.join("Downloads/a.pdf").exists());
        assert!(h.join("Downloads/b.pdf").exists());
        assert!(!h.join("Downloads/Archive").exists(), "the folder Voxi created is removed once empty");
    }

    #[test]
    fn delete_never_removes_data() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::write(h.join("Downloads/setup.dmg"), b"x").unwrap();
        let history = History::load(h.join("history.json"));
        let out = plan_and_run(&roots, &history, "delete installers from Downloads");
        assert!(out.error.is_none());
        assert!(!h.join("Downloads/setup.dmg").exists());
        assert_eq!(fs::read(h.join(".Trash/setup.dmg")).unwrap(), b"x", "in the (stand-in) Trash, not deleted");
        // And undo takes it back out.
        let back = run(&out.inverse, &roots);
        assert!(back.error.is_none(), "{:?}", back.error);
        assert_eq!(fs::read(h.join("Downloads/setup.dmg")).unwrap(), b"x");
    }

    #[test]
    fn existing_target_is_never_overwritten() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let to = h.join("Documents/a.txt");
        fs::write(h.join("Downloads/a.txt"), b"new").unwrap();
        fs::write(&to, b"old").unwrap();
        let out = run(&[Action::Move { from: h.join("Downloads/a.txt"), to: to.clone() }], &roots);
        assert!(out.error.is_some());
        assert_eq!(fs::read(&to).unwrap(), b"old");
    }

    #[test]
    fn zip_unzip_round_trip() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::create_dir(h.join("Documents/Project")).unwrap();
        fs::write(h.join("Documents/Project/readme.md"), b"hi").unwrap();
        let history = History::load(h.join("history.json"));
        assert!(plan_and_run(&roots, &history, "zip Project into Project.zip").error.is_none());
        assert!(h.join("Documents/Project.zip").exists());
        assert!(plan_and_run(&roots, &history, "unzip Project.zip into Documents/out").error.is_none());
        assert_eq!(fs::read(h.join("Documents/out/Project/readme.md")).unwrap(), b"hi");
    }

    #[test]
    fn image_and_pdf_tools_on_sample_files() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let pics = h.join("Pictures");
        // A 64x32 PNG made with sips from a generated TIFF-free source.
        tool("/usr/bin/sips", &["-s".as_ref(), "format".as_ref(), "png".as_ref(),
            "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns".as_ref(),
            "--out".as_ref(), pics.join("icon.png").as_os_str()]).unwrap();
        let history = History::load(h.join("history.json"));

        assert!(plan_and_run(&roots, &history, "convert icon.png to jpg").error.is_none());
        assert!(pics.join("icon.jpg").exists());

        assert!(plan_and_run(&roots, &history, "resize icon.png to 16 pixels wide").error.is_none());
        let w = Command::new("/usr/bin/sips").args(["-g", "pixelWidth"]).arg(pics.join("icon.png")).output().unwrap();
        assert!(String::from_utf8_lossy(&w.stdout).contains("pixelWidth: 16"));

        assert!(plan_and_run(&roots, &history, "make a pdf from images in Pictures named icon").error.is_none());
        assert_eq!(pdf::page_count(&pics.join("icon.pdf")).unwrap(), 2);

        fs::copy(pics.join("icon.pdf"), pics.join("two.pdf")).unwrap();
        assert!(plan_and_run(&roots, &history, "merge PDFs in Pictures into all.pdf").error.is_none());
        assert_eq!(pdf::page_count(&pics.join("all.pdf")).unwrap(), 4);

        assert!(plan_and_run(&roots, &history, "extract pages 2 to 3 from all.pdf into mid.pdf").error.is_none());
        assert_eq!(pdf::page_count(&pics.join("mid.pdf")).unwrap(), 2);

        assert!(plan_and_run(&roots, &history, "split all.pdf into pages").error.is_none());
        assert_eq!(fs::read_dir(pics.join("all pages")).unwrap().count(), 4);
    }
}
