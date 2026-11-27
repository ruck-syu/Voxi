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
        rule(IntentKind::Rename, "^relabel (.+)$", "rename $1 to pages", "relabel x.pdf", "rename x.pdf to pages"),
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
    for comment in ["Voxi vocabulary", "Voxi vocabulary schema=2 version=2", "Voxi vocabulary schema=1 version=1"] {
        assert_eq!(candidate().check_comment(comment), Err(Invalid::Signature));
    }
    let mut signature = BUNDLED_SIGNATURE.to_vec();
    signature.insert(20, 0);
    assert_eq!(verified_pack(BUNDLED, &signature).err(), Some(Invalid::Signature));
}

#[test]
fn compile_rejects_destructive_rewrites_and_invalid_capture_shapes() {
    let invalid = [
        rule(IntentKind::ImagesToPdf, r"^make 1 pdf from (.+)$", "delete $1", "make one PDF from x.pdf", "delete x.pdf"),
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
fn runtime_rejects_split_tokens_and_tries_the_next_rule() {
    let first = rule(
        IntentKind::ExtractPages,
        r"^pluck (\d)(?:2|) to (\d)(?:4|) from (.+)$",
        "extract pages $1 to $2 from $3",
        "pluck 1 to 3 from report.pdf",
        "extract pages 1 to 3 from report.pdf",
    );
    let second = rule(
        IntentKind::ExtractPages,
        r"^pluck (\d+) to (\d+) from (.+)$",
        "extract pages $1 to $2 from $3",
        "pluck 12 to 34 from report.pdf",
        "extract pages 12 to 34 from report.pdf",
    );
    // Even fixed suffix digits outside a capture cannot sever a whole token.
    let first = Rule::compile(&first).unwrap();
    assert!(first.rewrite("pluck 12 to 34 from report.pdf").is_none());
    let pack = Pack { version: 1, rules: vec![first, Rule::compile(&second).unwrap()] };
    assert_eq!(pack.rewrite("pluck 12 to 34 from report.pdf").unwrap().1, "extract pages 12 to 34 from report.pdf");
    let quote =
        rule(IntentKind::Rename, r"^relabel (.+) as (.+)$", "rename $1 to $2", "relabel x.pdf as final.pdf", "rename x.pdf to final.pdf");
    assert!(Rule::compile(&quote).unwrap().rewrite(r#"relabel "old as name.pdf" as final.pdf"#).is_some());
    assert!(Rule::compile(&quote).unwrap().rewrite(r#"relabel "old as name.pdf""#).is_none());
}

#[test]
fn hard_refusals_remain_terminal() {
    let file = rule(IntentKind::SplitPdf, r"^(?:chop|permanently delete) (.+)$", "split $1", "chop x.pdf", "split x.pdf");
    let rule = Rule::compile(&file).unwrap();
    assert!(rule.rewrite("permanently delete x.pdf").is_some());
    for original in ["permanently delete x.pdf", "Please permanently delete x.pdf."] {
        assert!(matches!(parse::parse_classified(original), Err(ParseFailure::HardRefusal(_))));
    }
}

#[test]
fn filter_semantics_are_preserved_and_use_parser_aliases() {
    let _date = corpus_tests::pin_regression_date();
    for subject in [
        "jpg in Scans",
        "jpg in Documents",
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
        let folder = if subject.ends_with("in Documents") { "Documents" } else { "Scans" };
        assert!(checked_output(IntentKind::List, &input, &format!("list files in {folder}")).is_none(), "{input}");
    }
    assert!(checked_output(IntentKind::ConvertImage, "recode image.png in Scans to webp", "convert image.png in Scans to png").is_none());
    assert!(checked_output(IntentKind::List, "survey files in Scans", "list PDFs in Scans").is_none());
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
