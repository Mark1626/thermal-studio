//! The official list of templates (`templates/` at the top of the repository) must load,
//! and its `index.json` must be up to date.

use std::path::Path;

use thermal_templates::{Date, Index, Template, Value, Values};

#[test]
fn official_index_is_up_to_date() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
    let index = Index::from_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
    let written = std::fs::read_to_string(dir.join("index.json")).expect("templates/index.json");
    assert!(
        written == index.to_json(),
        "templates/index.json is out of date: run `cargo run -p thermal-cli -- index-templates templates`"
    );
    assert_eq!(
        Index::parse(&written).unwrap(),
        index,
        "the apps would leave some templates out"
    );
}

fn official(id: &str) -> Template {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../templates/{id}.md"));
    Template::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn fill(t: &Template, pairs: &[(&str, Value)]) -> String {
    let values: Values = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    t.fill(&values, Date::new(2026, 11, 5).unwrap()).unwrap()
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

#[test]
fn calendar_tracker_lays_out_whole_months() {
    let t = official("calendar-tracker");
    // November 2026 starts on a Sunday, so with Monday first it needs six rows.
    let md = fill(&t, &[("title", text("Walk")), ("goal", text("20 days"))]);
    assert!(
        md.contains(
            "**November 2026** · Goal: 20 days\n\n\
             | &nbsp;Mo&nbsp; | &nbsp;Tu&nbsp; | &nbsp;We&nbsp; | &nbsp;Th&nbsp; | &nbsp;Fr&nbsp; \
             | &nbsp;Sa&nbsp; | &nbsp;Su&nbsp; |\n\
             |:-:|:-:|:-:|:-:|:-:|:-:|:-:|\n\
             |  |  |  |  |  |  | 1 |\n\
             | 2 | 3 | 4 | 5 | 6 | 7 | 8 |\n"
        ),
        "{md}"
    );
    assert!(md.ends_with("| 30 |  |  |  |  |  |  |\n"), "{md}");

    // February 2026 starts on a Sunday and has 28 days: four rows when Sunday is first.
    let feb = fill(
        &t,
        &[
            ("title", text("Piano")),
            ("month", text("2026-02-14")),
            ("week_starts", text("Sunday")),
        ],
    );
    assert!(feb.contains("| &nbsp;Su&nbsp; | &nbsp;Mo&nbsp; |"), "{feb}");
    assert!(
        feb.ends_with("| 22 | 23 | 24 | 25 | 26 | 27 | 28 |\n"),
        "{feb}"
    );
    assert_eq!(
        feb.matches("\n| ").count(),
        5,
        "header and four weeks:\n{feb}"
    );
}

#[test]
fn action_item_tracker_splits_what_who_when() {
    let t = official("action-item-tracker");
    let items = Value::List(vec![
        "Send the slides; Ada; Friday".into(),
        "Book a room;Sam".into(),
        "Check the *budget*;; 12/11".into(),
        "Call back".into(),
    ]);
    let md = fill(&t, &[("meeting", text("Team sync")), ("items", items)]);
    assert_eq!(
        md,
        "# Team sync\n\n**Thursday 5 November 2026**\n\n\
         - [ ] **Send the slides** · Ada · by Friday\n\
         - [ ] **Book a room** · Sam\n\
         - [ ] **Check the \\*budget\\*** · by 12\\/11\n\
         - [ ] **Call back**\n"
    );
}
