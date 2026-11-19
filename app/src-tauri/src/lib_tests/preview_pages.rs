use super::*;

fn items(n: usize, noted: &[usize]) -> Vec<PreviewItem> {
    (0..n)
        .map(|i| PreviewItem {
            kind: "move".into(),
            primary: format!("file {i}.pdf"),
            detail: "→ ~/Documents".into(),
            note: noted.contains(&i).then(|| format!("clash {i}")),
        })
        .collect()
}

#[test]
fn pages_continue_from_the_offset_and_stop_at_the_page_size() {
    let all = items(300, &[]);
    let first = preview_rows(&all, PREVIEW_LIMIT, false);
    assert_eq!(first.len(), PREVIEW_PAGE);
    assert_eq!((first[0].step, first[0].item.primary.as_str()), (PREVIEW_LIMIT, "file 20.pdf"));
    let last = preview_rows(&all, PREVIEW_LIMIT + PREVIEW_PAGE, false);
    assert_eq!(last.len(), 300 - PREVIEW_LIMIT - PREVIEW_PAGE);
    assert_eq!(last.last().unwrap().step, 299);
    assert!(preview_rows(&all, 300, false).is_empty());
}

#[test]
fn only_notes_keeps_each_noted_step_and_its_number() {
    let all = items(300, &[3, 150, 299]);
    let rows = preview_rows(&all, 0, true);
    assert_eq!(rows.iter().map(|r| r.step).collect::<Vec<_>>(), [3, 150, 299]);
    assert_eq!(rows[1].item.note.as_deref(), Some("clash 150"));
    assert_eq!(preview_rows(&all, 151, true).iter().map(|r| r.step).collect::<Vec<_>>(), [299]);
}

#[test]
fn a_row_serializes_as_a_preview_item_with_its_step() {
    let row = PreviewRow { step: 7, item: items(1, &[0]).remove(0) };
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(json["step"], 7);
    assert_eq!(json["primary"], "file 0.pdf");
    assert_eq!(json["note"], "clash 0");
}
