use super::{ParseFailure, RefusalReason, parse, parse_classified};

#[test]
fn command_byte_budget_rejects_raw_input_before_cleanup() {
    use super::{MAX_COMMAND_BYTES, check_length};
    let too_long = ParseFailure::HardRefusal(RefusalReason::TooLong);
    // These would become tiny valid commands after whitespace cleanup.
    for text in [
        "x".repeat(MAX_COMMAND_BYTES + 1),
        "🦀".repeat(MAX_COMMAND_BYTES / 4 + 1),
        format!("{}undo", " ".repeat(MAX_COMMAND_BYTES)),
        format!("{}undo", "\u{3000}".repeat(MAX_COMMAND_BYTES / 3 + 1)),
    ] {
        assert_eq!(check_length(&text), Err(too_long));
        assert_eq!(parse_classified(&text), Err(too_long));
        assert_eq!(parse(&text), Err("That command is too long.".into()));
        assert!(!check_length(&text).unwrap_err().allows_fallback());
        assert_eq!(super::parse_single_classified(&text), Err(too_long));
        assert_eq!(super::chain::parse(&text), Some(Err(too_long)));
    }
}

#[test]
fn command_byte_budget_preserves_exact_cap_and_utf8_boundaries() {
    use super::{MAX_COMMAND_BYTES, check_length};
    // No truncation: the cap falls inside the final emoji in the over-cap
    // case, and rejection must not try to slice at that byte offset.
    let prefix = "a".repeat(MAX_COMMAND_BYTES - 1);
    let over = format!("{prefix}🦀");
    assert_eq!(parse_classified(&over), Err(ParseFailure::HardRefusal(RefusalReason::TooLong)));
    for padding in [" ".repeat(MAX_COMMAND_BYTES - 4), "\u{3000}".repeat((MAX_COMMAND_BYTES - 4) / 3)] {
        let exact = format!("{padding}undo");
        assert_eq!(exact.len(), MAX_COMMAND_BYTES);
        assert_eq!(check_length(&exact), Ok(()));
        assert!(matches!(parse_classified(&exact), Ok(crate::model::Intent::Undo)));
    }
}

#[test]
fn oversized_single_token_is_refused_within_100ms() {
    let text = "x".repeat(200_000);
    let start = std::time::Instant::now();
    let failure = parse_classified(&text);
    let message = parse(&text);
    let elapsed = start.elapsed();
    assert_eq!(failure, Err(ParseFailure::HardRefusal(RefusalReason::TooLong)));
    assert_eq!(message, Err("That command is too long.".into()));
    assert!(elapsed < std::time::Duration::from_millis(100), "length refusal took {elapsed:?}");
}

#[test]
fn chain_byte_budget_is_aggregate_even_with_only_eight_commands() {
    use super::{MAX_COMMAND_BYTES, check_length};
    let command = format!("create folder named \"{}\" in Downloads", "🦀".repeat(520));
    assert_eq!(check_length(&command), Ok(()));
    let chain = std::iter::repeat_n(command.as_str(), 8).collect::<Vec<_>>().join(" then ");
    assert!(chain.len() > MAX_COMMAND_BYTES);
    let refused = ParseFailure::HardRefusal(RefusalReason::TooLong);
    assert_eq!(parse_classified(&chain), Err(refused));
    assert_eq!(super::chain::parse(&chain), Some(Err(refused)));
}
use crate::model::{FileType, Filter, Intent, Source};

#[test]
fn vague_references_in_a_chain_are_refused_not_searched() {
    for command in ["copy notes.pdf to Backup and delete the original", "copy notes.pdf to Backup and delete the copy"] {
        assert_eq!(parse_classified(command), Err(ParseFailure::HardRefusal(RefusalReason::UnresolvedReference)), "{command}");
    }
    // A verb-final second action (no trailing whitespace) is still a chain,
    // and an incomplete clause is refused rather than silently dropped.
    assert!(matches!(parse_classified("copy notes.pdf to Backup and delete"), Err(ParseFailure::HardRefusal(_))));
}

#[test]
fn bare_calendar_words_after_a_name_stay_in_the_item_name() {
    assert_eq!(parse("delete Notes 3 Jan").unwrap(), Intent::Trash { src: Source::item("Notes 3 Jan") });
    assert_eq!(
        parse("move Invoice 12 May to Archive").unwrap(),
        Intent::Move { src: Source::item("Invoice 12 May"), to: "Archive".into() }
    );
    // A plural type noun still introduces a date filter.
    let value = serde_json::to_value(parse("list PDFs 3 Jan").unwrap()).unwrap();
    assert_eq!(value["src"]["filter"]["since"], "2026-01-03");
    // An impossible day is still refused rather than becoming a name.
    assert!(matches!(parse_classified("move photos from February 29 2026 to Archive"), Err(ParseFailure::HardRefusal(_))));
}

#[test]
fn unknown_qualifiers_never_become_name_filters() {
    for command in ["move unread PDFs to Archive", "move awl PDFs to Archive", "count up PDFs"] {
        let failure = parse_classified(command).unwrap_err();
        assert_eq!(failure, ParseFailure::GrammarMiss, "{command}");
        assert!(parse(command).is_err(), "{command}");
    }
    // An explicit quoted name filter still works.
    let value = serde_json::to_value(parse("move \"unread\" PDFs to Archive").unwrap()).unwrap();
    assert_eq!(value["src"]["filter"]["nameContains"], "unread");
}

#[test]
fn destination_references_bind_with_or_without_the() {
    for tail in ["to the new folder", "to new folder", "to the created folder"] {
        let command = format!("create a folder called Backup and copy notes.pdf {tail}");
        let Intent::Chain { commands } = parse(&command).unwrap() else { panic!("{command}") };
        assert_eq!(commands.len(), 2, "{command}");
        assert!(
            matches!(&commands[1], Intent::Copy { src: Source::Item { name, folder: None }, to }
            if name == "notes.pdf" && to == "Desktop/Backup"),
            "{command}: {:?}",
            commands[1]
        );
    }
}

#[test]
fn unscoped_chain_sources_keep_the_single_command_default_scope() {
    let Intent::Chain { commands } = parse("create a folder named Archive and move PDFs into it").unwrap() else { panic!() };
    assert!(
        matches!(&commands[1], Intent::Move {
        src: Source::Set { folder: None, filter },
        to,
    } if filter.file_type == Some(FileType::Pdfs) && to == "Desktop/Archive"),
        "{:?}",
        commands[1]
    );
    let Intent::Chain { commands } = parse("create a folder named Archive and move screenshots into it").unwrap() else { panic!() };
    assert!(
        matches!(&commands[1], Intent::Move {
        src: Source::Set { folder: None, filter },
        ..
    } if filter.file_type == Some(FileType::Screenshots)),
        "{:?}",
        commands[1]
    );
    // An explicit destination parent does not narrow the default source.
    let Intent::Chain { commands } = parse("create a folder in Downloads called PDFS and move PDFs into it").unwrap() else { panic!() };
    assert!(matches!(&commands[1], Intent::Move { src: Source::Set { folder: None, .. }, .. }), "{:?}", commands[1]);
}

#[test]
fn comma_before_an_action_verb_splits_the_chain() {
    let Intent::Chain { commands } = parse("move notes.pdf to Archive, delete old.pdf").unwrap() else { panic!() };
    assert_eq!(commands.len(), 2);
    assert!(matches!(&commands[0], Intent::Move { to, .. } if to == "Archive"));
    assert!(matches!(&commands[1], Intent::Trash { src: Source::Item { name, .. } } if name == "old.pdf"));

    let Intent::Chain { commands } = parse("rename a.pdf to b.pdf, delete c.pdf").unwrap() else { panic!() };
    assert_eq!(commands.len(), 2);
    assert!(matches!(&commands[0], Intent::Rename { new_name, .. } if new_name == "b.pdf"));
    assert!(matches!(&commands[1], Intent::Trash { src: Source::Item { name, .. } } if name == "c.pdf"));

    // A leading scope's comma is not a chain separator.
    assert!(matches!(parse("from Downloads, move all the PDFs over to Documents").unwrap(), Intent::Move { .. }));
    assert!(matches!(parse("real quick, copy the whole Project folder to Backup").unwrap(), Intent::Copy { .. }));
}

#[test]
fn literal_permanent_and_question_filename_tokens_are_allowed() {
    assert_eq!(
        parse("move Permanent Resident Card.pdf to Documents").unwrap(),
        Intent::Move { src: Source::item("Permanent Resident Card.pdf"), to: "Documents".into() }
    );
    assert_eq!(parse("move What?.pdf to Archive").unwrap(), Intent::Move { src: Source::item("What?.pdf"), to: "Archive".into() });
    for command in
        ["delete report.pdf permanently", "delete report.pdf for good", "delete report.pdf permanent", "permanent delete report.pdf"]
    {
        assert!(matches!(parse_classified(command), Err(ParseFailure::HardRefusal(_))), "{command}");
    }
}

#[test]
fn permanence_synonyms_are_hard_refusals() {
    for command in [
        "erase notes.pdf",
        "erase the trash",
        "emty the trash",
        "delete notes.pdf without the bin",
        "delete notes.pdf so it can't be recovered",
        "delete notes.pdf for real",
        "wipe Downloads",
        "shred the old screenshots on the Desktop",
    ] {
        assert!(matches!(parse_classified(command), Err(ParseFailure::HardRefusal(_))), "{command}");
        assert!(!parse_classified(command).unwrap_err().allows_fallback(), "{command}");
    }
}

#[test]
fn destructive_verb_corpus_inputs_are_hard_refusals() {
    for command in [
        "wipe Downloads",
        "delete the largest screenshot",
        "delete junk in Downloads",
        "delete everything important from Desktop",
        "delete notes.pdf then empty the trash",
        "send report.pdf to bob@example.com",
    ] {
        assert!(matches!(parse_classified(command), Err(ParseFailure::HardRefusal(_))), "{command}");
    }
}

fn set(folder: &str, file_type: FileType, ext: Option<&str>) -> Source {
    Source::Set {
        folder: Some(folder.into()),
        filter: Filter { file_type: Some(file_type), ext: ext.map(|s| vec![s.into()]), ..Default::default() },
    }
}

#[test]
fn archive_and_explicit_extension_filters_stay_narrow() {
    assert_eq!(
        parse("move archive files from Downloads to External").unwrap(),
        Intent::Move { src: set("Downloads", FileType::Archives, None), to: "External".into() }
    );
    assert_eq!(parse("delete dmg files from Downloads").unwrap(), Intent::Trash { src: set("Downloads", FileType::Files, Some("dmg")) });
    assert!(matches!(
        parse("delete installers from Downloads"),
        Ok(Intent::Trash { src: Source::Set { filter: Filter { file_type: Some(FileType::Installers), ext: None, .. }, .. } })
    ));
    assert_eq!(
        parse("move \"Archive files\" to External").unwrap(),
        Intent::Move { src: Source::item("Archive files"), to: "External".into() }
    );
    assert_eq!(
        parse("delete \"DMG files\" from Downloads").unwrap(),
        Intent::Trash { src: Source::item("DMG files").pipe_folder("Downloads") }
    );
    let (base, when) = crate::scheduler::extract_schedule("delete dmg files from Downloads every 3 days", chrono::Local::now()).unwrap();
    assert_eq!(base, "delete dmg files from Downloads");
    assert_eq!(when.unwrap().rule.unwrap().every, 3);
    assert_eq!(parse(&base).unwrap(), Intent::Trash { src: set("Downloads", FileType::Files, Some("dmg")) });
}

#[test]
fn numbered_examples_are_templates_and_keep_their_width() {
    let intent = parse("number images in Camera Roll as \"shot-001\", \"shot-002\"").unwrap();
    assert_eq!(intent, Intent::Number { src: set("Camera Roll", FileType::Images, None), base: "shot-".into(), width: 3 });
    assert_eq!(
        parse("number images in Camera Roll as shot-01").unwrap(),
        Intent::Number { src: set("Camera Roll", FileType::Images, None), base: "shot-".into(), width: 2 }
    );
    assert_eq!(
        parse("number images in Trip as trip-001").unwrap(),
        Intent::Number { src: set("Trip", FileType::Images, None), base: "trip-".into(), width: 3 }
    );
}

#[test]
fn unsupported_operations_and_underspecified_folders_are_terminal() {
    for command in ["search the web for orchid care", "replace x with y in file contents", "make a new folder somewhere for these files"] {
        let failure = parse_classified(command).unwrap_err();
        assert!(matches!(failure, ParseFailure::HardRefusal(_)), "{command}: {failure:?}");
        assert!(!failure.allows_fallback(), "{command}");
    }
    assert_eq!(parse("list files in web").unwrap(), Intent::List { src: set("web", FileType::Files, None) });
    assert!(parse("move \"file contents\" to Documents").is_ok());
    assert!(parse("make a folder named \"somewhere for these files\"").is_ok());
}

#[test]
fn filters_after_move_destination_cannot_become_folder_names() {
    assert_eq!(
        parse("move files from Downloads to Desktop between 10 and 50 MB").unwrap(),
        Intent::Move {
            src: Source::Set {
                folder: Some("Downloads".into()),
                filter: Filter {
                    file_type: Some(FileType::Files),
                    larger_than_mb: Some(10.0),
                    smaller_than_mb: Some(50.0),
                    ..Default::default()
                }
            },
            to: "Desktop".into(),
        }
    );
    assert_eq!(
        parse("move files from Downloads to Archive older than 90 days").unwrap(),
        Intent::Move {
            src: Source::Set {
                folder: Some("Downloads".into()),
                filter: Filter { file_type: Some(FileType::Files), older_than_days: Some(90), ..Default::default() }
            },
            to: "Archive".into(),
        }
    );
    for command in [
        "move files from Downloads to Archive after 3 days",
        "move files from Downloads to Archive larger than 5 MB and smaller than 2 MB",
        "move files from Downloads to Archive from February 30 2026",
        "move files from Downloads to Archive between 10 and 50",
    ] {
        assert_eq!(parse_classified(command), Err(ParseFailure::HardRefusal(RefusalReason::InvalidConstraints)), "{command}");
    }
    assert_eq!(
        parse("move files from Downloads to \"Archive after 3 days\"").unwrap(),
        Intent::Move { src: set("Downloads", FileType::Files, None), to: "Archive after 3 days".into() }
    );
}

trait SourceFixtureExt {
    fn pipe_folder(self, folder: &str) -> Self;
}
impl SourceFixtureExt for Source {
    fn pipe_folder(self, folder: &str) -> Self {
        match self {
            Source::Item { name, .. } => Source::Item { name, folder: Some(folder.into()) },
            other => other,
        }
    }
}
