use crate::{
    model::{Destination, FileType, Filter, ImageFormat, Intent, Source},
    parse::{self, phrase},
};
use serde::Deserialize;

/// Explicit opt-in. New intent variants have to be reviewed against their
/// executor before a vocabulary pack can request them.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum IntentKind {
    Move,
    Copy,
    Rename,
    AddPrefix,
    AddDatePrefix,
    ReplaceInNames,
    Number,
    Lowercase,
    CreateFolders,
    SortByType,
    SortByMonth,
    ConvertImage,
    ResizeImage,
    MergePdf,
    SplitPdf,
    ExtractPages,
    ImagesToPdf,
    Zip,
    Unzip,
    List,
    Size,
    Count,
    Duplicates,
}

impl IntentKind {
    fn of(intent: &Intent) -> Option<Self> {
        Some(match intent {
            Intent::Move { .. } => Self::Move,
            Intent::Copy { .. } => Self::Copy,
            Intent::Rename { .. } => Self::Rename,
            Intent::AddPrefix { .. } => Self::AddPrefix,
            Intent::AddDatePrefix { .. } => Self::AddDatePrefix,
            Intent::ReplaceInNames { .. } => Self::ReplaceInNames,
            Intent::Number { .. } => Self::Number,
            Intent::Lowercase { .. } => Self::Lowercase,
            Intent::CreateFolders { .. } => Self::CreateFolders,
            Intent::SortByType { .. } => Self::SortByType,
            Intent::SortByMonth { .. } => Self::SortByMonth,
            Intent::ConvertImage { .. } => Self::ConvertImage,
            Intent::ResizeImage { .. } => Self::ResizeImage,
            Intent::MergePdf { .. } => Self::MergePdf,
            Intent::SplitPdf { .. } => Self::SplitPdf,
            Intent::ExtractPages { .. } => Self::ExtractPages,
            Intent::ImagesToPdf { .. } => Self::ImagesToPdf,
            Intent::Zip { .. } => Self::Zip,
            Intent::Unzip { .. } => Self::Unzip,
            Intent::List { .. } => Self::List,
            Intent::Size { .. } => Self::Size,
            Intent::Count { .. } => Self::Count,
            Intent::Duplicates { .. } => Self::Duplicates,
            Intent::Chain { .. } | Intent::Trash { .. } | Intent::Undo | Intent::Reveal { .. } | Intent::RecentActions => return None,
        })
    }
}

pub(super) fn checked_output(kind: IntentKind, input: &str, output: &str) -> Option<Intent> {
    let intent = parse::parse_vocab_output(output)?;
    if IntentKind::of(&intent)? != kind {
        return None;
    }
    let lower = input.to_lowercase();
    let present = |name: &str| contains_whole(&lower, &name.to_lowercase());
    let destination = |dest: &Destination| match dest {
        Destination::Path(path) => present(path),
        Destination::InFolder { folder, parent, literal, .. } => {
            present(folder) && present(parent) && literal.as_deref().is_none_or(present)
        }
    };
    let unquoted = phrase::unquoted_text(&lower)?;
    let number = |n: u32| unquoted.split_whitespace().any(|token| token.parse::<u32>() == Ok(n));
    let source_ok = match intent.source() {
        Some(Source::Item { name, folder }) => present(name) && folder.as_deref().is_none_or(present),
        Some(Source::Set { folder, .. }) => folder.as_deref().is_none_or(present),
        None => true,
    };
    let arguments_ok = match &intent {
        Intent::Move { to, .. }
        | Intent::Copy { to, .. }
        | Intent::SortByType { folder: to, .. }
        | Intent::SortByMonth { folder: to, .. } => destination(to),
        Intent::Rename { new_name: name, .. }
        | Intent::AddPrefix { prefix: name, .. }
        | Intent::Size { folder: name }
        | Intent::Duplicates { folder: name } => present(name),
        Intent::ReplaceInNames { find, replace, .. } => present(find) && (replace.is_empty() || present(replace)),
        Intent::CreateFolders { names, parent } => names.iter().all(|name| present(name)) && parent.as_deref().is_none_or(present),
        Intent::Number { base, width, .. } => present(base) && u32::try_from(*width).is_ok_and(number),
        Intent::ConvertImage { format, .. } => unquoted.split_whitespace().any(|token| format_word(token, *format)),
        Intent::ResizeImage { width, .. } => number(*width),
        Intent::MergePdf { output, .. } => output == "merged.pdf" || present(output),
        Intent::ExtractPages { from, to, output, .. } => number(*from) && number(*to) && output.as_deref().is_none_or(present),
        Intent::ImagesToPdf { output, .. } | Intent::Zip { output, .. } | Intent::Unzip { destination: output, .. } => {
            output.as_deref().is_none_or(present)
        }
        _ => true,
    };
    (source_ok && arguments_ok && preserves_filter(input, &intent)).then_some(intent)
}

fn contains_whole(input: &str, value: &str) -> bool {
    let boundary = |ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',');
    !value.is_empty()
        && input.match_indices(value).any(|(start, matched)| {
            input[..start].chars().next_back().is_none_or(boundary) && input[start + matched.len()..].chars().next().is_none_or(boundary)
        })
}

fn format_word(token: &str, format: ImageFormat) -> bool {
    token == format.ext() || (format == ImageFormat::Jpg && token == "jpeg")
}

/// Interpret the original qualifiers using the same parser helpers as the
/// built-in grammar. Equality binds the normalized filter values (including
/// days, units, date bounds and name clauses) and catches discarded clauses.
fn preserves_filter(input: &str, intent: &Intent) -> bool {
    let Some((mut stated, rest)) = phrase::vocab_qualifiers(input) else { return false };
    let mut actual = match intent {
        Intent::SortByType { include_subfolders, .. } | Intent::SortByMonth { include_subfolders, .. } => {
            Filter { include_subfolders: *include_subfolders, ..Filter::default() }
        }
        _ => match intent.source() {
            Some(Source::Set { filter, .. }) => filter.clone(),
            _ => Filter::default(),
        },
    };

    // Action phrases can contain a format or the noun "pages". Exclude these
    // fixed grammar roles before interpreting unquoted nouns as type words.
    let Some(rest) = phrase::unquoted_text(&rest) else { return false };
    let lower = rest.to_lowercase();
    let mut words: Vec<_> = lower.split_whitespace().collect();
    let folders = literal_folders(intent);
    for folder in &folders {
        let folder = folder.to_lowercase();
        let folder: Vec<_> = folder.split_whitespace().collect();
        for start in 1..words.len() {
            if matches!(words[start - 1], "in" | "from" | "on" | "inside" | "within" | "under" | "into" | "to")
                && words[start..].starts_with(&folder)
            {
                // A location such as Pictures or Documents is also a type
                // word. Ignore it only in an explicit location in the input.
                words[start..start + folder.len()].fill("");
            }
        }
    }
    let mut words = words.as_slice();
    match intent {
        Intent::ImagesToPdf { .. } => {
            if matches!(words, [_, "1" | "a", "pdf", "from", ..] | [_, "1" | "a", "pdf", "of", ..]) {
                words = &words[4..];
            }
        }
        Intent::SplitPdf { .. } if words.ends_with(&["into", "pages"]) => words = &words[..words.len() - 2],
        Intent::ExtractPages { .. } if matches!(words, [_, "pages", number, ..] if number.parse::<u32>().is_ok()) => {
            words = &words[2..];
        }
        Intent::ConvertImage { format, .. } if matches!(words, [.., "to" | "as" | "into", token] if format_word(token, *format)) => {
            words = &words[..words.len() - 2];
        }
        _ => {}
    }
    while !words.is_empty() {
        let type_word = (1..=words.len().min(4)).rev().find_map(|len| {
            let (kind, ext) = phrase::type_word(&words[..len].join(" "))?;
            (kind != FileType::Files || ext.is_some()).then_some((len, kind, ext))
        });
        if let Some((len, kind, ext)) = type_word {
            if stated.file_type.is_some_and(|file_type| file_type != kind)
                || ext.as_ref().is_some_and(|ext| actual.ext.as_ref() != Some(ext))
            {
                return false;
            }
            stated.file_type = Some(kind);
            if ext.is_some() {
                if stated.ext.is_some() && stated.ext != ext {
                    return false;
                }
                stated.ext = ext;
            }
            words = &words[len..];
        } else if words.get(1).is_some_and(|word| phrase::type_word(word).is_some())
            && let Some(month) = phrase::full_month(words[0])
        {
            if stated.month.replace(month).is_some_and(|prior| prior != month) {
                return false;
            }
            words = &words[1..];
        } else {
            words = &words[1..];
        }
    }

    // Generic "files" carries no type restriction. The narrower built-in
    // defaults of typed actions still need an explicit type word in the input.
    if actual.file_type == Some(FileType::Files) && stated.file_type.is_none() {
        actual.file_type = None;
    }
    stated == actual
}

fn literal_folders(intent: &Intent) -> Vec<&str> {
    let folder = match intent.source() {
        Some(Source::Item { folder, .. } | Source::Set { folder, .. }) => folder.as_deref(),
        _ => None,
    };
    let mut folders: Vec<_> = folder.into_iter().collect();
    match intent {
        Intent::Move { to, .. }
        | Intent::Copy { to, .. }
        | Intent::SortByType { folder: to, .. }
        | Intent::SortByMonth { folder: to, .. } => match to {
            Destination::Path(path) => folders.push(path),
            Destination::InFolder { folder, parent, .. } => folders.extend([folder.as_str(), parent.as_str()]),
        },
        Intent::Unzip { destination: Some(folder), .. }
        | Intent::CreateFolders { parent: Some(folder), .. }
        | Intent::Size { folder }
        | Intent::Duplicates { folder } => folders.push(folder),
        _ => {}
    }
    folders
}
