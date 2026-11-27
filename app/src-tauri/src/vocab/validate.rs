use crate::{
    model::{Destination, FileType, Filter, ImageFormat, Intent, Source},
    parse::{self, phrase},
    plan::Planner,
};
use regex::Captures;
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

pub(super) fn checked_output(kind: IntentKind, input: &str, output: &str, captures: &Captures<'_>) -> Option<Intent> {
    let intent = parse::parse_vocab_output(output)?;
    if IntentKind::of(&intent)? != kind {
        return None;
    }
    if let Intent::Move { to, .. } | Intent::Copy { to, .. } = &intent
        && to.path().is_some_and(Planner::names_the_trash)
    {
        return None;
    }
    preserves_filter(input, &intent, captures).then_some(intent)
}

fn format_word(token: &str, format: ImageFormat) -> bool {
    token == format.ext() || (format == ImageFormat::Jpg && token == "jpeg")
}

/// Interpret the original qualifiers using the same parser helpers as the
/// built-in grammar. Equality binds the normalized filter values (including
/// days, units, date bounds and name clauses) and catches discarded clauses.
fn preserves_filter(input: &str, intent: &Intent, captures: &Captures<'_>) -> bool {
    let mut input = input.to_owned();
    let mut compound_folders = Vec::new();
    let groups: Vec<_> = captures.iter().skip(1).flatten().collect();
    for folder in literal_folders(intent) {
        if let Some(capture) = groups.iter().rev().find(|capture| phrase::clean_name(&input[capture.range()]) == folder) {
            let span = capture.range();
            input.replace_range(span.clone(), &" ".repeat(span.len()));
        } else if groups.iter().any(|capture| input[capture.range()].contains(folder)) {
            compound_folders.push(folder);
        }
    }
    let Some((mut stated, rest)) = phrase::vocab_qualifiers(&input) else { return false };
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
    // A capture can hold a whole subject such as "PDFs in Documents named
    // Documents". Strip qualifiers before looking for the location in it.
    for folder in compound_folders {
        let folder = folder.to_lowercase();
        let parts: Vec<_> = folder.split_whitespace().collect();
        if !parts.is_empty()
            && let Some(start) = words.windows(parts.len()).rposition(|window| window == parts)
        {
            words[start..start + parts.len()].fill("");
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
