//! Reading the YAML front matter: template details and the form's fields.

use yaml_rust2::parser::{EventReceiver, Parser};
use yaml_rust2::{Event, ScanError, Yaml, YamlLoader};

use crate::{Choice, Date, DateDefault, Error, Field, FieldKind, SCHEMA};

/// Everything above the body.
pub(crate) struct Header {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub fields: Vec<Field>,
}

/// Split a template into its front matter and body. Also returns the line number the
/// body starts on, so errors can point at the right line of the file.
pub(crate) fn split(source: &str) -> Result<(&str, &str, usize), Error> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let yaml = source
        .strip_prefix("---")
        .and_then(|r| r.strip_prefix('\n').or_else(|| r.strip_prefix("\r\n")))
        .ok_or(Error::NoFrontMatter)?;
    let mut offset = 0;
    for (i, line) in yaml.split_inclusive('\n').enumerate() {
        if matches!(line.trim_end(), "---" | "...") {
            // Line 1 is the opening `---`, then the YAML, then this closing line.
            return Ok((&yaml[..offset], &yaml[offset + line.len()..], i + 3));
        }
        offset += line.len();
    }
    Err(fm("the `---` block at the top is never closed"))
}

pub(crate) fn read(yaml: &str) -> Result<Header, Error> {
    // Aliases are expanded by copying, so a few lines of them can grow exponentially.
    // Templates have no use for them; refuse them before loading anything.
    struct Aliases(bool);
    impl EventReceiver for Aliases {
        fn on_event(&mut self, ev: Event) {
            self.0 |= matches!(ev, Event::Alias(_));
        }
    }
    let mut aliases = Aliases(false);
    Parser::new_from_str(yaml)
        .load(&mut aliases, true)
        .map_err(yaml_error)?;
    if aliases.0 {
        return Err(fm("YAML aliases (`*name`) aren't supported"));
    }
    let docs = YamlLoader::load_from_str(yaml).map_err(yaml_error)?;
    let Some(Yaml::Hash(top)) = docs.first() else {
        return Err(fm("expected `key: value` lines"));
    };
    let get = |name: &str| top.get(&Yaml::String(name.into()));

    // Check the schema first: a newer template may use keys this version doesn't know.
    let schema = match get("schema").map(Yaml::as_i64) {
        None => 1,
        Some(Some(n)) if n > SCHEMA as i64 => return Err(Error::NewerSchema(n as u64)),
        Some(Some(n)) if n >= 1 => n as u32,
        Some(_) => {
            return Err(fm(format!(
                "`schema` must be a whole number from 1 to {SCHEMA}"
            )));
        }
    };

    let mut h = Header {
        schema,
        id: String::new(),
        name: String::new(),
        version: String::new(),
        description: String::new(),
        author: String::new(),
        fields: Vec::new(),
    };
    for (k, v) in top {
        match k.as_str().unwrap_or_default() {
            "schema" => {}
            "id" => h.id = text(v, "`id`")?,
            "name" => h.name = text(v, "`name`")?,
            "version" => h.version = text(v, "`version`")?,
            "description" => h.description = text(v, "`description`")?,
            "author" => h.author = text(v, "`author`")?,
            "fields" => h.fields = fields(v)?,
            other => return Err(fm(format!("unknown key `{other}`"))),
        }
    }
    if h.id.is_empty() {
        return Err(fm("missing `id` (a short name like `wifi-card`)"));
    }
    if !valid_id(&h.id) {
        return Err(fm(format!(
            "`id` may only use lowercase letters, digits and `-`, not `{}`",
            h.id
        )));
    }
    if h.name.trim().is_empty() {
        return Err(fm("missing `name`"));
    }
    Ok(h)
}

/// Lowercase letters, digits and `-`.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn fields(v: &Yaml) -> Result<Vec<Field>, Error> {
    let items = match v {
        Yaml::Array(a) => a,
        Yaml::Null => return Ok(Vec::new()),
        _ => return Err(fm("`fields` must be a list")),
    };
    let mut out: Vec<Field> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let f = field(i, item)?;
        if out.iter().any(|o| o.key == f.key) {
            return Err(fm(format!("two fields are called `{}`", f.key)));
        }
        out.push(f);
    }
    Ok(out)
}

const FIELD_KEYS: [&str; 8] = [
    "key",
    "label",
    "type",
    "required",
    "default",
    "placeholder",
    "help",
    "options",
];

fn field(i: usize, v: &Yaml) -> Result<Field, Error> {
    let Yaml::Hash(h) = v else {
        return Err(fm(format!("fields[{i}]: expected `key: value` lines")));
    };
    let get = |name: &str| h.get(&Yaml::String(name.into()));
    let key = get("key")
        .map(|v| text(v, &format!("fields[{i}].key")))
        .transpose()?
        .unwrap_or_default();
    let what = if key.is_empty() {
        format!("fields[{i}]")
    } else {
        format!("field `{key}`")
    };
    for k in h.keys() {
        let k = k.as_str().unwrap_or_default();
        if !FIELD_KEYS.contains(&k) {
            return Err(fm(format!("{what}: unknown key `{k}`")));
        }
    }
    if key.is_empty() {
        return Err(fm(format!("{what}: missing `key`")));
    }
    let mut chars = key.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(fm(format!(
            "{what}: a key may only use letters, digits and `_`, and can't start with a digit"
        )));
    }
    if key == "today" {
        return Err(fm(format!(
            "{what}: `today` is taken (it's today's date in the body)"
        )));
    }

    let opt_text = |name: &str| -> Result<String, Error> {
        get(name)
            .map(|v| text(v, &format!("{what}: `{name}`")))
            .transpose()
            .map(Option::unwrap_or_default)
    };
    let default = get("default");
    let ty = get("type")
        .map(|v| text(v, &format!("{what}: `type`")))
        .transpose()?
        .unwrap_or_else(|| "text".into());
    if ty != "choice" && get("options").is_some() {
        return Err(fm(format!(
            "{what}: `options` only applies to `type: choice`"
        )));
    }
    let kind = match ty.as_str() {
        "text" => FieldKind::Text {
            default: opt_text("default")?,
        },
        "multiline" => FieldKind::Multiline {
            default: opt_text("default")?,
        },
        "bool" => FieldKind::Bool {
            default: default
                .map(|v| boolean(v, &format!("{what}: `default`")))
                .transpose()?
                .unwrap_or(false),
        },
        "list" => FieldKind::List {
            default: match default {
                None | Some(Yaml::Null) => Vec::new(),
                Some(Yaml::Array(a)) => a
                    .iter()
                    .map(|v| text(v, &format!("{what}: `default`")))
                    .collect::<Result<_, _>>()?,
                Some(_) => return Err(fm(format!("{what}: `default` must be a list"))),
            },
        },
        "date" => FieldKind::Date {
            default: match default.map(|v| text(v, &format!("{what}: `default`"))) {
                None => DateDefault::Blank,
                Some(t) => match t?.trim() {
                    "" => DateDefault::Blank,
                    "today" => DateDefault::Today,
                    d => DateDefault::On(Date::parse(d).ok_or_else(|| {
                        fm(format!(
                            "{what}: `default` must be `today` or a date like 2026-11-05"
                        ))
                    })?),
                },
            },
        },
        "choice" => {
            let options: Vec<Choice> = match get("options") {
                Some(Yaml::Array(a)) if !a.is_empty() => a
                    .iter()
                    .map(|o| choice(o, &what))
                    .collect::<Result<_, _>>()?,
                _ => {
                    return Err(fm(format!(
                        "{what}: `options` must list at least one choice"
                    )));
                }
            };
            for (j, o) in options.iter().enumerate() {
                if options[..j].iter().any(|p| p.value == o.value) {
                    return Err(fm(format!("{what}: option `{}` is listed twice", o.value)));
                }
            }
            let default = match default {
                None => options[0].value.clone(),
                Some(v) => {
                    let d = text(v, &format!("{what}: `default`"))?;
                    if !options.iter().any(|o| o.value == d) {
                        return Err(fm(format!(
                            "{what}: `default` `{d}` is not one of the options"
                        )));
                    }
                    d
                }
            };
            FieldKind::Choice { options, default }
        }
        other => {
            return Err(fm(format!(
                "{what}: unknown type `{other}` (expected text, multiline, choice, bool, list or date)"
            )));
        }
    };
    Ok(Field {
        label: Some(opt_text("label")?)
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| key.clone()),
        help: opt_text("help")?,
        placeholder: opt_text("placeholder")?,
        required: get("required")
            .map(|v| boolean(v, &format!("{what}: `required`")))
            .transpose()?
            .unwrap_or(false),
        key,
        kind,
    })
}

/// `- WPA` or `- { value: WPA, label: WPA/WPA2/WPA3 }`.
fn choice(v: &Yaml, what: &str) -> Result<Choice, Error> {
    if let Yaml::Hash(h) = v {
        let mut c = Choice {
            value: String::new(),
            label: String::new(),
        };
        for (k, v) in h {
            match k.as_str().unwrap_or_default() {
                "value" => c.value = text(v, &format!("{what}: option `value`"))?,
                "label" => c.label = text(v, &format!("{what}: option `label`"))?,
                other => return Err(fm(format!("{what}: unknown option key `{other}`"))),
            }
        }
        if c.value.is_empty() {
            return Err(fm(format!("{what}: every option needs a `value`")));
        }
        if c.label.trim().is_empty() {
            c.label = c.value.clone();
        }
        return Ok(c);
    }
    let value = text(v, &format!("{what}: options"))?;
    if value.is_empty() {
        return Err(fm(format!("{what}: options can't be empty")));
    }
    Ok(Choice {
        label: value.clone(),
        value,
    })
}

/// Any scalar as text (so `version: 1.0` works); a missing value is empty.
fn text(v: &Yaml, what: &str) -> Result<String, Error> {
    match v {
        Yaml::String(s) | Yaml::Real(s) => Ok(s.clone()),
        Yaml::Integer(i) => Ok(i.to_string()),
        Yaml::Boolean(b) => Ok(b.to_string()),
        Yaml::Null => Ok(String::new()),
        _ => Err(fm(format!("{what}: expected text"))),
    }
}

fn boolean(v: &Yaml, what: &str) -> Result<bool, Error> {
    v.as_bool()
        .ok_or_else(|| fm(format!("{what}: expected `true` or `false`")))
}

fn fm(msg: impl Into<String>) -> Error {
    Error::FrontMatter(msg.into())
}

fn yaml_error(e: ScanError) -> Error {
    // The YAML starts on line 2 of the file, after the opening `---`.
    fm(format!("line {}: {}", e.marker().line() + 1, e.info()))
}
