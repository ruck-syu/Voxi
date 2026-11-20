#[test]
fn suggested_examples_parse() {
    let list: Vec<String> = serde_json::from_str(include_str!("../../../src/examples.json")).unwrap();
    for ex in list {
        let (rest, when) = crate::scheduler::extract_schedule(&ex, chrono::Local::now()).unwrap();
        let timed = ex.contains(" every ") || ex.contains(" at ");
        assert_eq!(when.is_some(), timed, "{ex}: schedule");
        assert!(crate::parse::parse(&rest).is_ok(), "{ex}: doesn't parse");
    }
}
