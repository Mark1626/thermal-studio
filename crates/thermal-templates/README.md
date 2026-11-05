# thermal-templates

Fill-in-the-blanks templates for thermal prints. A template is a small form plus Markdown;
filling it in gives a note for [`thermal-markdown`](../thermal-markdown) to render.

Templates are data, not code. They can't read files or reach the network, and a template
that loops forever is stopped, so it's safe to load one from a link.

```rust
use thermal_templates::{Date, Value, builtin};

let today = Date::new(2026, 11, 5).unwrap(); // the local date, from the app
let wifi = builtin().into_iter().find(|t| t.id == "wifi-card").unwrap();
let mut values = wifi.defaults(today);
values.insert("ssid".into(), Value::Text("Home".into()));
values.insert("password".into(), Value::Text("hunter2".into()));
assert!(wifi.missing(&values, today).is_empty());

let markdown = wifi.fill(&values, today).unwrap();
assert!(markdown.contains("WIFI:T:WPA;S:Home;P:hunter2;;"));
```

Built-in templates: [`wifi-card`](templates/wifi-card.md), [`checklist`](templates/checklist.md),
[`jar-label`](templates/jar-label.md) and [`contact-card`](templates/contact-card.md).

## Writing a template

A template is a Markdown file with a YAML block at the top that describes it and its form:

````markdown
---
schema: 1
id: name-badge
name: Name badge
description: A name and role, large enough to read across a room.
fields:
  - key: name
    label: Your name
    required: true
  - key: role
    label: Role
    placeholder: Volunteer
---
# {{ name }}

{{ role }}
````

Try it with the CLI: `thermal template name-badge.md --set name=Ada -o badge.png`.

### Template details

| Key | Meaning |
|---|---|
| `schema` | Template format version. Currently `1`, which is the default. Apps refuse templates whose version is newer than they support. |
| `id` | Required. A unique short name made of lowercase letters, digits and `-`, like `name-badge`. |
| `name` | Required. Shown in the list of templates. |
| `version`, `description`, `author` | Optional. |
| `fields` | The form, as a list of fields. |

Misspelt or unknown keys are reported as errors instead of being ignored.

### Fields

| Key | Meaning |
|---|---|
| `key` | Required. The name the body uses, like `{{ name }}`. |
| `label` | Shown next to the input. Defaults to `key`. |
| `type` | `text` (the default), `multiline`, `choice`, `bool`, `list` or `date`. |
| `default` | The starting value. For a `date`, `today` or a date like `2026-11-05`. |
| `required` | `true` if the note shouldn't print until this field is filled in. |
| `placeholder` | Example text shown in an empty input. |
| `help` | A longer explanation shown under the input. |
| `options` | For `choice` only: a list of values, or of `{ value: …, label: … }` pairs. |

In the body, `text`, `multiline` and `choice` fields are strings, `bool` fields are
`true` or `false`, `list` fields are lists of strings with blank entries removed, and `date`
fields are dates like `2026-11-05` (or empty). `{{ today }}` is the date the note is filled
in, so a field can't be called `today`.

### The body

The body is Markdown, with everything `thermal-markdown` supports, including QR codes in
fenced code blocks marked `qr`. It is filled in with
[MiniJinja](https://docs.rs/minijinja) syntax: `{{ field }}`, `{% if %}`, `{% for %}` and
filters such as `upper`, `join` and `default`. A line holding only a `{% … %}` tag leaves
no blank line behind. A tag at the end of a line takes that line break with it, so end such a
tag with `+%}` to keep the break: `{% if goal %} · Goal: {{ goal }}{% endif +%}`.

Values print exactly as typed, wherever they appear:

- In text, Markdown characters such as `*`, `#` and `|` are escaped, and line breaks are
  kept. Headings and table cells hold one line, so line breaks there become spaces.
- In code blocks, `` `code` `` and `qr` blocks, values are inserted unchanged, so a QR
  code encodes exactly what was typed.
- To have a value read as Markdown instead (for example a notes field where people
  write lists), use `{{ notes | safe }}`.

Extra filters:

| Filter | Use |
|---|---|
| `date` | Prints a date: `{{ made \| date }}` gives `5 Nov 2026`. Takes a format too: `date("%A %-d %B")` gives `Thursday 5 November`. Codes: `%Y` 2026, `%y` 26, `%m` 11, `%-m` 11, `%d` 05, `%-d` 5, `%B` November, `%b` Nov, `%A` Thursday, `%a` Thu, `%u` 4 (1 is Monday, 7 is Sunday), `%%` %. |
| `add_days` | The date some days later: `{{ made \| add_days(30) \| date }}`. |
| `wifi_escape` | Escapes `\ ; , : "` for the `S:` and `P:` parts of a `WIFI:` QR code. |
| `vcard_escape` | Escapes text for a `BEGIN:VCARD` contact QR code. |

Dates print in English. Empty dates stay empty through `date` and `add_days`.

### Lists of templates

The apps offer the templates listed in [`templates/index.json`](../../templates/index.json). An `index.json` names
each template's file relative to itself, so a copy of that folder hosted anywhere works the same way. Write it with
`thermal index-templates <folder>` rather than by hand; apps skip entries they can't use, such as templates in a
newer format or ids that a built-in template already has.

### Limits

A template file can be at most 256 KB. Filling one in may run a limited number of
template instructions and produce a limited amount of text. The limits are far above what
a real note needs and are there to stop templates that run away.
