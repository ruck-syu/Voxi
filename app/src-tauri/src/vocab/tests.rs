use super::*;
use crate::{model::Source, parse::corpus_tests};

fn candidate() -> PackFile {
    PackFile::decode(BUNDLED).unwrap()
}

fn rule(pattern: &str, replace: &str, input: &str, expect: &str) -> RuleFile {
    RuleFile {
        id: "test-alias".into(),
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
    pack.min_app_version = Version::new(APP_VERSION.major + 1, 0, 0);
    assert_eq!(pack.compile(&APP_VERSION, 0).err(), Some(Invalid::AppVersion));
    assert_eq!(PackFile::decode(br#"{"schema":2,"version":1,"minAppVersion":"0.0.0","rules":[]}"#).err(), Some(Invalid::Schema));
}

#[test]
fn size_and_regex_budgets_are_enforced() {
    assert_eq!(verified_pack(&vec![b' '; MAX_PACK_BYTES + 1], BUNDLED_SIGNATURE).err(), Some(Invalid::Size));
    let mut pack = candidate();
    pack.rules.resize_with(MAX_RULES + 1, || rule("^chop (.+)$", "split $1", "chop x.pdf", "split x.pdf"));
    assert_eq!(pack.compile(&APP_VERSION, 0).err(), Some(Invalid::Size));
    for pattern in ["^chop (\\w{10000})$".into(), format!("^chop ({})$", "x".repeat(MAX_PATTERN_BYTES))] {
        assert!(Rule::compile(&rule(&pattern, "split $1", "chop x.pdf", "split x.pdf")).is_none());
    }
}

#[test]
fn one_bad_rule_rejects_the_whole_pack() {
    for invalid in [
        rule("^relocate (.+)$", "move $1 to Archive", "relocate x.pdf", "move x.pdf to Archive"),
        rule("^relocate (.+)$", "move $1 to 1", "relocate x.pdf", "move x.pdf to 1"),
        rule("^relabel (.+)$", "rename $1 to pages", "relabel x.pdf", "rename x.pdf to pages"),
        rule("^chop (.+)$", "split $1", "chop", "split x.pdf"),
        rule("^demonstrate (.+)$", "copy $1", "demonstrate x.pdf", "copy x.pdf"),
        rule("^chop (.+)$", "split $2", "chop x.pdf", "split x.pdf"),
    ] {
        let mut pack = candidate();
        pack.rules.push(invalid);
        assert_eq!(pack.compile(&APP_VERSION, 0).err(), Some(Invalid::Rule("test-alias".into())));
    }
}

#[test]
fn valid_example_cannot_make_a_refused_variant_eligible() {
    let mut pack = candidate();
    // The hard refusal is not included among the examples used to load this rule.
    pack.rules.push(rule(r"^(?:jettison|permanently delete) (.+)$", "delete $1", "jettison x.pdf", "delete x.pdf"));
    let pack = pack.compile(&APP_VERSION, 0).unwrap();
    assert_eq!(parse::parse_builtin("jettison x.pdf"), Err(ParseFailure::GrammarMiss));
    assert_eq!(parse::parse_with_vocab("jettison x.pdf", |input| pack.rewrite(input)), Ok(Intent::Trash { src: Source::item("x.pdf") }));
    for original in ["permanently delete x.pdf", "Please permanently delete x.pdf."] {
        assert!(matches!(parse::parse_with_vocab(original, |_| panic!("refusal reached vocabulary")), Err(ParseFailure::HardRefusal(_))));
    }

    let mut pack = candidate();
    pack.rules.push(rule("^permanently delete (.+)$", "delete $1", "permanently delete x.pdf", "delete x.pdf"));
    assert_eq!(pack.compile(&APP_VERSION, 0).err(), Some(Invalid::Rule("test-alias".into())));
}

#[test]
fn examples_do_not_authorize_invalid_or_invented_arguments_at_runtime() {
    let mut pack = candidate();
    pack.rules.push(rule(
        r"^relabel (.+?) (?:tagged pages|untagged)$",
        "rename $1 to pages",
        "relabel x.pdf tagged pages",
        "rename x.pdf to pages",
    ));
    let pack = pack.compile(&APP_VERSION, 0).unwrap();
    assert_eq!(parse::parse_with_vocab("relabel x.pdf untagged", |input| pack.rewrite(input)), Err(ParseFailure::GrammarMiss));
}

#[test]
fn bundled_pack_rewrites_each_seed_to_its_existing_intent() {
    let pack = verified_pack(BUNDLED, BUNDLED_SIGNATURE).unwrap().compile(&APP_VERSION, 0).unwrap();
    let actual: Vec<_> = candidate()
        .rules
        .iter()
        .map(|rule| {
            let example = &rule.examples[0];
            let input = phrase::normalize(&example.input);
            assert_eq!(pack.rewrite(&input), parse::parse_builtin(&example.expect).ok(), "{}", rule.id);
            parse::parse_classified(&example.input).unwrap()
        })
        .collect();
    assert!(matches!(&actual[0], Intent::ImagesToPdf { src: Source::Set { folder: Some(folder), filter }, .. }
        if folder == "Scans" && filter.name_contains.as_deref() == Some("receipts")));
    assert_eq!(actual[1], Intent::SplitPdf { src: Source::item("report.pdf") });
    assert_eq!(actual[2], Intent::ExtractPages { src: Source::item("report.pdf"), from: 2, to: 5, output: None });
    assert_eq!(actual[3], Intent::Unzip { src: Source::item("archive.zip"), destination: Some("Downloads/archive".into()) });
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
