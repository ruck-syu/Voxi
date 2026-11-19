use super::*;

fn notes(n: usize, noted: &[usize]) -> Vec<Option<String>> {
    (0..n).map(|i| noted.contains(&i).then(|| format!("clash {i}"))).collect()
}

#[test]
fn pages_continue_from_the_offset_and_stop_at_the_page_size() {
    let all = notes(300, &[]);
    let first = page_steps(&all, 300, PREVIEW_LIMIT, false);
    assert_eq!((first.len(), first[0]), (PREVIEW_PAGE, PREVIEW_LIMIT));
    let last = page_steps(&all, 300, PREVIEW_LIMIT + PREVIEW_PAGE, false);
    assert_eq!((last.len(), *last.last().unwrap()), (300 - PREVIEW_LIMIT - PREVIEW_PAGE, 299));
    assert!(page_steps(&all, 300, 300, false).is_empty());
}

#[test]
fn only_notes_keeps_each_noted_step() {
    let all = notes(300, &[3, 150, 299]);
    assert_eq!(page_steps(&all, 300, 0, true), [3, 150, 299]);
    assert_eq!(page_steps(&all, 300, 151, true), [299]);
}

#[test]
fn rows_are_described_on_request_with_their_note_and_step() {
    let dir = tempfile::tempdir().unwrap();
    let h = fsx::canonical(dir.path());
    let roots = Roots::new(&h, None, &[]);
    let actions: Vec<Action> =
        (0..3).map(|i| Action::Move { from: h.join(format!("Downloads/{i}.pdf")), to: h.join(format!("Documents/{i}.pdf")) }).collect();
    let source = PreviewSource::new(&actions, notes(3, &[1]), &roots, true);
    assert_eq!(source.noted(), 1);
    let item = source.item(&actions, 1).unwrap();
    assert_eq!((item.primary.as_str(), item.note.as_deref(), item.skippable), ("1.pdf", Some("clash 1"), true));
    assert!(source.item(&actions, 3).is_none());
    let json = serde_json::to_value(PreviewRow { step: 1, item }).unwrap();
    assert_eq!((json["step"].as_u64(), json["primary"].as_str()), (Some(1), Some("1.pdf")));
}
