//! Rule-based command parser. Each category file contributes named regex
//! patterns; they are tried in order, most specific first, and the first one
//! that matches and builds an intent wins.

mod archives;
mod cleanup;
mod dates;
mod files;
mod find;
mod folders;
mod images;
mod meta;
mod pdf;
pub mod phrase;
mod rename;

use crate::model::{FileType, Intent, Source};
use regex::{Captures, Regex};
use std::sync::LazyLock;

pub struct Pattern {
    #[allow(dead_code)] // identifies the pattern when debugging
    pub name: &'static str,
    pub re: Regex,
    pub build: fn(&Captures) -> Option<Intent>,
}

pub fn pat(name: &'static str, re: &str, build: fn(&Captures) -> Option<Intent>) -> Pattern {
    Pattern { name, re: Regex::new(&format!("(?i){re}")).expect(name), build }
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    let mut v = Vec::new();
    v.extend(meta::patterns());
    v.extend(find::specific());
    v.extend(folders::patterns());
    v.extend(pdf::patterns());
    v.extend(images::patterns());
    v.extend(archives::patterns());
    v.extend(rename::patterns());
    v.extend(cleanup::patterns());
    v.extend(files::patterns());
    v.extend(find::general());
    v.extend(meta::open());
    v
});

/// Parses a command. `Err` carries the raw text for "Didn't understand".
pub fn parse(text: &str) -> Result<Intent, String> {
    let cleaned = phrase::normalize(text);
    for p in PATTERNS.iter() {
        if let Some(caps) = p.re.captures(&cleaned)
            && let Some(intent) = (p.build)(&caps)
        {
            return Ok(intent);
        }
    }
    Err(text.trim().to_string())
}

/// Capture group `i` as a trimmed string, if present and non-empty.
pub(crate) fn cap(c: &Captures, i: usize) -> Option<String> {
    c.get(i).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty())
}

pub(crate) fn src(c: &Captures, i: usize) -> Option<Source> {
    phrase::parse_source(&cap(c, i)?)
}

/// Like `src`, but a generic "files" set is narrowed to `t`
/// ("merge everything in Downloads" means its PDFs).
pub(crate) fn typed_src(c: &Captures, i: usize, t: FileType) -> Option<Source> {
    match src(c, i)? {
        Source::Set { folder, mut filter } => {
            if matches!(filter.file_type, None | Some(FileType::Files)) && filter.ext.is_none() {
                filter.file_type = Some(t);
            }
            Some(Source::Set { folder, filter })
        }
        item => Some(item),
    }
}

#[cfg(test)]
mod corpus_tests;

#[cfg(test)]
mod catalog_tests {
    //! One test per row of the command catalog in BUILD.md, section 4.
    use super::parse;
    use crate::model::{DayRef, FileType::*, Filter, ImageFormat, Intent::*, Source};

    fn set(folder: Option<&str>, f: Filter) -> Source {
        Source::set(folder, f)
    }
    fn of(t: crate::model::FileType) -> Filter {
        Filter { file_type: Some(t), ..Default::default() }
    }
    fn item(n: &str) -> Source {
        Source::item(n)
    }
    fn p(s: &str) -> crate::model::Intent {
        parse(s).unwrap_or_else(|e| panic!("didn't understand: {e}"))
    }

    #[test]
    fn r01_move_file() {
        assert_eq!(p("move report.pdf to Documents"), Move { src: item("report.pdf"), to: "Documents".into() });
    }
    #[test]
    fn r02_move_screenshots_today() {
        assert_eq!(
            p("move screenshots from today to Screenshots"),
            Move { src: set(None, Filter { day: Some(DayRef::Today), ..of(Screenshots) }), to: "Screenshots".into() }
        );
    }
    #[test]
    fn r03_move_pdfs() {
        assert_eq!(
            p("move all PDFs from Downloads to Documents"),
            Move { src: set(Some("Downloads"), of(Pdfs)), to: "Documents".into() }
        );
    }
    #[test]
    fn r04_move_large_files() {
        assert_eq!(
            p("move files larger than 100 MB from Downloads to External"),
            Move { src: set(Some("Downloads"), Filter { larger_than_mb: Some(100.0), ..of(Files) }), to: "External".into() }
        );
    }
    #[test]
    fn r05_move_containing() {
        assert_eq!(
            p("move files containing invoice from Downloads to Invoices"),
            Move { src: set(Some("Downloads"), Filter { name_contains: Some("invoice".into()), ..of(Files) }), to: "Invoices".into() }
        );
    }
    #[test]
    fn r06_copy_file() {
        assert_eq!(p("copy notes.txt to Desktop"), Copy { src: item("notes.txt"), to: "Desktop".into() });
    }
    #[test]
    fn r07_copy_images() {
        assert_eq!(p("copy all images from Pictures to Backup"), Copy { src: set(Some("Pictures"), of(Images)), to: "Backup".into() });
    }
    #[test]
    fn r08_rename() {
        assert_eq!(p("rename report.pdf to Q3-report.pdf"), Rename { src: item("report.pdf"), new_name: "Q3-report.pdf".into() });
    }
    #[test]
    fn r09_add_prefix() {
        assert_eq!(
            p("add prefix Invoice- to files named bill in Downloads"),
            AddPrefix { src: set(Some("Downloads"), Filter { name_contains: Some("bill".into()), ..of(Files) }), prefix: "Invoice-".into() }
        );
    }
    #[test]
    fn r10_date_prefix() {
        assert_eq!(p("add date prefix to files in Screenshots"), AddDatePrefix { src: set(Some("Screenshots"), of(Files)) });
    }
    #[test]
    fn r11_replace_spaces() {
        assert_eq!(
            p("replace spaces with dashes in files in Downloads"),
            ReplaceInNames { src: set(Some("Downloads"), of(Files)), find: " ".into(), replace: "-".into() }
        );
    }
    #[test]
    fn r12_replace_words() {
        assert_eq!(
            p("replace draft with final in names in Documents"),
            ReplaceInNames { src: set(Some("Documents"), of(Files)), find: "draft".into(), replace: "final".into() }
        );
    }
    #[test]
    fn r13_number() {
        assert_eq!(
            p("number images in Trip as trip-001, trip-002"),
            Number { src: set(Some("Trip"), of(Images)), base: "trip-".into(), width: 3 }
        );
    }
    #[test]
    fn r14_lowercase() {
        assert_eq!(p("lowercase file names in Downloads"), Lowercase { src: set(Some("Downloads"), of(Files)) });
    }
    #[test]
    fn r15_create_folder() {
        assert_eq!(p("create folder Invoices on Desktop"), CreateFolders { names: vec!["Invoices".into()], parent: Some("Desktop".into()) });
    }
    #[test]
    fn r16_create_folders() {
        assert_eq!(
            p("create folders 2026, 2027 in Documents"),
            CreateFolders { names: vec!["2026".into(), "2027".into()], parent: Some("Documents".into()) }
        );
    }
    #[test]
    fn r17_sort_type() {
        assert_eq!(p("sort Downloads by type"), SortByType { folder: "Downloads".into() });
    }
    #[test]
    fn r18_sort_month() {
        assert_eq!(p("sort Screenshots by month"), SortByMonth { folder: "Screenshots".into() });
    }
    #[test]
    fn r19_convert_one() {
        assert_eq!(p("convert photo.heic to png"), ConvertImage { src: item("photo.heic"), format: ImageFormat::Png });
    }
    #[test]
    fn r20_convert_many() {
        assert_eq!(
            p("convert all HEIC in Pictures to jpg"),
            ConvertImage {
                src: set(Some("Pictures"), Filter { ext: Some(vec!["heic".into(), "heif".into()]), ..of(Files) }),
                format: ImageFormat::Jpg
            }
        );
    }
    #[test]
    fn r21_resize_one() {
        assert_eq!(p("resize photo.jpg to 1200 pixels wide"), ResizeImage { src: item("photo.jpg"), width: 1200 });
    }
    #[test]
    fn r22_resize_many() {
        assert_eq!(p("resize all images in Trip to 1600 pixels wide"), ResizeImage { src: set(Some("Trip"), of(Images)), width: 1600 });
    }
    #[test]
    fn r23_merge_named() {
        assert_eq!(
            p("merge PDFs in Documents named scan into scan-all.pdf"),
            MergePdf { src: set(Some("Documents"), Filter { name_contains: Some("scan".into()), ..of(Pdfs) }), output: "scan-all.pdf".into() }
        );
    }
    #[test]
    fn r24_merge_all() {
        assert_eq!(p("merge all PDFs in Downloads into combined.pdf"), MergePdf { src: set(Some("Downloads"), of(Pdfs)), output: "combined.pdf".into() });
    }
    #[test]
    fn r25_split() {
        assert_eq!(p("split report.pdf into pages"), SplitPdf { src: item("report.pdf") });
    }
    #[test]
    fn r26_extract() {
        assert_eq!(
            p("extract pages 2 to 5 from report.pdf into excerpt.pdf"),
            ExtractPages { src: item("report.pdf"), from: 2, to: 5, output: Some("excerpt.pdf".into()) }
        );
    }
    #[test]
    fn r27_images_to_pdf() {
        assert_eq!(
            p("make a PDF from images in Scans named receipts"),
            ImagesToPdf { src: set(Some("Scans"), Filter { name_contains: Some("receipts".into()), ..of(Images) }), output: None }
        );
    }
    #[test]
    fn r28_zip() {
        assert_eq!(p("zip Project into Project.zip"), Zip { src: item("Project"), output: Some("Project.zip".into()) });
    }
    #[test]
    fn r29_unzip() {
        assert_eq!(p("unzip archive.zip into Downloads/archive"), Unzip { src: item("archive.zip"), destination: Some("Downloads/archive".into()) });
    }
    #[test]
    fn r30_clean_up() {
        assert_eq!(
            p("clean up Downloads older than 90 days"),
            Trash { src: set(Some("Downloads"), Filter { older_than_days: Some(90), ..of(Files) }) }
        );
    }
    #[test]
    fn r31_installers_to_trash() {
        assert_eq!(p("move installers from Downloads to Trash folder"), Trash { src: set(Some("Downloads"), of(Installers)) });
    }
    #[test]
    fn r32_delete_screenshots() {
        assert_eq!(
            p("delete screenshots older than 30 days"),
            Trash { src: set(None, Filter { older_than_days: Some(30), ..of(Screenshots) }) }
        );
    }
    #[test]
    fn r33_list_large() {
        assert_eq!(
            p("list files in Downloads larger than 500 MB"),
            List { src: set(Some("Downloads"), Filter { larger_than_mb: Some(500.0), ..of(Files) }) }
        );
    }
    #[test]
    fn r34_list_old() {
        assert_eq!(
            p("list files older than 1 year in Documents"),
            List { src: set(Some("Documents"), Filter { older_than_days: Some(365), ..of(Files) }) }
        );
    }

    #[test]
    fn stl_file_types_are_extension_filters_not_name_filters() {
        let want = List { src: set(Some("Downloads"), Filter {
            ext: Some(vec!["stl".into()]), ..of(Files)
        }) };
        for command in [" list the STL files in Downloads. ", "list stl files in Downloads", "list .STL files in Downloads", "list STLs in Downloads"] {
            assert_eq!(p(command), want, "{command}");
        }
        assert_eq!(p("list files named STL in Downloads"), List {
            src: set(Some("Downloads"), Filter { name_contains: Some("STL".into()), ..of(Files) }),
        });
    }
    #[test]
    fn r35_size() {
        assert_eq!(p("show size of Downloads"), Size { folder: "Downloads".into() });
    }
    #[test]
    fn r36_count() {
        assert_eq!(p("count PDFs in Documents"), Count { src: set(Some("Documents"), of(Pdfs)) });
    }
    #[test]
    fn r37_duplicates() {
        assert_eq!(p("list duplicate files in Downloads"), Duplicates { folder: "Downloads".into() });
    }
    #[test]
    fn r38_open_folder() {
        assert_eq!(p("open Downloads in Finder"), Reveal { target: "Downloads".into() });
    }
    #[test]
    fn r39_reveal_file() {
        assert_eq!(p("reveal report.pdf in Finder"), Reveal { target: "report.pdf".into() });
    }
    #[test]
    fn r40_undo() {
        assert_eq!(p("undo last"), Undo);
    }
    #[test]
    fn r41_recent() {
        assert_eq!(p("show recent actions"), RecentActions);
    }

    #[test]
    fn spoken_file_and_folder_words_are_dropped_from_names() {
        let want = Trash { src: Source::Item { name: "claude.dmg".into(), folder: Some("downloads".into()) } };
        assert_eq!(p("Delete the claude.dmg file in downloads folder."), want);
        assert_eq!(p("delete the file claude.dmg in downloads"), want);
    }

    #[test]
    fn singular_folder_names_work_like_plural() {
        assert_eq!(
            p("clean up download older than 30 days"),
            Trash { src: set(Some("download"), Filter { older_than_days: Some(30), ..of(Files) }) }
        );
    }

    #[test]
    fn who_did_it_clauses_are_ignored() {
        assert_eq!(
            p("List all the screenshots I took in the last week"),
            List { src: set(None, Filter { newer_than_days: Some(7), ..of(Screenshots) }) }
        );
        assert_eq!(
            p("Show me the PDFs I downloaded yesterday."),
            List { src: set(None, Filter { day: Some(DayRef::Yesterday), ..of(Pdfs) }) }
        );
        assert_eq!(
            p("delete the installers that I've downloaded from Downloads"),
            Trash { src: set(Some("Downloads"), of(Installers)) }
        );
    }

    #[test]
    fn month_filters() {
        assert_eq!(
            p("Find screenshots taken in the month of august."),
            List { src: set(None, Filter { month: Some(8), ..of(Screenshots) }) }
        );
        assert_eq!(
            p("move PDFs from March 2025 to Archive"),
            Move { src: set(None, Filter { month: Some(3), year: Some(2025), ..of(Pdfs) }), to: "Archive".into() }
        );
        assert_eq!(
            p("list screenshots taken last week"),
            List { src: set(None, Filter { newer_than_days: Some(7), ..of(Screenshots) }) }
        );
        assert_eq!(
            p("delete installers downloaded in Sept from Downloads"),
            Trash { src: set(Some("Downloads"), Filter { month: Some(9), ..of(Installers) }) }
        );
    }

    #[test]
    fn unrecognized_input_is_not_understood() {
        for s in ["", "hello there", "make me a sandwich", "what's the weather", "move", "rename report.pdf"] {
            assert!(parse(s).is_err(), "should not understand {s:?}");
        }
    }

    #[test]
    fn relative_file_references_do_not_become_filename_searches() {
        for command in [
            "delete the last file",
            "Delete the last file.",
            "delete last files from Downloads",
            "delete the previous file in Downloads",
            "move the last file to Documents",
            "copy the last file to Desktop",
            "rename the last file to report.pdf",
            "list the last file",
            "delete the last one",
            "delete last",
        ] {
            assert!(parse(command).is_err(), "unresolved reference must not search: {command}");
        }
        assert_eq!(p("delete files from the last week"), Trash {
            src: set(None, Filter { newer_than_days: Some(7), ..of(Files) }),
        });
        assert_eq!(p("undo last"), Undo);
        assert_eq!(p("delete last.pdf"), Trash { src: item("last.pdf") });
        assert_eq!(p("delete \"last\""), Trash { src: item("last") });
        assert_eq!(p("delete files named \"last\" in Downloads"), Trash {
            src: set(Some("Downloads"), Filter { name_contains: Some("last".into()), ..of(Files) }),
        });
    }

    #[test]
    fn quoted_arguments_preserve_literal_names_and_command_words() {
        for name in ["Notes to Self.txt", "Chapter one.pdf", "first", "Month 13", "called notes", "My folder", "the report.pdf"] {
            assert_eq!(p(&format!("delete \"{name}\" in Downloads")), Trash {
                src: Source::Item { name: name.into(), folder: Some("Downloads".into()) },
            }, "{name}");
        }
        assert_eq!(p("move \"Notes to Self.txt\" to Documents"), Move { src: item("Notes to Self.txt"), to: "Documents".into() });
        assert_eq!(p("rename 'Notes to Self.txt' to 'Journal as of today.txt'"), Rename {
            src: item("Notes to Self.txt"), new_name: "Journal as of today.txt".into(),
        });
        assert_eq!(p("copy report.pdf to \"Terms and Conditions\""), Copy { src: item("report.pdf"), to: "Terms and Conditions".into() });
        for command in ["move Notes to Self.txt to Documents", "rename Notes to Self.txt to Journal.txt", "move \"unterminated to Documents"] {
            assert!(parse(command).is_err(), "{command}");
        }
    }

    #[test]
    fn missing_subjects_and_explicit_file_quantifiers_are_distinct() {
        for command in ["move the file to Documents", "copy file from Downloads to Desktop", "delete from Downloads"] {
            assert!(parse(command).is_err(), "{command}");
        }
        for command in ["move every file in Downloads to Documents", "move all the files from Downloads to Documents"] {
            assert_eq!(p(command), Move { src: set(Some("Downloads"), of(Files)), to: "Documents".into() });
        }
        assert_eq!(p("lowercase all the file names in Desktop"), Lowercase { src: set(Some("Desktop"), of(Files)) });
    }

    #[test]
    fn conflicting_or_repeated_filters_never_drop_constraints() {
        for command in [
            "delete files older than 30 days older than 90 days",
            "delete files larger than 1 MB larger than 5 MB",
            "list files before March 2025 before July 2026",
            "list files after March 2025 since July 2026",
            "delete files larger than 1 GB but smaller than 500 MB",
            "delete files bigger than 5 in Downloads",
            "delete files over 5 in Downloads",
            "delete files under 5 in Downloads",
            "delete files above 5 in Downloads",
            "delete files below 5 in Downloads",
            "delete files at least 5 in Downloads",
            "delete files at most 5 in Downloads",
            "delete files under 0 MB in Downloads",
        ] { assert!(parse(command).is_err(), "{command}"); }
        assert_eq!(p("list files between 5 and 10 MB in Downloads"), List {
            src: set(Some("Downloads"), Filter { larger_than_mb: Some(5.0), smaller_than_mb: Some(10.0), ..of(Files) }),
        });
    }

    #[test]
    fn unsupported_references_are_refused_in_read_only_commands_too() {
        for command in ["reveal the first file", "open the last file", "show me the second file in Finder", "what's the size of the largest PDF"] {
            assert!(parse(command).is_err(), "{command}");
        }
        assert_eq!(p("open \"first\""), Reveal { target: "first".into() });
        assert!(parse("undo the last delete").is_err());
        assert!(parse("undo the previous move").is_err());
        assert_eq!(p("undo the last change"), Undo);
    }

    #[test]
    fn dictated_forms_match_typed_forms() {
        assert_eq!(p("Please move the report.pdf to Documents."), p("move report.pdf to Documents"));
        assert_eq!(p("Undo that."), parse("undo last").unwrap());
        assert_eq!(p("Delete screenshots older than thirty days."), p("delete screenshots older than 30 days"));
    }
}
