//! The Templates tab: ready-made notes with a few blanks to fill in. Some are built in;
//! more can be added from a link or a file, and are saved so they work offline.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};
use serde::{Deserialize, Serialize};
use thermal_markdown::{FsImages, ImageResolver, NoImages, Style};
use thermal_templates::{
    Date, FieldKind, Index, IndexEntry, MAX_INDEX_BYTES, MAX_TEMPLATE_BYTES, OFFICIAL_INDEX,
    Template, Value, Values, template_link,
};

use crate::{MARKDOWN_EXTS, Preview, SAMPLE_MD, Studio, Tab};

const ADDED_KEY: &str = "templates";
const SELECTED_KEY: &str = "template";

/// A template the user added, kept exactly as downloaded.
#[derive(Clone, Serialize, Deserialize)]
pub struct Saved {
    /// An `https://` link or a file path; Refresh reloads the template from here.
    origin: String,
    source: String,
}

struct Added {
    saved: Saved,
    template: Template,
}

/// The official list of templates, loaded the first time the "Add" form opens.
enum Catalog {
    NotLoaded,
    Loading(mpsc::Receiver<Result<Vec<Offer>, String>>),
    Ready(Vec<Offer>),
    Failed(String),
}

/// A template the list offers, and the link to its file.
struct Offer {
    entry: IndexEntry,
    url: String,
}

/// A download in progress, for a new template or a refresh.
struct Pending {
    origin: String,
    rx: mpsc::Receiver<Result<String, String>>,
}

pub struct TemplatesTab {
    builtin: Vec<Template>,
    added: Vec<Added>,
    /// Saved templates this version can't read (made for a newer version, say). Kept,
    /// so they come back after an upgrade.
    unreadable: Vec<Saved>,
    /// Id of the template being filled in.
    selected: String,
    /// What has been typed into each template's form this session. Not saved: forms
    /// can hold passwords.
    values: HashMap<String, Values>,
    pub changed_at: Option<f64>,
    /// Why the current template can't be filled in.
    pub error: Option<String>,
    pub preview: Preview,
    /// The "Add a template" form: link being typed and the last error.
    adding: Option<(String, Option<String>)>,
    pending: Option<Pending>,
    catalog: Catalog,
    confirm_edit: bool,
    confirm_remove: bool,
    /// List entry to focus on the next frame (after Enter adds a row).
    focus: Option<(String, usize)>,
}

impl Default for TemplatesTab {
    fn default() -> Self {
        let builtin = thermal_templates::builtin();
        Self {
            selected: builtin[0].id.clone(),
            builtin,
            added: Vec::new(),
            unreadable: Vec::new(),
            values: HashMap::new(),
            changed_at: Some(0.0),
            error: None,
            preview: Preview::default(),
            adding: None,
            pending: None,
            catalog: Catalog::NotLoaded,
            confirm_edit: false,
            confirm_remove: false,
            focus: None,
        }
    }
}

impl TemplatesTab {
    pub fn load(&mut self, storage: Option<&dyn eframe::Storage>) {
        let Some(storage) = storage else { return };
        for saved in eframe::get_value::<Vec<Saved>>(storage, ADDED_KEY).unwrap_or_default() {
            let source = saved.source.clone();
            if self.add(saved.origin.clone(), &source).is_err() {
                self.unreadable.push(saved);
            }
        }
        if let Some(id) = storage.get_string(SELECTED_KEY)
            && self.find(&id).is_some()
        {
            self.selected = id;
        }
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let all: Vec<&Saved> = self
            .added
            .iter()
            .map(|a| &a.saved)
            .chain(&self.unreadable)
            .collect();
        eframe::set_value(storage, ADDED_KEY, &all);
        storage.set_string(SELECTED_KEY, self.selected.clone());
    }

    fn find(&self, id: &str) -> Option<&Template> {
        self.builtin
            .iter()
            .chain(self.added.iter().map(|a| &a.template))
            .find(|t| t.id == id)
    }

    fn current(&self) -> &Template {
        self.find(&self.selected).unwrap_or(&self.builtin[0])
    }

    fn current_added(&self) -> Option<&Added> {
        self.added.iter().find(|a| a.template.id == self.selected)
    }

    fn values(&self, t: &Template) -> Values {
        self.values
            .get(&t.id)
            .cloned()
            .unwrap_or_else(|| t.defaults(today()))
    }

    /// Required fields still empty, by label.
    pub fn missing(&self) -> Vec<String> {
        let t = self.current();
        t.missing(&self.values(t), today())
            .iter()
            .map(|f| f.label.clone())
            .collect()
    }

    /// Add (or update) a template and return its id.
    fn add(&mut self, origin: String, source: &str) -> Result<String, String> {
        let template = Template::parse(source).map_err(|e| e.to_string())?;
        if self.builtin.iter().any(|t| t.id == template.id) {
            return Err(format!(
                "Its id, `{}`, is already used by a built-in template.",
                template.id
            ));
        }
        let id = template.id.clone();
        let added = Added {
            saved: Saved {
                origin,
                source: source.to_string(),
            },
            template,
        };
        match self.added.iter_mut().find(|a| a.template.id == id) {
            Some(a) => *a = added,
            None => self.added.push(added),
        }
        // Its fields may have changed.
        self.values.remove(&id);
        Ok(id)
    }

    /// Read a template file and select it.
    pub fn add_file(&mut self, path: &Path) -> Result<String, String> {
        let source =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let origin = path
            .canonicalize()
            .unwrap_or(path.to_path_buf())
            .display()
            .to_string();
        let id = self.add(origin, &source)?;
        self.select(id);
        Ok(self.current().name.clone())
    }

    fn select(&mut self, id: String) {
        self.selected = id;
        self.confirm_edit = false;
        self.confirm_remove = false;
        self.changed_at = Some(0.0);
    }

    fn fetch(&mut self, ctx: &egui::Context, origin: String) {
        let (tx, rx) = mpsc::channel();
        let url = origin.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(download(&url, MAX_TEMPLATE_BYTES));
            ctx.request_repaint();
        });
        self.pending = Some(Pending { origin, rx });
    }

    fn load_catalog(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_catalog(&index_url()));
            ctx.request_repaint();
        });
        self.catalog = Catalog::Loading(rx);
    }

    fn poll_catalog(&mut self) {
        let Catalog::Loading(rx) = &self.catalog else {
            return;
        };
        self.catalog = match rx.try_recv() {
            Ok(Ok(offers)) => Catalog::Ready(offers),
            Ok(Err(e)) => Catalog::Failed(e),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Catalog::Failed("The download stopped.".into())
            }
        };
    }

    /// Images in a template file are found next to it; downloaded ones can't have any.
    fn images(&self) -> Box<dyn ImageResolver> {
        match self.current_added() {
            Some(a) if !is_link(&a.saved.origin) => {
                let dir = Path::new(&a.saved.origin).parent().map(Path::to_path_buf);
                Box::new(FsImages::new(dir.unwrap_or_default()))
            }
            _ => Box::new(NoImages),
        }
    }
}

impl Studio {
    pub(crate) fn update_template(&mut self, ctx: &egui::Context) {
        self.poll_download();
        self.templates.poll_catalog();
        let tab = &mut self.templates;
        let Some(changed) = tab.changed_at else {
            return;
        };
        let now = ctx.input(|i| i.time);
        const DEBOUNCE: f64 = 0.15;
        if now - changed < DEBOUNCE {
            ctx.request_repaint_after(Duration::from_secs_f64(DEBOUNCE - (now - changed)));
            return;
        }
        tab.changed_at = None;
        let start = Instant::now();
        let t = tab.current();
        match t.fill(&tab.values(t), today()) {
            Ok(markdown) => {
                // Shares the Markdown tab's renderer, so the fonts are only loaded once.
                let bmp = self.markdown.renderer.render(
                    &markdown,
                    &Style::default(),
                    tab.images().as_ref(),
                );
                let ms = start.elapsed().as_secs_f32() * 1000.0;
                tab.preview.set_bitmap(ctx, "template-preview", bmp, ms);
                tab.error = None;
            }
            Err(e) => {
                tab.error = Some(match e {
                    // Something typed into the form, such as a half-written date.
                    thermal_templates::Error::Value(why) => why,
                    e => format!("This template can't be filled in: {e}"),
                });
                tab.preview.clear();
            }
        }
    }

    fn poll_download(&mut self) {
        let tab = &mut self.templates;
        let Some(pending) = &tab.pending else { return };
        let result = match pending.rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The download stopped.".into()),
        };
        let origin = tab.pending.take().unwrap().origin;
        let refresh = tab.added.iter().any(|a| a.saved.origin == origin);
        match result.and_then(|source| tab.add(origin, &source)) {
            Ok(id) => {
                tab.adding = None;
                tab.select(id);
                let name = &tab.current().name;
                self.notice = Some(if refresh {
                    format!("Updated “{name}”")
                } else {
                    format!("Added “{name}”")
                });
            }
            Err(e) => match &mut tab.adding {
                Some((_, error)) => *error = Some(e),
                None => self.message = Some(format!("Couldn't update the template: {e}")),
            },
        }
    }

    pub(crate) fn template_controls(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let tab = &mut self.templates;
        ui.add_space(4.0);

        // Which template.
        let mut selected = tab.selected.clone();
        // Right to left, so the picker takes whatever room the button leaves. (Widths
        // computed from `available_width` overshoot and make the panel grow every frame.)
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .selectable_label(tab.adding.is_some(), "Add…")
                    .on_hover_text("Add a template from a link or a file")
                    .clicked()
                {
                    tab.adding = match tab.adding {
                        Some(_) => None,
                        None => Some((String::new(), None)),
                    };
                }
                egui::ComboBox::from_id_salt("template-picker")
                    .width(ui.available_width())
                    .selected_text(RichText::new(&tab.current().name).strong())
                    .show_ui(ui, |ui| {
                        for t in &tab.builtin {
                            ui.selectable_value(&mut selected, t.id.clone(), &t.name);
                        }
                        if !tab.added.is_empty() {
                            ui.separator();
                            for a in &tab.added {
                                let t = &a.template;
                                ui.selectable_value(&mut selected, t.id.clone(), &t.name);
                            }
                        }
                    });
            });
        });
        if selected != tab.selected {
            tab.select(selected);
        }

        if tab.adding.is_some() {
            self.add_template_form(ui, &ctx);
        }
        let tab = &mut self.templates;

        let t = tab.current().clone();
        if !t.description.is_empty() {
            ui.label(RichText::new(&t.description).weak());
        }
        if let Some(a) = tab.current_added() {
            let origin = a.saved.origin.clone();
            ui.add(
                egui::Label::new(RichText::new(format!("From {origin}")).small().weak()).truncate(),
            )
            .on_hover_text(&origin);
            ui.horizontal(|ui| {
                let busy = tab.pending.is_some();
                if ui
                    .add_enabled(!busy, egui::Button::new("Refresh"))
                    .on_hover_text("Load the latest version of this template")
                    .clicked()
                {
                    if is_link(&origin) {
                        tab.fetch(&ctx, origin);
                    } else if let Err(e) = tab.add_file(Path::new(&origin)) {
                        self.message = Some(format!("Couldn't update the template: {e}"));
                    }
                }
                if busy {
                    ui.spinner();
                }
                if tab.confirm_remove {
                    ui.label("Remove it?");
                    if ui.button("Remove").clicked() {
                        tab.added.retain(|a| a.template.id != t.id);
                        tab.values.remove(&t.id);
                        let first = tab.builtin[0].id.clone();
                        tab.select(first);
                    }
                    if ui.button("Keep").clicked() {
                        tab.confirm_remove = false;
                    }
                } else if ui.button("Remove").clicked() {
                    tab.confirm_remove = true;
                }
            });
        }
        let tab = &mut self.templates;
        ui.separator();

        // The form.
        let mut values = tab.values(&t);
        let before = values.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if t.fields.is_empty() {
                    ui.label(RichText::new("Nothing to fill in.").weak());
                }
                for f in &t.fields {
                    let v = values
                        .entry(f.key.clone())
                        .or_insert_with(|| f.default_value(today()));
                    let label = if f.required {
                        format!("{} *", f.label)
                    } else {
                        f.label.clone()
                    };
                    let hint = RichText::new(&f.placeholder).weak();
                    let text_width = ui.available_width();
                    match (&f.kind, v) {
                        (FieldKind::Text { .. }, Value::Text(s)) => {
                            ui.label(label);
                            ui.add(
                                egui::TextEdit::singleline(s)
                                    .hint_text(hint)
                                    .desired_width(f32::INFINITY),
                            );
                        }
                        (FieldKind::Multiline { .. }, Value::Text(s)) => {
                            ui.label(label);
                            ui.add(
                                egui::TextEdit::multiline(s)
                                    .hint_text(hint)
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                        }
                        (FieldKind::Choice { options, .. }, Value::Text(s)) => {
                            ui.label(label);
                            let shown = options
                                .iter()
                                .find(|o| o.value == *s)
                                .map_or(s.clone(), |o| o.label.clone());
                            egui::ComboBox::from_id_salt(("template-choice", &f.key))
                                .width(text_width)
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    for o in options {
                                        ui.selectable_value(s, o.value.clone(), &o.label);
                                    }
                                });
                        }
                        (FieldKind::Bool { .. }, Value::Bool(b)) => {
                            ui.checkbox(b, label);
                        }
                        (FieldKind::List { .. }, Value::List(items)) => {
                            ui.label(label);
                            list_input(ui, &f.key, items, hint, &mut tab.focus);
                        }
                        (FieldKind::Date { .. }, Value::Text(s)) => {
                            ui.label(label);
                            date_input(ui, &f.key, s);
                        }
                        // Values always start from the field's own default.
                        _ => {}
                    }
                    if !f.help.is_empty() {
                        ui.label(RichText::new(&f.help).small().weak());
                    }
                    ui.add_space(8.0);
                }

                ui.separator();
                let missing = t.missing(&values, today());
                if !missing.is_empty() {
                    let names: Vec<_> = missing.iter().map(|f| f.label.as_str()).collect();
                    ui.label(
                        RichText::new(format!("Fill in {} to print.", names.join(", "))).weak(),
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Edit as Markdown")
                        .on_hover_text("Copy this note to the Markdown tab to change it freely")
                        .clicked()
                    {
                        tab.confirm_edit = true;
                    }
                    if ui
                        .button("Clear form")
                        .on_hover_text("Go back to the template's starting values")
                        .clicked()
                    {
                        values = t.defaults(today());
                    }
                });
            });
        if values != before {
            tab.values.insert(t.id.clone(), values);
            tab.changed_at = Some(ui.ctx().input(|i| i.time));
        }

        if tab.confirm_edit {
            let untouched = self.markdown.text == SAMPLE_MD || self.markdown.text.trim().is_empty();
            let mut go = untouched;
            if !untouched {
                ui.horizontal_wrapped(|ui| {
                    ui.label("This replaces the text in the Markdown tab.");
                    go = ui.button("Replace").clicked();
                    if ui.button("Cancel").clicked() {
                        self.templates.confirm_edit = false;
                    }
                });
            }
            if go {
                self.edit_as_markdown();
            }
        }
    }

    fn add_template_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let tab = &mut self.templates;
        if matches!(tab.catalog, Catalog::NotLoaded) {
            tab.load_catalog(ctx);
        }
        let fetching = tab.pending.is_some();
        let Some((url, error)) = &mut tab.adding else {
            return;
        };
        let (mut start, mut file, mut install, mut reload) = (None, None, None, false);
        ui.group(|ui| {
            ui.label(RichText::new("Official templates").strong());
            match &tab.catalog {
                Catalog::NotLoaded | Catalog::Loading(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Loading the list…").weak());
                    });
                }
                Catalog::Failed(e) => {
                    ui.label(
                        RichText::new(format!("Couldn't load the list. {e}"))
                            .color(ui.visuals().warn_fg_color),
                    );
                    reload = ui.button("Try again").clicked();
                }
                Catalog::Ready(offers) if offers.is_empty() => {
                    ui.label(RichText::new("No more templates yet.").weak());
                }
                Catalog::Ready(offers) => {
                    for offer in offers {
                        let e = &offer.entry;
                        let have = tab.added.iter().find(|a| a.template.id == e.id);
                        let (action, enabled) = match have {
                            None => ("Add", true),
                            Some(a) if a.template.version != e.version => ("Update", true),
                            Some(_) => ("Added", false),
                        };
                        ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(
                                            enabled && !fetching,
                                            egui::Button::new(action),
                                        )
                                        .clicked()
                                    {
                                        install = Some(offer.url.clone());
                                    }
                                    // The name fills the rest of the row, from the left.
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(RichText::new(&e.name).strong())
                                                    .truncate(),
                                            )
                                            .on_hover_text(format!("Version {}", e.version));
                                        },
                                    );
                                },
                            );
                        });
                        if !e.description.is_empty() {
                            ui.label(RichText::new(&e.description).small().weak());
                        }
                        ui.add_space(4.0);
                    }
                }
            }
            ui.separator();
            ui.label("Or add one from a link");
            let resp = ui.add(
                egui::TextEdit::singleline(url)
                    .hint_text("https://…/template.md")
                    .desired_width(f32::INFINITY),
            );
            let entered = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let ready = !fetching && !url.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("Add")).clicked() || (ready && entered) {
                    start = Some(template_link(url));
                }
                if fetching {
                    ui.spinner();
                    ui.label("Downloading…");
                } else if ui.button("Open file…").clicked() {
                    file = rfd::FileDialog::new()
                        .add_filter("Template", &MARKDOWN_EXTS)
                        .pick_file();
                }
            });
            if let Some(e) = error.as_deref() {
                ui.label(RichText::new(e).color(ui.visuals().warn_fg_color));
            }
            ui.label(
                RichText::new(
                    "A template is a Markdown file with a few blanks to fill in. \
                     It can't run programs or reach the network.",
                )
                .small()
                .weak(),
            );
        });
        match start.or(install.map(Ok)) {
            Some(Ok(link)) => {
                *error = None;
                tab.fetch(ctx, link);
            }
            Some(Err(e)) => *error = Some(e),
            None => {}
        }
        if reload {
            tab.load_catalog(ctx);
        }
        if let Some(path) = file {
            match tab.add_file(&path) {
                Ok(name) => {
                    tab.adding = None;
                    self.notice = Some(format!("Added “{name}”"));
                }
                Err(e) => {
                    if let Some((_, error)) = &mut tab.adding {
                        *error = Some(e);
                    }
                }
            }
        }
    }

    fn edit_as_markdown(&mut self) {
        let tab = &mut self.templates;
        tab.confirm_edit = false;
        let t = tab.current();
        match t.fill(&tab.values(t), today()) {
            Ok(markdown) => {
                let md = &mut self.markdown;
                md.text = markdown;
                md.path = None;
                if let Some(a) = tab.current_added()
                    && !is_link(&a.saved.origin)
                {
                    md.base_dir = PathBuf::from(&a.saved.origin)
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                }
                md.changed_at = Some(0.0);
                self.tab = Tab::Markdown;
            }
            Err(e) => self.message = Some(e.to_string()),
        }
    }
}

/// One text box per entry, each with a remove button. Enter adds an entry below.
fn list_input(
    ui: &mut egui::Ui,
    key: &str,
    items: &mut Vec<String>,
    hint: RichText,
    focus: &mut Option<(String, usize)>,
) {
    let (mut remove, mut insert) = (None, None);
    for (i, item) in items.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
                let resp = ui.add(
                    egui::TextEdit::singleline(item)
                        .id_salt((key, i))
                        .hint_text(hint.clone())
                        .desired_width(f32::INFINITY),
                );
                if focus.as_ref().is_some_and(|(k, n)| k == key && *n == i) {
                    resp.request_focus();
                    *focus = None;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    insert = Some(i + 1);
                }
            });
        });
    }
    if ui.button("+ Add").clicked() {
        insert = Some(items.len());
    }
    if let Some(i) = remove {
        items.remove(i);
    } else if let Some(i) = insert {
        items.insert(i, String::new());
        *focus = Some((key.to_string(), i));
    }
}

/// A date as `2026-11-05`, with a button for today and the day written out underneath.
fn date_input(ui: &mut egui::Ui, key: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Today").clicked() {
                *value = today().to_string();
            }
            ui.add(
                egui::TextEdit::singleline(value)
                    .id_salt(("template-date", key))
                    .hint_text(RichText::new(today().to_string()).weak())
                    .desired_width(f32::INFINITY),
            );
        });
    });
    match Date::parse(value) {
        Some(d) => {
            let long = d.format("%A %-d %B %Y").unwrap_or_else(|_| d.to_string());
            ui.label(RichText::new(long).small().weak());
        }
        None if value.trim().is_empty() => {}
        None => {
            ui.label(
                RichText::new("Write the date like 2026-11-05")
                    .small()
                    .color(ui.visuals().warn_fg_color),
            );
        }
    }
}

/// The local date.
fn today() -> Date {
    let d = jiff::Zoned::now().date();
    Date::new(d.year().into(), d.month() as u32, d.day() as u32).unwrap_or(Date::MIN)
}

fn is_link(origin: &str) -> bool {
    let lower = origin.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Where the list of templates comes from: the official one, unless `THERMAL_STUDIO_INDEX`
/// names another (to try a fork, or a local copy while writing templates).
fn index_url() -> String {
    std::env::var("THERMAL_STUDIO_INDEX")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| OFFICIAL_INDEX.to_string())
}

fn fetch_catalog(index_url: &str) -> Result<Vec<Offer>, String> {
    let index = Index::parse(&download(index_url, MAX_INDEX_BYTES)?).map_err(|e| e.to_string())?;
    Ok(index
        .templates
        .into_iter()
        .map(|entry| Offer {
            url: Index::file_url(index_url, &entry),
            entry,
        })
        .collect())
}

/// Download at most `limit` bytes of text.
fn download(url: &str, limit: usize) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(url.to_ascii_lowercase().starts_with("https://"))
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent("thermal-studio")
        .build()
        .into();
    let describe = |e: ureq::Error| match e {
        ureq::Error::StatusCode(404) => "Nothing was found at that link.".to_string(),
        ureq::Error::StatusCode(code) => format!("The server refused (HTTP {code})."),
        ureq::Error::BodyExceedsLimit(_) => {
            format!("That file is too large (the limit is {} KB).", limit / 1024)
        }
        ureq::Error::Timeout(_) => "The server took too long to answer.".into(),
        ureq::Error::HostNotFound => {
            "Couldn't find that server. Check the link and your connection.".into()
        }
        e => format!("Couldn't download it: {e}"),
    };
    agent
        .get(url)
        .call()
        .map_err(describe)?
        .body_mut()
        .with_config()
        .limit(limit as u64)
        .read_to_string()
        .map_err(describe)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    /// Serve canned files on this computer, one request each, in any order; returns
    /// the server's address. Unknown paths get a 404.
    fn serve(files: &[(&str, &[u8])]) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let files: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(p, b)| (p.to_string(), b.to_vec()))
            .collect();
        std::thread::spawn(move || {
            for _ in 0..files.len() {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let n = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..n]);
                let path = request.split(' ').nth(1).unwrap_or_default();
                let (status, body) = match files.iter().find(|(p, _)| p == path) {
                    Some((_, body)) => ("200 OK", body.as_slice()),
                    None => ("404 Not Found", &[][..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn serve_once(status: &str, body: &[u8]) -> String {
        let path = if status.starts_with("200") {
            "/template.md"
        } else {
            "/gone"
        };
        format!("{}/template.md", serve(&[(path, body)]))
    }

    fn wait_for_download(studio: &mut Studio) {
        let start = Instant::now();
        while studio.templates.pending.is_some() {
            assert!(start.elapsed() < Duration::from_secs(10), "download hung");
            std::thread::sleep(Duration::from_millis(10));
            studio.poll_download();
        }
    }

    const BADGE: &str = "---\nid: badge\nname: Badge\nfields:\n  - key: name\n---\n# {{ name }}\n";

    #[test]
    fn downloads() {
        let get = |url: &str| download(url, MAX_TEMPLATE_BYTES);
        assert_eq!(get(&serve_once("200 OK", BADGE.as_bytes())).unwrap(), BADGE);
        let e = get(&serve_once("404 Not Found", b"")).unwrap_err();
        assert!(e.contains("Nothing was found"), "{e}");
        let huge = vec![b'x'; MAX_TEMPLATE_BYTES + 10];
        let e = get(&serve_once("200 OK", &huge)).unwrap_err();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn templates_from_the_list_are_added_from_next_to_it() {
        let index = Index::build([("badge.md".to_string(), BADGE.to_string())]).unwrap();
        let server = serve(&[
            ("/list/index.json", index.to_json().as_bytes()),
            ("/list/badge.md", BADGE.as_bytes()),
        ]);
        let offers = fetch_catalog(&format!("{server}/list/index.json")).unwrap();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].entry.name, "Badge");
        assert_eq!(offers[0].url, format!("{server}/list/badge.md"));

        let mut studio = Studio::default();
        studio
            .templates
            .fetch(&egui::Context::default(), offers[0].url.clone());
        wait_for_download(&mut studio);
        assert_eq!(studio.templates.selected, "badge");

        let e = fetch_catalog(&serve_once("404 Not Found", b""))
            .err()
            .unwrap();
        assert!(e.contains("Nothing was found"), "{e}");
    }

    #[test]
    fn adding_from_a_link_selects_the_template() {
        let mut studio = Studio::default();
        let ctx = egui::Context::default();
        studio.templates.adding = Some((String::new(), None));
        let link = serve_once("200 OK", BADGE.as_bytes());
        studio.templates.fetch(&ctx, link.clone());
        wait_for_download(&mut studio);
        assert_eq!(studio.templates.selected, "badge");
        assert!(studio.templates.adding.is_none(), "the form closes");
        assert_eq!(studio.notice.as_deref(), Some("Added “Badge”"));
        assert_eq!(studio.templates.current_added().unwrap().saved.origin, link);

        // A bad template keeps the form open and says why.
        studio.templates.adding = Some((String::new(), None));
        studio
            .templates
            .fetch(&ctx, serve_once("200 OK", b"<html>Not a template</html>"));
        wait_for_download(&mut studio);
        let (_, error) = studio.templates.adding.as_ref().unwrap();
        assert!(
            error.as_deref().unwrap().contains("must start with"),
            "{error:?}"
        );
    }

    /// Run with `--ignored` to check that HTTPS (certificates and all) works.
    #[test]
    #[ignore = "needs the internet"]
    fn downloads_over_https() {
        if let Err(e) = download("https://example.com/", MAX_TEMPLATE_BYTES) {
            panic!("{e}");
        }
    }

    #[test]
    fn added_templates_can_not_replace_built_in_ones() {
        let mut tab = TemplatesTab::default();
        let wifi = include_str!("../../../crates/thermal-templates/templates/wifi-card.md");
        assert!(tab.add("x".into(), wifi).unwrap_err().contains("built-in"));

        assert_eq!(tab.add("a".into(), BADGE).unwrap(), "badge");
        let renamed = BADGE.replace("Badge", "Name badge");
        tab.add("b".into(), &renamed).unwrap();
        assert_eq!(tab.added.len(), 1, "same id replaces");
        assert_eq!(tab.find("badge").unwrap().name, "Name badge");
    }
}
