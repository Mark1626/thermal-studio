#![doc = include_str!("../README.md")]

mod date;
mod escape;
mod front_matter;
mod index;

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use minijinja::{Environment, ErrorKind, UndefinedBehavior};

pub use date::Date;
pub use index::{INDEX_SCHEMA, Index, IndexEntry, MAX_INDEX_BYTES, OFFICIAL_INDEX};

/// Newest template format this version reads. Templates declare theirs with `schema:`;
/// a newer one is refused with [`Error::NewerSchema`] rather than half-understood.
///
/// Once a version has been released, raise this whenever templates gain something older
/// versions can't do (a field type, filter or date code). Templates that use it then declare
/// the new number, so older apps say they need an update, and leave them out of the list of
/// templates, instead of failing part-way through.
pub const SCHEMA: u32 = 1;

/// Largest template file accepted.
pub const MAX_TEMPLATE_BYTES: usize = 256 * 1024;

/// Largest Markdown a template may produce (well over a metre of paper).
const MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// Template instructions allowed per fill: far more than any real template needs, but
/// it stops a runaway loop.
const FUEL: u64 = 100_000;

const BUILTIN: [&str; 4] = [
    include_str!("../templates/wifi-card.md"),
    include_str!("../templates/checklist.md"),
    include_str!("../templates/jar-label.md"),
    include_str!("../templates/contact-card.md"),
];

/// The templates that ship with the app.
pub fn builtin() -> Vec<Template> {
    BUILTIN
        .iter()
        .map(|src| Template::parse(src).expect("built-in templates are valid"))
        .collect()
}

/// Check a link typed to add a template, and turn a GitHub file page into the file
/// itself. Only `https://` is accepted, so a template can't be changed on its way; plain
/// `http://` is allowed for this computer, to test a template while writing it.
pub fn template_link(input: &str) -> Result<String, String> {
    let url = input.trim();
    let url = github_raw(url).unwrap_or_else(|| url.to_string());
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(url);
    }
    if let Some(rest) = lower.strip_prefix("http://") {
        let host = match rest.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or_default(),
            None => rest.split(['/', ':', '?', '#']).next().unwrap_or_default(),
        };
        return if matches!(host, "localhost" | "127.0.0.1" | "::1") {
            Ok(url)
        } else {
            Err("Use an https:// link, so the template can't be changed on its way.".into())
        };
    }
    Err("Enter a link starting with https://".into())
}

/// `https://github.com/u/r/blob/main/x.md` → `https://raw.githubusercontent.com/u/r/main/x.md`.
fn github_raw(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.splitn(4, '/');
    let (user, repo, blob, path) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    (blob == "blob").then(|| {
        let path = path.split(['?', '#']).next().unwrap_or_default();
        format!("https://raw.githubusercontent.com/{user}/{repo}/{path}")
    })
}

/// A parsed template: details to show in a list, a form to fill in, and a body that
/// turns the form's values into Markdown.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// The template format it's written in (its `schema:`), at most [`SCHEMA`].
    pub schema: u32,
    /// Short and unique, like `wifi-card`.
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub fields: Vec<Field>,
    body: String,
    /// Line of the file the body starts on, for error messages.
    body_line: usize,
}

/// One input on the template's form.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The name the body uses: `{{ key }}`.
    pub key: String,
    /// Shown next to the input (the key if the template gives none).
    pub label: String,
    /// A longer explanation, shown under the input. May be empty.
    pub help: String,
    /// Example text shown in an empty input. May be empty.
    pub placeholder: String,
    /// The note shouldn't be printed until this is filled in (see [`Template::missing`]).
    pub required: bool,
    pub kind: FieldKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    /// One line of text.
    Text { default: String },
    /// Several lines of text.
    Multiline { default: String },
    /// One of a fixed set of values.
    Choice {
        options: Vec<Choice>,
        default: String,
    },
    /// On or off.
    Bool { default: bool },
    /// Any number of lines, one per entry (checklist items, say).
    List { default: Vec<String> },
    /// A day of the calendar. Its value is written `2026-11-05`, or empty for none.
    Date { default: DateDefault },
}

/// What a date field starts as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateDefault {
    /// No date.
    Blank,
    /// The day the form is filled in.
    Today,
    On(Date),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// What the body sees.
    pub value: String,
    /// What the form shows.
    pub label: String,
}

/// A value for one field: text for text, multiline and choice fields, a flag for bool
/// fields, entries for list fields, and `2026-11-05` (or empty) for date fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Bool(bool),
    List(Vec<String>),
}

/// Values by field key. Fields left out use their defaults.
pub type Values = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the template is too large (the limit is {} KB)", MAX_TEMPLATE_BYTES / 1024)]
    TooLarge,
    #[error("a template must start with a `---` block describing it")]
    NoFrontMatter,
    #[error(
        "this template needs a newer version of Thermal Studio (it uses format {0}; this version reads up to {SCHEMA})"
    )]
    NewerSchema(u64),
    #[error("in the `---` block: {0}")]
    FrontMatter(String),
    #[error("in the template body: {0}")]
    Body(String),
    /// A value doesn't suit its field (or there's no such field).
    #[error("{0}")]
    Value(String),
    /// A list of templates (`index.json`) that can't be used.
    #[error("{0}")]
    Index(String),
}

impl Template {
    /// Read a template file: YAML front matter between `---` lines, then the body.
    ///
    /// The body is also filled in once with the defaults, so mistakes such as a
    /// misspelt field name are reported here rather than when someone uses it.
    pub fn parse(source: &str) -> Result<Template, Error> {
        if source.len() > MAX_TEMPLATE_BYTES {
            return Err(Error::TooLarge);
        }
        let (yaml, body, body_line) = front_matter::split(source)?;
        let h = front_matter::read(yaml)?;
        let t = Template {
            schema: h.schema,
            id: h.id,
            name: h.name.trim().to_string(),
            version: h.version,
            description: h.description.trim().to_string(),
            author: h.author,
            fields: h.fields,
            // Token markers can't come from the template itself.
            body: body.replace([escape::OPEN, escape::CLOSE], ""),
            body_line,
        };
        t.fill(&Values::new(), Date::MIN)?;
        Ok(t)
    }

    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Every field's default value, with `today` for dates that start as today.
    pub fn defaults(&self, today: Date) -> Values {
        self.fields
            .iter()
            .map(|f| (f.key.clone(), f.default_value(today)))
            .collect()
    }

    /// Required fields that are still empty (taking defaults into account).
    pub fn missing(&self, values: &Values, today: Date) -> Vec<&Field> {
        self.fields
            .iter()
            .filter(|f| f.required)
            .filter(|f| {
                match values
                    .get(&f.key)
                    .cloned()
                    .unwrap_or_else(|| f.default_value(today))
                {
                    Value::Text(s) => s.trim().is_empty(),
                    Value::List(items) => items.iter().all(|s| s.trim().is_empty()),
                    Value::Bool(_) => false,
                }
            })
            .collect()
    }

    /// Fill the template in, giving Markdown ready for `thermal-markdown`.
    ///
    /// Values print exactly as typed: Markdown characters in them are escaped (see
    /// the crate docs). Required fields aren't enforced here, so a preview can be shown
    /// while the form is still being filled in; check [`Template::missing`] before
    /// printing.
    ///
    /// `today` is the local date: the default for dates that start as today, and the
    /// body's `{{ today }}`.
    pub fn fill(&self, values: &Values, today: Date) -> Result<String, Error> {
        if let Some(k) = values.keys().find(|k| self.field(k).is_none()) {
            return Err(Error::Value(format!(
                "`{}` has no field called `{k}`",
                self.name
            )));
        }
        let mut ctx = BTreeMap::new();
        ctx.insert("today", minijinja::Value::from(today.to_string()));
        for f in &self.fields {
            let v = values
                .get(&f.key)
                .cloned()
                .unwrap_or_else(|| f.default_value(today));
            ctx.insert(f.key.as_str(), f.to_jinja(v)?);
        }

        // Every `{{ … }}` becomes a token; `escape` swaps them for escaped values.
        let tokens = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut env = Environment::new();
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_fuel(Some(FUEL));
        env.add_filter("wifi_escape", wifi_escape);
        env.add_filter("vcard_escape", vcard_escape);
        env.add_filter("date", date_filter);
        env.add_filter("add_days", add_days);
        let sink = tokens.clone();
        env.set_formatter(move |out, _state, value| {
            if value.is_undefined() || value.is_none() {
                return Ok(());
            }
            let text = value.to_string();
            if value.is_safe() {
                // `| safe`: the value is Markdown.
                out.write_str(&text.replace([escape::OPEN, escape::CLOSE], ""))?;
            } else {
                let mut tokens = sink.lock().unwrap();
                out.write_str(&escape::token(tokens.len()))?;
                tokens.push(text);
            }
            Ok(())
        });

        let tmpl = env
            .template_from_str(&self.body)
            .map_err(|e| self.body_error(&e))?;
        let mut out = Limited(Vec::new());
        tmpl.render_captured_to(&ctx, &mut out).map_err(|e| {
            if e.kind() == minijinja::ErrorKind::WriteFailure {
                Error::Body("it produces far too much text to print".into())
            } else {
                self.body_error(&e)
            }
        })?;
        let markdown = String::from_utf8(out.0).expect("templates render UTF-8");
        let tokens = tokens.lock().unwrap();
        Ok(escape::substitute(&markdown, &tokens))
    }

    fn body_error(&self, e: &minijinja::Error) -> Error {
        let what = match e.detail() {
            Some(d) => format!("{}: {d}", e.kind()),
            None => e.kind().to_string(),
        };
        Error::Body(match e.line() {
            Some(l) => format!("line {}: {what}", self.body_line + l - 1),
            None => what,
        })
    }
}

impl Field {
    pub fn default_value(&self, today: Date) -> Value {
        match &self.kind {
            FieldKind::Text { default }
            | FieldKind::Multiline { default }
            | FieldKind::Choice { default, .. } => Value::Text(default.clone()),
            FieldKind::Bool { default } => Value::Bool(*default),
            FieldKind::List { default } => Value::List(default.clone()),
            FieldKind::Date { default } => Value::Text(match default {
                DateDefault::Blank => String::new(),
                DateDefault::Today => today.to_string(),
                DateDefault::On(d) => d.to_string(),
            }),
        }
    }

    /// `text`, `multiline`, `choice`, `bool`, `list` or `date`, as written in templates.
    pub fn type_name(&self) -> &'static str {
        match self.kind {
            FieldKind::Text { .. } => "text",
            FieldKind::Multiline { .. } => "multiline",
            FieldKind::Choice { .. } => "choice",
            FieldKind::Bool { .. } => "bool",
            FieldKind::List { .. } => "list",
            FieldKind::Date { .. } => "date",
        }
    }

    fn to_jinja(&self, v: Value) -> Result<minijinja::Value, Error> {
        let wrong = || {
            Error::Value(format!(
                "`{}` is a {} field; it can't take {v:?}",
                self.key,
                self.type_name()
            ))
        };
        Ok(match (&self.kind, &v) {
            (FieldKind::Text { .. } | FieldKind::Multiline { .. }, Value::Text(s)) => {
                minijinja::Value::from(s.as_str())
            }
            (FieldKind::Choice { options, .. }, Value::Text(s)) => {
                if !options.iter().any(|o| o.value == *s) {
                    let all: Vec<_> = options.iter().map(|o| o.value.as_str()).collect();
                    return Err(Error::Value(format!(
                        "`{}` must be one of: {}",
                        self.key,
                        all.join(", ")
                    )));
                }
                minijinja::Value::from(s.as_str())
            }
            (FieldKind::Bool { .. }, Value::Bool(b)) => minijinja::Value::from(*b),
            (FieldKind::Date { .. }, Value::Text(s)) if s.trim().is_empty() => {
                minijinja::Value::from("")
            }
            (FieldKind::Date { .. }, Value::Text(s)) => match Date::parse(s) {
                Some(d) => minijinja::Value::from(d.to_string()),
                None => {
                    return Err(Error::Value(format!(
                        "`{}` must be a date like 2026-11-05, not `{s}`",
                        self.key
                    )));
                }
            },
            // Blank entries are the empty rows of a form; leave them out.
            (FieldKind::List { .. }, Value::List(items)) => items
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(minijinja::Value::from)
                .collect(),
            _ => return Err(wrong()),
        })
    }
}

/// Escape text for the `S:` and `P:` parts of a `WIFI:` QR code.
fn wifi_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | ';' | ',' | ':' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape text for a vCard (contact) QR code.
fn vcard_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.replace("\r\n", "\n").chars() {
        match c {
            '\\' | ',' | ';' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

fn parse_date(value: &str) -> Result<Option<Date>, minijinja::Error> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    Date::parse(value).map(Some).ok_or_else(|| {
        minijinja::Error::new(
            ErrorKind::InvalidOperation,
            format!("`{value}` is not a date like 2026-11-05"),
        )
    })
}

/// `{{ made | date }}` → `5 Nov 2026`; `{{ made | date("%A") }}` → `Thursday`. An
/// empty date stays empty.
fn date_filter(value: &str, format: Option<&str>) -> Result<String, minijinja::Error> {
    let Some(day) = parse_date(value)? else {
        return Ok(String::new());
    };
    day.format(format.unwrap_or(date::DEFAULT_FORMAT))
        .map_err(|why| minijinja::Error::new(ErrorKind::InvalidOperation, why))
}

/// `{{ made | add_days(30) }}` → the date 30 days later. An empty date stays empty.
fn add_days(value: &str, days: i64) -> Result<String, minijinja::Error> {
    Ok(parse_date(value)?
        .map(|d| d.add_days(days).to_string())
        .unwrap_or_default())
}

/// Collects rendered output, failing once it passes [`MAX_OUTPUT_BYTES`].
struct Limited(Vec<u8>);

impl io::Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.0.len() + buf.len() > MAX_OUTPUT_BYTES {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
