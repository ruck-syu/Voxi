use super::*;

#[test]
fn preview_shows_the_source_folder_for_transfers_and_trash() {
    let temp = tempfile::tempdir().unwrap();
    let home = fsx::canonical(temp.path());
    let roots = Roots::new(&home, None, &[]);
    for (action, kind, primary, source) in [
        (Action::Move { from: home.join("Downloads/a.pdf"), to: home.join("Documents/a.pdf") }, "move", "a.pdf", "Downloads"),
        (Action::Copy { from: home.join("Pictures/b.png"), to: home.join("Backup/b.png") }, "copy", "b.png", "Pictures"),
        (Action::Trash { path: home.join("Desktop/c.txt") }, "trash", "c.txt", "Desktop"),
    ] {
        let (got_kind, got_primary, detail) = describe(&action, &roots);
        assert_eq!((got_kind.as_str(), got_primary.as_str()), (kind, primary));
        assert!(detail.contains(source), "{kind}: {detail}");
    }
}

#[cfg(feature = "tiny-model")]
#[test]
fn schedule_cross_check_matches_the_scheduler_and_named_weekdays() {
    use chrono::TimeZone;
    let now = chrono::Local.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).single().unwrap();
    let text = "move PDFs from Downloads to Archive at 21:00";
    let (_, chosen) = scheduler::extract_schedule(text, now).unwrap();
    assert!(model_schedule_ok(&chosen, chosen.as_ref(), text));
    let wrong = chosen.clone().map(|mut w| {
        w.due += chrono::Duration::hours(1);
        w
    });
    assert!(!model_schedule_ok(&wrong, chosen.as_ref(), text));
    // A schedule only in the plan, or only in the text, is rejected.
    assert!(!model_schedule_ok(&chosen, None, "move PDFs from Downloads to Archive"));
    assert!(!model_schedule_ok(&None, chosen.as_ref(), text));

    let monday_text = "move PDFs to Archive every monday at 9:00";
    let (_, monday) = scheduler::extract_schedule(monday_text, now).unwrap();
    assert_eq!(named_weekday(monday_text), Some(chrono::Weekday::Mon));
    assert!(model_schedule_ok(&monday, monday.as_ref(), monday_text));
    let (_, tuesday) = scheduler::extract_schedule("move PDFs to Archive every tuesday at 9:00", now).unwrap();
    assert!(!model_schedule_ok(&tuesday, monday.as_ref(), monday_text));
    assert_eq!(named_weekday("rename monday notes.pdf to done.pdf"), Some(chrono::Weekday::Mon));
}
