//! The template catalogue: an `index.json` listing templates that can be added, with each
//! template's file next to it. The official one is the `templates/` folder of this
//! project's repository.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, SCHEMA, Template, builtin, front_matter};

/// Where the apps find the official templates.
pub const OFFICIAL_INDEX: &str =
    "https://raw.githubusercontent.com/Mark1626/thermal-studio/master/templates/index.json";

/// Newest index format this version reads.
pub const INDEX_SCHEMA: u32 = 1;

/// Largest `index.json` accepted.
pub const MAX_INDEX_BYTES: usize = 1024 * 1024;

/// A list of templates that can be added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub templates: Vec<IndexEntry>,
}

/// One template in an [`Index`]: enough to show it in a list without downloading it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// The template file, relative to the index.
    pub file: String,
    /// The template format the file is written in.
    #[serde(default = "first_schema")]
    pub schema: u32,
}

fn first_schema() -> u32 {
    1
}

impl Index {
    /// List the templates in `files` (file name and contents). Fails on any template
    /// that doesn't load, isn't named after its id (`name-badge.md`), or uses the id of
    /// a built-in template.
    pub fn build(files: impl IntoIterator<Item = (String, String)>) -> Result<Index, String> {
        let builtin: Vec<String> = builtin().into_iter().map(|t| t.id).collect();
        let mut templates = Vec::new();
        for (file, source) in files {
            let t = Template::parse(&source).map_err(|e| format!("{file}: {e}"))?;
            if file != format!("{}.md", t.id) {
                return Err(format!(
                    "{file}: name the file after the template's id: {}.md",
                    t.id
                ));
            }
            if builtin.contains(&t.id) {
                return Err(format!(
                    "{file}: `{}` is the id of a built-in template",
                    t.id
                ));
            }
            templates.push(IndexEntry {
                id: t.id,
                name: t.name,
                description: t.description,
                version: t.version,
                author: t.author,
                file,
                schema: t.schema,
            });
        }
        templates.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Index {
            schema: INDEX_SCHEMA,
            templates,
        })
    }

    /// [`Index::build`] from every `.md` file in `dir` except `README.md`.
    pub fn from_dir(dir: &Path) -> Result<Index, String> {
        let mut files = Vec::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".md") || name.eq_ignore_ascii_case("README.md") {
                continue;
            }
            let source =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            files.push((name.to_string(), source));
        }
        Index::build(files)
    }

    /// The index as written to `index.json`.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("an index serialises") + "\n"
    }

    /// Read a downloaded `index.json`. Entries this version can't use are left out:
    /// newer template formats, unsafe file paths, ids of built-in templates and repeats.
    pub fn parse(json: &str) -> Result<Index, Error> {
        if json.len() > MAX_INDEX_BYTES {
            return Err(Error::Index("The list of templates is too large.".into()));
        }
        #[derive(Deserialize)]
        struct Raw {
            schema: u32,
            #[serde(default)]
            templates: Vec<serde_json::Value>,
        }
        let raw: Raw = serde_json::from_str(json)
            .map_err(|e| Error::Index(format!("That isn't a list of templates ({e}).")))?;
        if raw.schema > INDEX_SCHEMA {
            return Err(Error::Index(
                "The list of templates needs a newer version of Thermal Studio.".into(),
            ));
        }
        let builtin: HashSet<String> = builtin().into_iter().map(|t| t.id).collect();
        let mut seen = HashSet::new();
        let templates = raw
            .templates
            .into_iter()
            .filter_map(|v| serde_json::from_value::<IndexEntry>(v).ok())
            .filter(|e| {
                e.schema <= SCHEMA
                    && front_matter::valid_id(&e.id)
                    && safe_file(&e.file)
                    && !builtin.contains(&e.id)
                    && seen.insert(e.id.clone())
            })
            .collect();
        Ok(Index {
            schema: raw.schema,
            templates,
        })
    }

    /// Link to `entry`'s file, next to the index at `index_url`.
    pub fn file_url(index_url: &str, entry: &IndexEntry) -> String {
        let path = &index_url[..index_url.find(['?', '#']).unwrap_or(index_url.len())];
        let dir = path.rsplit_once('/').map_or(path, |(dir, _)| dir);
        format!("{dir}/{}", entry.file)
    }
}

/// A relative path to a `.md` file inside the index's folder.
fn safe_file(file: &str) -> bool {
    file.ends_with(".md")
        && file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        && file.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BADGE: &str = "---\nid: name-badge\nname: Name badge\nversion: 1.0.0\nfields:\n  - key: name\n---\n# {{ name }}\n";

    #[test]
    fn builds_from_template_files() {
        let index = Index::build([("name-badge.md".into(), BADGE.into())]).unwrap();
        assert_eq!(index.templates[0].name, "Name badge");
        assert_eq!(index.templates[0].version, "1.0.0");
        assert_eq!(index.templates[0].schema, 1);
        assert_eq!(Index::parse(&index.to_json()).unwrap(), index);

        let wrong_name = Index::build([("badge.md".into(), BADGE.into())]).unwrap_err();
        assert!(wrong_name.contains("name-badge.md"), "{wrong_name}");
        let broken = Index::build([("x.md".into(), "# nope".into())]).unwrap_err();
        assert!(broken.starts_with("x.md: "), "{broken}");
        let wifi = include_str!("../templates/wifi-card.md");
        let clash = Index::build([("wifi-card.md".into(), wifi.into())]).unwrap_err();
        assert!(clash.contains("built-in"), "{clash}");
    }

    #[test]
    fn leaves_out_entries_it_cannot_use() {
        let json = r#"{
            "schema": 1,
            "future_key": true,
            "templates": [
                {"id": "ok", "name": "OK", "file": "ok.md", "extra": 1},
                {"id": "ok", "name": "Again", "file": "again.md"},
                {"id": "newer", "name": "Newer", "file": "newer.md", "schema": 99},
                {"id": "up", "name": "Up", "file": "../secret.md"},
                {"id": "abs", "name": "Abs", "file": "/etc/passwd.md"},
                {"id": "far", "name": "Far", "file": "https://evil.example/x.md"},
                {"id": "txt", "name": "Txt", "file": "notes.txt"},
                {"id": "Bad Id", "name": "Bad", "file": "bad.md"},
                {"id": "wifi-card", "name": "Mine", "file": "wifi-card.md"},
                {"name": "No id", "file": "noid.md"},
                {"id": "sub", "name": "Sub", "file": "more/sub.md"}
            ]
        }"#;
        let ids: Vec<_> = Index::parse(json)
            .unwrap()
            .templates
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, ["ok", "sub"]);

        let newer = Index::parse(r#"{"schema": 2, "templates": []}"#).unwrap_err();
        assert!(newer.to_string().contains("newer version"), "{newer}");
        let html = Index::parse("<html>404</html>").unwrap_err();
        assert!(
            html.to_string().contains("isn't a list of templates"),
            "{html}"
        );
    }

    #[test]
    fn files_sit_next_to_the_index() {
        let entry = |file: &str| IndexEntry {
            id: "x".into(),
            name: "X".into(),
            description: String::new(),
            version: String::new(),
            author: String::new(),
            file: file.into(),
            schema: 1,
        };
        assert_eq!(
            Index::file_url(OFFICIAL_INDEX, &entry("name-badge.md")),
            "https://raw.githubusercontent.com/Mark1626/thermal-studio/master/templates/name-badge.md"
        );
        assert_eq!(
            Index::file_url("https://example.com/t/index.json?v=2#top", &entry("a/b.md")),
            "https://example.com/t/a/b.md"
        );
    }
}
