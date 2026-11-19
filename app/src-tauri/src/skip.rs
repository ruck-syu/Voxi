//! Leaving steps out of a previewed plan. Only independent moves, copies,
//! trashes and renames can be left out: a step that another step reads from
//! or writes near stays, so the rest of the plan runs exactly as previewed.

use crate::model::Action;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Every path a step reads or writes.
fn paths(a: &Action) -> Vec<PathBuf> {
    match a {
        Action::Move { from, to } | Action::Copy { from, to } => vec![from.clone(), to.clone()],
        Action::Rename { path, new_name } => vec![path.clone(), path.with_file_name(new_name)],
        Action::Trash { path } | Action::CreateFolder { path } | Action::OpenInFinder { path, .. } | Action::RemoveEmptyFolder { path } => {
            vec![path.clone()]
        }
        Action::ConvertImage { path, output, .. }
        | Action::ExtractPages { path, output, .. }
        | Action::Zip { path, output }
        | Action::ResizeImage { path, backup: output, .. } => vec![path.clone(), output.clone()],
        Action::SplitPdf { path, output_dir } => vec![path.clone(), output_dir.clone()],
        Action::Unzip { path, destination } => vec![path.clone(), destination.clone()],
        Action::ImagesToPdf { paths, output } | Action::MergePdf { paths, output } => {
            paths.iter().cloned().chain(std::iter::once(output.clone())).collect()
        }
        Action::RestoreFromTrash { from, to } | Action::Restore { from, to } => vec![from.clone(), to.clone()],
    }
}

fn related(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// A path as the file system sees it, for comparing steps: the deepest
/// existing folder resolved through any symbolic links, then the rest, all
/// lowercased because macOS volumes usually ignore case. Folding case on a
/// case-sensitive volume only ties more steps together, never fewer.
fn key(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut base = p;
    let resolved = loop {
        if let Ok(real) = std::fs::canonicalize(base) {
            break real;
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                base = parent;
            }
            _ => break base.to_path_buf(),
        }
    };
    let full = rest.iter().rev().fold(resolved, |acc, name| acc.join(name));
    PathBuf::from(full.to_string_lossy().to_lowercase())
}

/// True when a step reads a symbolic link. A moved or renamed link can
/// point at another step's output, which paths alone can't show.
fn reads_link(a: &Action) -> bool {
    paths(a).iter().any(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()))
}

/// Which steps the user may leave out. A step qualifies when it is a move,
/// copy, trash or rename and no other step touches the same path, a folder
/// above it, or anything inside it. Folders the plan creates don't count:
/// they are dropped later if only left-out steps needed them.
pub fn skippable(actions: &[Action]) -> Vec<bool> {
    // A plan that touches a link keeps every step: what depends on what
    // can't be read from the paths.
    if actions.iter().any(reads_link) {
        return vec![false; actions.len()];
    }
    let all: Vec<Vec<PathBuf>> = actions.iter().map(|a| paths(a).iter().map(|p| key(p)).collect()).collect();
    // How many path mentions sit at each path, and strictly below each folder,
    // across the steps that aren't folder creations.
    let mut at: HashMap<&Path, usize> = HashMap::new();
    let mut below: HashMap<&Path, usize> = HashMap::new();
    for (a, ps) in actions.iter().zip(&all) {
        if matches!(a, Action::CreateFolder { .. }) {
            continue;
        }
        for p in ps {
            *at.entry(p).or_default() += 1;
            for up in p.ancestors().skip(1) {
                *below.entry(up).or_default() += 1;
            }
        }
    }
    actions
        .iter()
        .zip(&all)
        .map(|(a, mine)| {
            matches!(a, Action::Move { .. } | Action::Copy { .. } | Action::Trash { .. } | Action::Rename { .. })
                && mine.iter().all(|m| {
                    // Mentions related to `m`: at `m` or a folder above it, or below it.
                    let near = m.ancestors().map(|up| at.get(up).copied().unwrap_or(0)).sum::<usize>()
                        + below.get(m.as_path()).copied().unwrap_or(0);
                    // This step's own mentions related to `m` (including `m` itself).
                    let own = mine.iter().filter(|o| related(o, m)).count();
                    near == own
                })
        })
        .collect()
}

/// The plan without the steps at `skip`, which must all be skippable. Every
/// other step stays, folder creations included: Voxi can't tell a folder
/// the user asked for from one planned only to hold the left-out files, and
/// a kept folder's own parent may be one of them.
pub fn without(actions: &[Action], skip: &[usize]) -> Result<Vec<Action>, String> {
    let ok = skippable(actions);
    let skip: HashSet<usize> = skip.iter().copied().collect();
    if let Some(&bad) = skip.iter().find(|&&i| !ok.get(i).copied().unwrap_or(false)) {
        return Err(if bad >= actions.len() {
            "That preview changed. Run the command again.".into()
        } else {
            format!("Step {} can't be left out, because other steps depend on it.", bad + 1)
        });
    }
    let out: Vec<Action> = actions.iter().enumerate().filter(|(i, _)| !skip.contains(i)).map(|(_, a)| a.clone()).collect();
    if out.is_empty() {
        return Err("Every step was left out, so there is nothing to run.".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(from: &str, to: &str) -> Action {
        Action::Move { from: from.into(), to: to.into() }
    }

    #[test]
    fn independent_moves_can_be_left_out_but_dependent_steps_cannot() {
        let actions = vec![
            Action::CreateFolder { path: "/h/Documents/PDFs".into() },
            mv("/h/Downloads/a.pdf", "/h/Documents/PDFs/a.pdf"),
            mv("/h/Downloads/b.pdf", "/h/Documents/PDFs/b.pdf"),
            // Converting c.png then moving the output: both depend on each other.
            Action::ConvertImage {
                path: "/h/Downloads/c.png".into(),
                format: crate::model::ImageFormat::Jpg,
                output: "/h/Downloads/c.jpg".into(),
            },
            mv("/h/Downloads/c.jpg", "/h/Pictures/c.jpg"),
            Action::Trash { path: "/h/Downloads/old.dmg".into() },
            Action::Rename { path: "/h/Desktop/x.txt".into(), new_name: "y.txt".into() },
        ];
        assert_eq!(skippable(&actions), [false, true, true, false, false, true, true]);
    }

    #[test]
    fn a_move_into_a_folder_another_step_zips_stays() {
        let actions = vec![
            mv("/h/Downloads/a.pdf", "/h/Documents/Box/a.pdf"),
            Action::Zip { path: "/h/Documents/Box".into(), output: "/h/Documents/Box.zip".into() },
        ];
        assert_eq!(skippable(&actions), [false, false]);
        // A folder move and a move of a file inside it are tied together too.
        let actions = vec![mv("/h/Downloads/Box", "/h/Documents/Box"), mv("/h/Downloads/Box/a.pdf", "/h/Desktop/a.pdf")];
        assert_eq!(skippable(&actions), [false, false]);
    }

    #[test]
    fn leaving_out_steps_keeps_every_folder_creation() {
        let actions = vec![
            Action::CreateFolder { path: "/h/Documents/Parent".into() },
            Action::CreateFolder { path: "/h/Documents/Parent/Empty".into() },
            Action::CreateFolder { path: "/h/Documents/Parent/Incoming".into() },
            mv("/h/Downloads/a.pdf", "/h/Documents/Parent/Incoming/a.pdf"),
            Action::Trash { path: "/h/Downloads/old.dmg".into() },
        ];
        assert_eq!(without(&actions, &[3]).unwrap(), [&actions[..3], &actions[4..]].concat());
        assert_eq!(without(&actions, &[3, 4]).unwrap(), actions[..3]);
        assert!(without(&actions, &[0]).unwrap_err().contains("Step 1 can't be left out"));
        assert!(without(&actions, &[9]).is_err());
        let one = vec![mv("/h/Downloads/a.pdf", "/h/Documents/a.pdf")];
        assert!(without(&one, &[0]).unwrap_err().contains("nothing to run"));
    }

    #[test]
    fn paths_differing_only_in_case_are_the_same_file() {
        let actions = vec![mv("/h/Downloads/Photo.png", "/h/Pictures/photo.png"), Action::Trash { path: "/h/pictures/PHOTO.PNG".into() }];
        assert_eq!(skippable(&actions), [false, false]);
    }

    #[cfg(unix)]
    #[test]
    fn links_and_linked_folders_tie_steps_together() {
        let dir = tempfile::tempdir().unwrap();
        let h = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(h.join("Real")).unwrap();
        std::os::unix::fs::symlink(h.join("Real"), h.join("Alias")).unwrap();
        // The same file reached through a linked folder.
        let actions = vec![
            mv(h.join("a.png").to_str().unwrap(), h.join("Alias/a.png").to_str().unwrap()),
            Action::Trash { path: h.join("Real/a.png") },
        ];
        assert_eq!(skippable(&actions), [false, false]);
        // A plan that moves a link keeps every step.
        std::fs::write(h.join("b.png"), b"b").unwrap();
        std::os::unix::fs::symlink(h.join("b.png"), h.join("link.png")).unwrap();
        let actions = vec![
            mv(h.join("link.png").to_str().unwrap(), h.join("Real/link.png").to_str().unwrap()),
            Action::Trash { path: h.join("c.png") },
        ];
        assert_eq!(skippable(&actions), [false, false]);
    }
}
