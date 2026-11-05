use super::*;

use thermal_markdown::{NoImages, Renderer, Style};

fn builtin_template(id: &str) -> Template {
    builtin().into_iter().find(|t| t.id == id).unwrap()
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn values<const N: usize>(pairs: [(&str, Value); N]) -> Values {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// The text of the only QR code in a rendered note.
fn scan(markdown: &str) -> String {
    let bmp = Renderer::new().render(markdown, &Style::default(), &NoImages);
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(
        bmp.width() as usize,
        bmp.height() as usize,
        |x, y| if bmp.get(x as u32, y as u32) { 0 } else { 255 },
    );
    let grids = img.detect_grids();
    assert_eq!(grids.len(), 1, "expected one QR code in:\n{markdown}");
    grids[0].decode().expect("QR code should decode").1
}

fn parse_err(src: &str) -> String {
    Template::parse(src).unwrap_err().to_string()
}

const MINIMAL: &str = "---\nid: t\nname: T\nfields:\n  - key: x\n---\n";

/// A Thursday.
const TODAY: Date = Date::new(2026, 11, 5).unwrap();

#[test]
fn built_in_templates_parse_with_unique_ids() {
    let all = builtin();
    assert_eq!(all.len(), BUILTIN.len());
    for (i, t) in all.iter().enumerate() {
        assert!(all[..i].iter().all(|o| o.id != t.id), "duplicate {}", t.id);
        assert!(!t.description.is_empty(), "{} has no description", t.id);
        t.fill(&t.defaults(TODAY), TODAY).unwrap();
    }
}

#[test]
fn wifi_card_scans_and_prints_tricky_names_as_typed() {
    let t = builtin_template("wifi-card");
    let v = values([
        ("ssid", text("Café; *Guest*")),
        ("password", text(r#"p:a,s"s\w"#)),
    ]);
    let md = t.fill(&v, TODAY).unwrap();
    assert_eq!(
        scan(&md),
        r#"WIFI:T:WPA;S:Café\; *Guest*;P:p\:a\,s\"s\\w;;"#
    );
    assert!(md.contains(r"**Network:** Café\; \*Guest\*"), "{md}");
    assert!(md.contains(r#"**Password:** p\:a\,s\"s\\w"#), "{md}");
}

#[test]
fn wifi_card_options() {
    let t = builtin_template("wifi-card");
    let open = values([
        ("ssid", text("Lobby")),
        ("password", text("unused")),
        ("security", text("nopass")),
        ("hidden", Value::Bool(true)),
    ]);
    let md = t.fill(&open, TODAY).unwrap();
    assert_eq!(scan(&md), "WIFI:T:nopass;S:Lobby;H:true;;");
    assert!(!md.contains("Password"), "{md}");

    let quiet = values([
        ("ssid", text("Home")),
        ("password", text("secret")),
        ("show_password", Value::Bool(false)),
    ]);
    let md = t.fill(&quiet, TODAY).unwrap();
    assert_eq!(scan(&md), "WIFI:T:WPA;S:Home;P:secret;;");
    assert!(!md.contains("Password"), "{md}");
}

#[test]
fn checklist_drops_blank_items_and_keeps_note_lines() {
    let t = builtin_template("checklist");
    let v = values([
        ("title", text("Trip")),
        (
            "items",
            Value::List(vec!["Passport".into(), "  ".into(), "[x] *socks*".into()]),
        ),
        ("notes", text("Leave by 6.\nTaxi #42")),
    ]);
    let md = t.fill(&v, TODAY).unwrap();
    assert_eq!(
        md,
        "# Trip\n\n- [ ] Passport\n- [ ] \\[x\\] \\*socks\\*\n\nLeave by 6\\.\\\nTaxi \\#42\n"
    );
}

#[test]
fn missing_lists_empty_required_fields() {
    let t = builtin_template("wifi-card");
    let names: Vec<_> = t
        .missing(&t.defaults(TODAY), TODAY)
        .iter()
        .map(|f| f.key.as_str())
        .collect();
    assert_eq!(names, ["ssid"]);
    assert!(t.missing(&values([("ssid", text("x"))]), TODAY).is_empty());
    assert_eq!(t.missing(&values([("ssid", text("  "))]), TODAY).len(), 1);
}

#[test]
fn values_must_suit_their_fields() {
    let t = builtin_template("wifi-card");
    let err = |v| t.fill(&v, TODAY).unwrap_err().to_string();
    assert!(err(values([("nope", text("x"))])).contains("no field called `nope`"));
    assert!(err(values([("hidden", text("yes"))])).contains("bool field"));
    assert!(err(values([("security", text("WPA3"))])).contains("one of: WPA, WEP, nopass"));
}

#[test]
fn safe_values_are_markdown() {
    let t = Template::parse(&format!("{MINIMAL}{{{{ x | safe }}}} and {{{{ x }}}}")).unwrap();
    let md = t.fill(&values([("x", text("**hi**"))]), TODAY).unwrap();
    assert_eq!(md, r"**hi** and \*\*hi\*\*");
}

#[test]
fn field_types_and_defaults() {
    let src = r#"---
schema: 1
id: kinds
name: Kinds
version: 1.0
fields:
  - key: a
  - key: b
    type: multiline
    default: two
  - key: c
    type: choice
    label: Pick
    options: [x, { value: y, label: Why }]
    default: y
  - key: d
    type: bool
    default: true
  - key: e
    type: list
    default: [p, q]
---
{{ a }}|{{ b }}|{{ c }}|{{ d }}|{{ e | join(",") }}
"#;
    let t = Template::parse(src).unwrap();
    assert_eq!(t.version, "1.0");
    assert_eq!(t.fields[0].label, "a");
    assert_eq!(t.fields[0].type_name(), "text");
    let FieldKind::Choice { options, default } = &t.fields[2].kind else {
        panic!("{:?}", t.fields[2]);
    };
    assert_eq!(options[0].label, "x");
    assert_eq!(options[1].label, "Why");
    assert_eq!(default, "y");
    assert_eq!(t.fill(&Values::new(), TODAY).unwrap(), r"|two|y|True|p\,q");
}

#[test]
fn front_matter_mistakes_are_explained() {
    assert!(parse_err("# Just Markdown").contains("must start with a `---` block"));
    assert!(parse_err("---\nid: t\nname: T\n").contains("never closed"));
    assert!(parse_err("---\nname: T\n---\n").contains("missing `id`"));
    assert!(parse_err("---\nid: My Card\nname: T\n---\n").contains("lowercase letters"));
    assert!(parse_err("---\nid: t\n---\n").contains("missing `name`"));
    assert!(parse_err("---\nid: t\nname: T\ncolour: red\n---\n").contains("unknown key `colour`"));
    let field = |f: &str| parse_err(&format!("---\nid: t\nname: T\nfields:\n{f}---\n"));
    assert!(field("  - label: X\n").contains("missing `key`"));
    assert!(field("  - key: 2x\n").contains("can't start with a digit"));
    assert!(
        field("  - key: x\n    requried: true\n").contains("field `x`: unknown key `requried`")
    );
    assert!(field("  - key: x\n    type: colour\n").contains("unknown type `colour`"));
    assert!(field("  - key: x\n  - key: x\n").contains("two fields are called `x`"));
    assert!(field("  - key: x\n    type: choice\n").contains("at least one choice"));
    assert!(field("  - key: x\n    type: choice\n    options: [a, a]\n").contains("listed twice"));
    assert!(
        field("  - key: x\n    type: choice\n    options: [a]\n    default: b\n")
            .contains("`b` is not one of the options")
    );
    assert!(field("  - key: x\n    options: [a]\n").contains("only applies to `type: choice`"));
    assert!(
        field("  - key: x\n    type: bool\n    default: maybe\n").contains("`true` or `false`")
    );
    assert!(parse_err("---\nid: t\nname: [unclosed\n---\n").contains("in the `---` block: line"));
}

#[test]
fn newer_formats_are_refused_before_anything_else() {
    let e = Template::parse("---\nschema: 2\nid: t\nname: T\nshiny_new_key: 1\n---\n").unwrap_err();
    assert_eq!(e, Error::NewerSchema(2));
    assert!(e.to_string().contains("newer version of Thermal Studio"));
}

#[test]
fn yaml_aliases_are_refused() {
    let bomb = "---\nid: t\nname: T\na: &a [x, x, x, x, x, x, x, x]\nb: &b [*a, *a, *a, *a, *a, *a, *a, *a]\nc: [*b, *b, *b, *b, *b, *b, *b, *b]\n---\n";
    assert!(parse_err(bomb).contains("aliases"));
}

#[test]
fn body_errors_point_at_the_file_line() {
    // The body starts on line 7 of the file.
    let src = format!("{MINIMAL}ok\n{{{{ x }}}} {{% if %}}\n");
    assert!(
        parse_err(&src).contains("line 8: syntax error"),
        "{}",
        parse_err(&src)
    );
    // Misspelt names are caught when the template is loaded.
    let src = format!("{MINIMAL}\n\n{{{{ y }}}}\n");
    assert!(
        parse_err(&src).contains("line 9: undefined value"),
        "{}",
        parse_err(&src)
    );
}

#[test]
fn runaway_templates_are_stopped() {
    let forever = format!(
        "{MINIMAL}{{% for i in range(10000) %}}{{% for j in range(10000) %}}{{% endfor %}}{{% endfor %}}"
    );
    assert!(parse_err(&forever).contains("in the template body"));

    let flood = format!(
        "{MINIMAL}{{% for i in range(10000) %}}{}{{% endfor %}}",
        "x".repeat(100)
    );
    assert!(parse_err(&flood).contains("far too much text"));

    let huge = format!("{MINIMAL}{}", "x".repeat(MAX_TEMPLATE_BYTES));
    assert_eq!(Template::parse(&huge), Err(Error::TooLarge));
}

#[test]
fn templates_cannot_include_other_files() {
    let src = format!("{MINIMAL}{{% include \"/etc/passwd\" %}}");
    assert!(Template::parse(&src).is_err());
}

#[test]
fn windows_line_endings_and_bom() {
    let src = "\u{feff}---\r\nid: t\r\nname: T\r\n---\r\n# Hi\r\n";
    let t = Template::parse(src).unwrap();
    assert_eq!(t.fill(&Values::new(), TODAY).unwrap().trim_end(), "# Hi");
}

#[test]
fn jar_label_dates() {
    let t = builtin_template("jar-label");
    let md = t
        .fill(&values([("contents", text("Lime pickle"))]), TODAY)
        .unwrap();
    assert_eq!(
        md,
        "# Lime pickle\n\nMade **5 Nov 2026**\n\nUse by **5 Dec 2026**\n"
    );

    let v = values([
        ("contents", text("Jam")),
        ("made", text("2026-12-25")),
        ("use_within", text("7")),
        ("note", text("Keep cool")),
    ]);
    let md = t.fill(&v, TODAY).unwrap();
    assert!(
        md.contains("Made **25 Dec 2026**\n\nUse by **1 Jan 2027**"),
        "{md}"
    );
    assert!(md.ends_with("*Keep cool*\n"), "{md}");

    let no_use_by = values([("contents", text("Jam")), ("use_within", text("none"))]);
    assert!(!t.fill(&no_use_by, TODAY).unwrap().contains("Use by"));
    let undated = values([("contents", text("Jam")), ("made", text(""))]);
    assert_eq!(t.fill(&undated, TODAY).unwrap(), "# Jam\n\n");

    let e = t
        .fill(&values([("made", text("5/11/2026"))]), TODAY)
        .unwrap_err();
    assert!(e.to_string().contains("a date like 2026-11-05"), "{e}");
}

#[test]
fn contact_card_scans_as_a_vcard() {
    let t = builtin_template("contact-card");
    let v = values([
        ("name", text("Ada Lovelace")),
        ("job", text("Analyst")),
        ("org", text("Babbage, Lovelace & Co; Ltd")),
        ("phone", text("+44 20 7946 0000")),
        ("email", text("ada@example.com")),
    ]);
    let md = t.fill(&v, TODAY).unwrap();
    assert_eq!(
        scan(&md),
        "BEGIN:VCARD\nVERSION:3.0\nN:;Ada Lovelace;;;\nFN:Ada Lovelace\n\
         ORG:Babbage\\, Lovelace & Co\\; Ltd\nTITLE:Analyst\nTEL;TYPE=CELL:+44 20 7946 0000\n\
         EMAIL:ada@example.com\nEND:VCARD"
    );
    assert!(
        md.contains("Analyst, Babbage\\, Lovelace \\& Co\\; Ltd"),
        "{md}"
    );
    assert!(md.contains("**Phone:** \\+44 20 7946 0000"), "{md}");
    assert!(!md.contains("Web"), "{md}");

    let quiet = values([("name", text("Ada")), ("show_details", Value::Bool(false))]);
    let md = t.fill(&quiet, TODAY).unwrap();
    assert_eq!(
        scan(&md),
        "BEGIN:VCARD\nVERSION:3.0\nN:;Ada;;;\nFN:Ada\nEND:VCARD"
    );
    assert!(!md.contains("Phone"), "{md}");
}

#[test]
fn date_fields_and_today() {
    let src = "---\nid: d\nname: D\nfields:\n  - key: a\n    type: date\n  - key: b\n    type: date\n    default: today\n  - key: c\n    type: date\n    default: 2026-01-31\n---\n\
         [{{ a }}] {{ b }} {{ c | add_days(1) | date(\"%A %-d %B\") }} {{ today | date(\"%a\") }}";
    let t = Template::parse(src).unwrap();
    assert_eq!(t.fields[0].type_name(), "date");
    assert_eq!(t.defaults(TODAY)["b"], text("2026-11-05"));
    assert_eq!(
        t.fill(&Values::new(), TODAY).unwrap(),
        r"[] 2026\-11\-05 Sunday 1 February Thu"
    );
    let field = |f: &str| parse_err(&format!("---\nid: t\nname: T\nfields:\n{f}---\n"));
    assert!(
        field("  - key: d\n    type: date\n    default: tomorrow\n").contains("`today` or a date")
    );
    assert!(field("  - key: today\n").contains("`today` is taken"));
    let bad = format!("{MINIMAL}{{{{ \"soon\" | date }}}}");
    assert!(
        parse_err(&bad).contains("`soon` is not a date"),
        "{}",
        parse_err(&bad)
    );
    // A code from a newer version is named, instead of failing somewhere further on.
    let newer = format!("{MINIMAL}{{{{ today | date(\"%q\") | int }}}}");
    assert!(
        parse_err(&newer).contains("line 7: invalid operation: `%q` isn't a date code"),
        "{}",
        parse_err(&newer)
    );
}

#[test]
fn links() {
    assert_eq!(
        template_link(" https://example.com/t.md ").unwrap(),
        "https://example.com/t.md"
    );
    assert_eq!(
        template_link("https://github.com/u/r/blob/main/dir/t.md?plain=1").unwrap(),
        "https://raw.githubusercontent.com/u/r/main/dir/t.md"
    );
    assert!(template_link("http://localhost:8000/t.md").is_ok());
    assert!(template_link("http://127.0.0.1/t.md").is_ok());
    assert!(template_link("http://[::1]:8000/t.md").is_ok());
    assert!(template_link("http://localhost.evil.com/t.md").is_err());
    assert!(template_link("http://example.com/t.md").is_err());
    assert!(template_link("file:///etc/passwd").is_err());
    assert!(template_link("example.com/t.md").is_err());
}
