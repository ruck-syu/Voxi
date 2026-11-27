use super::*;
use crate::{model::Source, parse::corpus_tests};

fn candidate() -> PackFile {
    PackFile::decode(BUNDLED).unwrap()
}

fn rule(intent: IntentKind, pattern: &str, replace: &str, input: &str, expect: &str) -> RuleFile {
    RuleFile {
        id: "test-alias".into(),
        intent,
        pattern: pattern.into(),
        replace: replace.into(),
        examples: vec![Example { input: input.into(), expect: expect.into() }],
    }
}

#[test]
fn signature_rejects_tampered_pack_and_invalid_signature() {
    assert!(verified_pack(BUNDLED, BUNDLED_SIGNATURE).is_ok());
    let mut changed = BUNDLED.to_vec();
    changed.push(b' ');
    assert_eq!(verified_pack(&changed, BUNDLED_SIGNATURE).err(), Some(Invalid::Signature));
    assert_eq!(verified_pack(BUNDLED, b"bad signature").err(), Some(Invalid::Signature));
}

#[test]
fn version_and_compatibility_are_required() {
    let version = candidate().version;
    for floor in [version, version + 1] {
        assert_eq!(candidate().compile(&APP_VERSION, floor).err(), Some(Invalid::Rollback));
    }
    let mut pack = candidate();
    pack.min_app_version = Version::new(2, 0, 0);
    assert_eq!(pack.compile(&Version::new(1, 0, 0), 0).err(), Some(Invalid::AppVersion));
    let mut pack = candidate();
    pack.min_app_version = Version::new(2, 0, 0);
    assert!(pack.compile(&Version::new(0, 0, 0), 0).is_ok());
    assert_eq!(PackFile::decode(br#"{"schema":2,"version":1,"minAppVersion":"0.0.0","rules":[]}"#).err(), Some(Invalid::Schema));
}

#[test]
fn size_and_regex_budgets_are_enforced() {
    assert_eq!(verified_pack(&vec![b' '; MAX_PACK_BYTES + 1], BUNDLED_SIGNATURE).err(), Some(Invalid::Size));
    let mut pack = candidate();
    pack.rules.resize_with(MAX_RULES + 1, || rule(IntentKind::SplitPdf, "^chop (.+)$", "split $1", "chop x.pdf", "split x.pdf"));
    assert_eq!(pack.compile(&APP_VERSION, 0).err(), Some(Invalid::Size));
    for pattern in ["^chop (\\w{10000})$".into(), format!("^chop ({})$", "x".repeat(MAX_PATTERN_BYTES))] {
        assert!(Rule::compile(&rule(IntentKind::SplitPdf, &pattern, "split $1", "chop x.pdf", "split x.pdf")).is_none());
    }
}

#[test]
fn one_bad_rule_rejects_the_whole_pack() {
    for invalid in [
        rule(IntentKind::Move, "^relocate (.+)$", "move $1 to Archive", "relocate x.pdf", "move x.pdf to Archive"),
        rule(IntentKind::Move, "^relocate (.+)$", "move $1 to 1", "relocate x.pdf", "move x.pdf to 1"),
        rule(IntentKind::Rename, "^relabel (.+)$", "rename $1 to $1", "relabel x.pdf", "rename x.pdf to x.pdf"),
        rule(IntentKind::SplitPdf, "^chop (.+)$", "split $1", "chop", "split x.pdf"),
        rule(IntentKind::Copy, "^demonstrate (.+)$", "copy $1", "demonstrate x.pdf", "copy x.pdf"),
        rule(IntentKind::SplitPdf, "^chop (.+)$", "split $2", "chop x.pdf", "split x.pdf"),
    ] {
        let mut pack = candidate();
        pack.rules.push(invalid);
        assert_eq!(pack.compile(&APP_VERSION, 0).err(), Some(Invalid::Rule("test-alias".into())));
    }
}

#[test]
fn signature_binds_version_and_schema_and_rejects_nul() {
    let pack = candidate();
    for comment in [
        "Voxi vocabulary".into(),
        format!("Voxi vocabulary schema=2 version={}", pack.version),
        format!("Voxi vocabulary schema=1 version={}", pack.version - 1),
    ] {
        assert_eq!(pack.check_comment(&comment), Err(Invalid::Signature));
    }
    let mut signature = BUNDLED_SIGNATURE.to_vec();
    signature.insert(20, 0);
    assert_eq!(verified_pack(BUNDLED, &signature).err(), Some(Invalid::Signature));
}

#[test]
fn compile_rejects_destructive_rewrites_and_invalid_capture_shapes() {
    let invalid = [
        rule(IntentKind::ImagesToPdf, r"^build pdf from (.+)$", "delete $1", "build pdf from x.pdf", "delete x.pdf"),
        rule(IntentKind::ResizeImage, r"^squeeze (.+?(\d+)\.jpg)$", "resize $1 to $2", "squeeze scan800.jpg", "resize scan800.jpg to 800"),
        rule(
            IntentKind::ExtractPages,
            r"^pluck (\d)\d* to (\d)\d* from (.+)$",
            "extract pages $1 to $2 from $3",
            "pluck 12 to 34 from report.pdf",
            "extract pages 1 to 3 from report.pdf",
        ),
        rule(IntentKind::List, r"^purge .* in (.+)$", "list files in $1", "purge PDFs older than 90 days in Scans", "list files in Scans"),
        rule(IntentKind::List, r"^inspect \S in (.+)$", "list files in $1", "inspect x in Scans", "list files in Scans"),
        rule(IntentKind::List, r"^inspect [A-Z]+ in (.+)$", "list files in $1", "inspect x in Scans", "list files in Scans"),
        rule(IntentKind::Count, r"^tally files$", "count files", "tally files", "count files"),
        rule(IntentKind::List, r"^survey (.+) in (.+)$", "list files in $2", "survey PDFs in Scans", "list files in Scans"),
        rule(
            IntentKind::List,
            r"^survey pdfs older than 90 days in (.+)$",
            "list files in $1",
            "survey PDFs older than 90 days in Scans",
            "list files in Scans",
        ),
        rule(IntentKind::List, r"^survey pdfs in (.+)$", "list files in $1", "survey PDFs in Scans", "list files in Scans"),
        rule(IntentKind::Copy, r"^scan (.+)$", "list $1", "scan PDFs in Scans", "list PDFs in Scans"),
    ];
    for file in invalid {
        assert!(Rule::compile(&file).is_none(), "{}", file.pattern);
    }
    for intent in ["trash", "undo", "reveal", "chain"] {
        assert!(serde_json::from_value::<IntentKind>(serde_json::json!(intent)).is_err());
    }
}

#[test]
fn rejects_alternation_quantifiers_digits_and_flags_outside_captures() {
    for body in [
        "gather (?:report|taxes) in (.+)",
        "gather files in (?:scans|archive) toward (.+)",
        "gather (?:invoice files|files) in (.+)",
        "gather (.+) toward (?:archive|backup)",
        "squeeze (.+) from (?:800|1200) to (1)",
        "squeeze (.+) from 800 to (1)",
        "gather (?:all )?(.+)",
        "gather (?:all )+(.+)",
        "gather (?x:files in )(.+)",
        "gather (?-i:files in) (.+)",
        "gather ((?x:.+))",
        "gather ((?-i:.+))",
    ] {
        assert!(!pattern_allowed(&Parser::new().parse(body).unwrap(), false), "{body}");
    }
    assert!(pattern_allowed(&Parser::new().parse(r"gather (?:files in) ((?:Scans|Archive)\d*)").unwrap(), false));
}

#[test]
fn rejects_swapped_source_and_destination_captures() {
    let mut file = rule(
        IntentKind::Move,
        r"^relay (.+) from (.+) toward (.+)$",
        "move $1 from $2 to $3",
        "relay report.pdf from Scans toward Desktop",
        "move report.pdf from Scans to Desktop",
    );
    assert!(Rule::compile(&file).is_some());
    file.replace = "move $1 from $3 to $2".into();
    file.examples[0].expect = "move report.pdf from Desktop to Scans".into();
    assert!(Rule::compile(&file).is_none());
}

#[test]
fn rejects_trash_destinations_in_examples_and_at_runtime() {
    for (kind, verb) in [(IntentKind::Move, "move"), (IntentKind::Copy, "copy")] {
        let mut file = rule(
            kind,
            r"^relay (.+) toward (.+)$",
            &format!("{verb} $1 to $2"),
            "relay report.pdf toward Documents",
            &format!("{verb} report.pdf to Documents"),
        );
        let compiled = Rule::compile(&file).unwrap();
        for destination in ["Documents/Trash", "\"Trash\""] {
            assert!(compiled.rewrite(&format!("relay report.pdf toward {destination}")).is_some());
        }
        for destination in ["trash", "the bin", "Recycle Bin"] {
            let input = format!("relay report.pdf toward {destination}");
            assert!(compiled.rewrite(&input).is_none(), "{verb}: {input}");
            file.examples[0] = Example { input, expect: format!("{verb} report.pdf to {destination}") };
            assert!(Rule::compile(&file).is_none(), "{verb}: {destination}");
        }
    }

    let input = "check trash for PDFs in Downloads older than 90 days";
    let output = "move PDFs in Downloads to trash older than 90 days";
    assert!(matches!(parse::parse_vocab_output(output), Some(Intent::Move { .. })));
    let file = rule(IntentKind::Move, r"^check trash for (.+) (older than .+)$", "move $1 to trash $2", input, output);
    assert!(Rule::compile(&file).is_none());
    let captures = Regex::new(&file.pattern).unwrap().captures(input).unwrap();
    assert!(checked_output(IntentKind::Move, input, output, &captures).is_none());
}

#[test]
fn runtime_preserves_token_and_quote_boundaries_and_tries_the_next_rule() {
    let first =
        rule(IntentKind::Rename, r"^relabel (.+?) as (.+)$", "rename $1 to $2", "relabel x.pdf as final.pdf", "rename x.pdf to final.pdf");
    let second =
        rule(IntentKind::Rename, r"^relabel (.+) as (.+)$", "rename $1 to $2", "relabel x.pdf as final.pdf", "rename x.pdf to final.pdf");
    let first = Rule::compile(&first).unwrap();
    let pack = Pack { version: 1, rules: vec![first, Rule::compile(&second).unwrap()] };
    assert!(pack.rules[0].rewrite(r#"relabel "old as name.pdf" as final.pdf"#).is_none());
    assert_eq!(pack.rewrite(r#"relabel "old as name.pdf" as final.pdf"#).unwrap().1, r#"rename "old as name.pdf" to final.pdf"#);
    assert!(pack.rewrite(r#"relabel "old as name.pdf""#).is_none());

    let split = rule(IntentKind::SplitPdf, r"^chop (.+\.pdf)s?$", "split $1", "chop x.pdf", "split x.pdf");
    assert!(Rule::compile(&split).is_none());
    let split = rule(IntentKind::SplitPdf, r"^chop (.+\.pdf)s$", "split $1", "chop x.pdfs", "split x.pdf");
    assert!(Rule::compile(&split).is_none());
}

#[test]
fn hard_refusals_remain_terminal() {
    let refused = rule(IntentKind::SplitPdf, r"^permanently delete (.+)$", "split $1", "permanently delete x.pdf", "split x.pdf");
    assert!(Rule::compile(&refused).is_none());
}

#[test]
fn filter_semantics_are_preserved_and_use_parser_aliases() {
    let _date = corpus_tests::pin_regression_date();
    for subject in [
        "jpg in Scans",
        "jpg in Documents",
        "PDFs in Documents named Documents",
        "heic in Scans",
        "raw in Scans",
        "pngs in Scans",
        "PDFs older than 90 days in Scans",
        "PDFs newer than 3 days in Scans",
        "PDFs in Scans over 5 MB and under 8 MB",
        "files ending with .abc in Scans",
        "PDFs from yesterday in Scans",
        "October PDFs in Scans",
        "PDFs in Scans from July 2026",
        "PDFs in Scans since March 2026 and before July 2026",
        "PDFs modified yesterday in Scans",
        "PDFs in Scans named receipts",
        "PDFs in Scans starting with draft",
    ] {
        let input = format!("survey {subject}");
        let output = format!("list {subject}");
        let file = rule(IntentKind::List, r"^survey (.+)$", "list $1", &input, &output);
        let compiled = Rule::compile(&file).unwrap_or_else(|| panic!("should compile: {input}"));
        assert!(compiled.rewrite(&input).is_some());
        let folder = if subject.contains("in Documents") { "Documents" } else { "Scans" };
        let captures = compiled.pattern.captures(&input).unwrap();
        assert!(checked_output(IntentKind::List, &input, &format!("list files in {folder}"), &captures).is_none(), "{input}");
    }
    let input = "survey files in Scans";
    let pattern = Regex::new(r"^survey (.+)$").unwrap();
    let captures = pattern.captures(input).unwrap();
    assert!(checked_output(IntentKind::List, input, "list PDFs in Scans", &captures).is_none());
}

#[test]
fn accepts_folder_captures_number_templates_and_spoken_characters() {
    for file in [
        rule(
            IntentKind::Move,
            r"^relay (.+) toward (.+)$",
            "move $1 to $2",
            "relay report.pdf toward Documents",
            "move report.pdf to Documents",
        ),
        rule(
            IntentKind::Number,
            r"^enumerate (.+) like (.+)$",
            "number $1 as $2",
            "enumerate images in Trip like trip-001",
            "number images in Trip as trip-001",
        ),
        rule(
            IntentKind::ReplaceInNames,
            r"^substitute (.+) for (.+) across (.+)$",
            "replace $1 with $2 in $3",
            "substitute spaces for dashes across files in Downloads",
            "replace spaces with dashes in files in Downloads",
        ),
    ] {
        let compiled = Rule::compile(&file).unwrap_or_else(|| panic!("should compile: {}", file.pattern));
        let example = &file.examples[0];
        assert_eq!(compiled.rewrite(&example.input).unwrap().0, parse::parse_builtin(&example.expect).unwrap());
    }
}

pub(crate) fn with_test_pack<T>(json: &str, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Pack>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PACK.replace(self.0.take());
        }
    }
    let pack = PackFile::decode(json.as_bytes()).unwrap().compile(&APP_VERSION, 0).unwrap();
    let _restore = Restore(TEST_PACK.replace(Some(pack)));
    run()
}

#[test]
fn bundled_pack_rewrites_each_seed_to_its_existing_intent() {
    let pack = verified_pack(BUNDLED, BUNDLED_SIGNATURE).unwrap().compile(&APP_VERSION, 0).unwrap();
    let actual: Vec<_> = candidate()
        .rules
        .iter()
        .flat_map(|rule| rule.examples.iter().map(move |example| (&rule.id, example)))
        .map(|(id, example)| {
            assert_eq!(parse::parse_builtin(&example.input), Err(ParseFailure::GrammarMiss), "{id}");
            let input = phrase::normalize(&example.input);
            assert_eq!(pack.rewrite(&input).map(|(intent, _)| intent), parse::parse_builtin(&example.expect).ok(), "{id}");
            parse::parse_classified(&example.input).unwrap()
        })
        .collect();
    assert!(matches!(&actual[0], Intent::ImagesToPdf { src: Source::Set { folder: Some(folder), filter }, .. }
        if folder == "Scans" && filter.name_contains.as_deref() == Some("receipts")));
    assert_eq!(actual[1], Intent::SplitPdf { src: Source::item("report.pdf") });
    assert_eq!(actual[2], Intent::Unzip { src: Source::item("archive.zip"), destination: Some("Downloads/archive".into()) });
    assert_eq!(
        parse::parse_classified(r#"BREAK "Quarterly Report.PDF" INTO PAGES"#),
        Ok(Intent::SplitPdf { src: Source::item("Quarterly Report.PDF") })
    );
}

#[test]
fn bundled_pack_preserves_existing_corpus_results() {
    let _date = corpus_tests::pin_regression_date();
    for (command, _, at) in corpus_tests::regression_cases() {
        let baseline = parse::parse_builtin(&command);
        if baseline != Err(ParseFailure::GrammarMiss) {
            assert_eq!(parse::parse_classified(&command), baseline, "{at}: {command}");
        }
    }
}

#[test]
fn refresh_interval_is_shared_across_launches() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(check_due(dir.path(), 3600), Duration::ZERO);
    atomic_write(&dir.path().join("vocab.last-check"), b"3600").unwrap();
    assert_eq!(check_due(dir.path(), 3601), CHECK_INTERVAL - Duration::from_secs(1));
    assert_eq!(check_due(dir.path(), 90_000), Duration::ZERO);
    assert!(client().is_ok(), "the TLS client must work before the app updater runs");
}
