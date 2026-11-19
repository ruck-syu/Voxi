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

/// Which steps the user may leave out. A step qualifies when it is a move,
/// copy, trash or rename and no other step touches the same path, a folder
/// above it, or anything inside it. Folders the plan creates don't count:
/// they are dropped later if only left-out steps needed them.
pub fn skippable(actions: &[Action]) -> Vec<bool> {
    let all: Vec<Vec<PathBuf>> = actions.iter().map(paths).collect();
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

/// The plan without the steps at `skip`, which must all be skippable. Folder
/// creations that only left-out steps needed are dropped too, so leaving out
/// every file bound for a new folder doesn't leave an empty folder behind.
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
    let left_out: Vec<&PathBuf> = skip.iter().flat_map(|&i| paths_of_target(&actions[i])).collect();
    let kept: Vec<(usize, &Action)> = actions.iter().enumerate().filter(|(i, _)| !skip.contains(i)).collect();
    let needed = |folder: &Path| {
        kept.iter().any(|(_, a)| !matches!(a, Action::CreateFolder { .. }) && paths(a).iter().any(|p| p.starts_with(folder) && p != folder))
    };
    let out: Vec<Action> = kept
        .iter()
        .filter(|(_, a)| match a {
            Action::CreateFolder { path } => needed(path) || !left_out.iter().any(|p| p.starts_with(path)),
            _ => true,
        })
        .map(|(_, a)| (*a).clone())
        .collect();
    if out.is_empty() {
        return Err("Every step was left out, so there is nothing to run.".into());
    }
    Ok(out)
}

/// Where a skippable step would have put something.
fn paths_of_target(a: &Action) -> Vec<&PathBuf> {
    match a {
        Action::Move { to, .. } | Action::Copy { to, .. } => vec![to],
        _ => vec![],
    }
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
    fn leaving_out_every_file_for_a_new_folder_drops_the_folder() {
        let actions = vec![
            Action::CreateFolder { path: "/h/Documents/PDFs".into() },
            mv("/h/Downloads/a.pdf", "/h/Documents/PDFs/a.pdf"),
            mv("/h/Downloads/b.pdf", "/h/Documents/PDFs/b.pdf"),
            Action::Trash { path: "/h/Downloads/old.dmg".into() },
        ];
        assert_eq!(without(&actions, &[1]).unwrap(), [actions[0].clone(), actions[2].clone(), actions[3].clone()]);
        assert_eq!(without(&actions, &[1, 2]).unwrap(), [actions[3].clone()]);
        assert!(without(&actions, &[1, 2, 3]).unwrap_err().contains("nothing to run"));
        assert!(without(&actions, &[0]).unwrap_err().contains("Step 1 can't be left out"));
        assert!(without(&actions, &[9]).is_err());
    }
}
