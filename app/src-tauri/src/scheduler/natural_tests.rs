use super::*;

fn now() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 18, 7, 0, 0).single().unwrap()
}

#[test]
fn calendar_and_prefix_schedules_keep_destinations_clean() {
    let base = "move deck.pptx from Desktop to Documents";
    for (suffix, date, hour) in [
        ("on Friday at 2pm", (2026, 11, 20), 14),
        ("next Monday at 10am", (2026, 11, 23), 10),
        ("tonight at 8pm", (2026, 11, 18), 20),
        ("tonight at 8", (2026, 11, 18), 20),
        ("this evening at 8", (2026, 11, 18), 20),
        ("this afternoon at 2", (2026, 11, 18), 14),
        ("this morning at 8", (2026, 11, 18), 8),
        ("on November 20 at 2pm", (2026, 11, 20), 14),
        ("on 2026-11-20 at 2pm", (2026, 11, 20), 14),
        ("tmrw at 9am", (2026, 11, 19), 9),
    ] {
        let (command, when) = extract_schedule(&format!("{base} {suffix}"), now()).unwrap();
        assert_eq!(command, base, "{suffix}");
        let when = when.unwrap();
        assert_eq!(when.due, Local.with_ymd_and_hms(date.0, date.1, date.2, hour, 0, 0).single().unwrap(), "{suffix}");
        assert!(when.rule.is_none());
    }
    for (input, base, minutes) in [
        ("in 10 minutes move lease.pdf from Downloads to Documents", "move lease.pdf from Downloads to Documents", 10),
        ("at 5 pm move deck.pptx from Desktop to Documents", base, 600),
        ("tomorrow at 9am create a folder called X in Documents", "create a folder called X in Documents", 1560),
        ("move deck.pptx from Desktop to Documents in 2 hours and 30 minutes", base, 150),
        ("in 2 hrs and 30 mins move deck.pptx from Desktop to Documents", base, 150),
        ("move deck.pptx from Desktop to Documents in 5 mins", base, 5),
    ] {
        let (command, when) = extract_schedule(input, now()).unwrap();
        assert_eq!(command, base);
        assert_eq!(when.unwrap().due, now() + Duration::minutes(minutes));
    }
}

#[test]
fn repeats_have_exact_rules_and_first_runs() {
    let base = "move all images from Desktop to Pictures";
    for (suffix, unit, every, days, hour) in [
        ("weekly on Friday at 5pm", Unit::Weeks, 1, 2, 17),
        ("every morning at 8", Unit::Days, 1, 0, 8),
        ("each morning at 8", Unit::Days, 1, 0, 8),
        ("every evening at 8", Unit::Days, 1, 0, 20),
        ("evryday at 9am", Unit::Days, 1, 0, 9),
        ("evry 2 hours", Unit::Hours, 2, 0, 9),
        ("every day", Unit::Days, 1, 1, 7),
    ] {
        let (command, when) = extract_schedule(&format!("{base} {suffix}"), now()).unwrap();
        assert_eq!(command, base);
        let when = when.unwrap();
        assert_eq!(when.due, Local.with_ymd_and_hms(2026, 11, 18 + days, hour, 0, 0).single().unwrap());
        let rule = when.rule.unwrap();
        assert_eq!((rule.unit, rule.every, rule.command.as_str()), (unit, every, base));
        assert_eq!(rule.time, if unit == Unit::Hours { None } else { NaiveTime::from_hms_opt(hour, 0, 0) });
    }
    let base = "create a folder called X in Documents and move a.pdf from Downloads into it";
    let (command, when) = extract_schedule(&format!("every monday {base}"), now()).unwrap();
    assert_eq!(command, base);
    let when = when.unwrap();
    assert_eq!(when.due, Local.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).single().unwrap());
    assert_eq!(when.rule.unwrap().unit, Unit::Weeks);
    let (_, when) = extract_schedule(&format!("every 2 hours {base}"), now()).unwrap();
    assert_eq!(when.unwrap().due, now() + Duration::hours(2));
    assert!(runs_on_confirm(&format!("every 2 hours {base}")));
    assert!(!runs_on_confirm(&format!("{base} every morning at 8")));
}

#[test]
fn unclear_dates_never_become_destination_names() {
    for suffix in [
        "on Friday",
        "next Monday",
        "tonight",
        "this morning",
        "this evening",
        "this afternoon",
        "on November 20",
        "on 2026-11-20",
        "each morning",
        "every morning",
        "every month",
        "monthly",
        "every -5 minutes",
        "every 0 minutes",
        "daily at 9",
        "on firday",
        "next fortnight",
        "last week",
        "in 0 minutes",
    ] {
        let command = format!("move dinner.jpg from Desktop to Pictures {suffix}");
        assert!(extract_schedule(&command, now()).is_err(), "{command}");
    }
    for suffix in ["yesterday at 3pm", "on 2025-01-01 at 10am", "today at 6am"] {
        assert_eq!(
            extract_schedule(&format!("move a.pdf from Downloads to Documents {suffix}"), now()).unwrap_err(),
            "That time has already passed."
        );
    }
    assert!(extract_schedule("move a.pdf to Documents every day and move b.pdf to Documents every week", now()).is_err());
    for name in [
        "Pictures morning",
        "Documents month",
        "Documents on firday",
        "Pictures weekly on Friday",
        "Documents in 5 minutes",
        "Documents on 2025-01-01",
    ] {
        let command = format!("move a.pdf to \"{name}\"");
        assert_eq!(extract_schedule(&command, now()).unwrap(), (command, None));
    }
}
