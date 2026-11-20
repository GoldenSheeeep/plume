//! Docs: documents. Opens and saves Word files (.docx), Markdown and plain
//! text. Claude, in the panel on the right, reads and edits the open
//! document through tools.rs.

mod doc;
mod docx;
mod editor;
mod tools;

use doc::{Align, Doc, Kind};
use editor::Editor;
use atelier::bridge::Bridge;
use atelier::claude::Claude;
use atelier::closing::{Choice, CloseGuard};
use atelier::egui::{self, Key, RichText, Ui};
use atelier::icons::{self, Icon};
use atelier::{files, fonts, theme, titlebar, widgets};
use std::path::{Path, PathBuf};

struct Find {
    query: String,
    replace: String,
    current: usize,
    focus: bool,
}

pub struct App {
    pub ed: Editor,
    path: Option<PathBuf>,
    claude: Claude,
    bridge: Bridge,
    close: CloseGuard,
    find: Option<Find>,
    status: Option<(String, f64)>,
}

fn load(path: &Path) -> Result<Doc, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase().as_str() {
        "docx" => docx::load(&bytes),
        "md" | "markdown" => Ok(doc::from_markdown(&String::from_utf8_lossy(&bytes))),
        _ => {
            let text = String::from_utf8_lossy(&bytes);
            let mut d = Doc { paras: text.lines().map(|l| doc::Para::plain(Kind::Normal, l)).collect() };
            for p in &mut d.paras {
                p.normalize();
            }
            if d.paras.is_empty() {
                d = Doc::default();
            }
            Ok(d)
        }
    }
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        let bridge = Bridge::start("docs", cc.egui_ctx.clone());
        let claude = Claude::new(
            "Docs",
            bridge.path.clone(),
            "word processor",
            &["Summarise this document", "Fix spelling and grammar", "Make the selected paragraph shorter and clearer"],
        );
        let mut app = App {
            ed: Editor::new(Doc::default()),
            path: None,
            claude,
            bridge,
            close: CloseGuard::default(),
            find: None,
            status: None,
        };
        if let Some(p) = atelier::file_arg() {
            app.open_path(p);
        }
        app
    }

    fn name(&self) -> String {
        self.path.as_deref().map(files::stem).unwrap_or_else(|| "Untitled document".to_owned())
    }

    fn say(&mut self, msg: impl Into<String>, ctx: &egui::Context) {
        self.status = Some((msg.into(), ctx.input(|i| i.time) + 4.0));
    }

    fn open_path(&mut self, p: PathBuf) {
        match load(&p) {
            Ok(d) => {
                self.ed = Editor::new(d);
                self.path = Some(p);
            }
            Err(e) => self.status = Some((format!("Couldn't open {}: {e}", p.display()), f64::MAX)),
        }
    }

    fn open(&mut self) {
        if let Some(p) = files::open("Documents", &["docx", "md", "txt"]) {
            self.open_path(p);
        }
    }

    fn save(&mut self, ctx: &egui::Context) -> bool {
        let path = match &self.path {
            Some(p) if matches!(p.extension().and_then(|e| e.to_str()), Some("docx" | "md" | "txt")) => p.clone(),
            _ => match files::save("Word document", &["docx"], &format!("{}.docx", self.name()), None) {
                Some(p) => p,
                None => return false,
            },
        };
        self.save_to(path, ctx)
    }

    fn save_as(&mut self, ctx: &egui::Context, kind: &str, ext: &str) {
        let dir = self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        if let Some(p) = files::save(kind, &[ext], &format!("{}.{ext}", self.name()), dir.as_deref()) {
            self.save_to(p, ctx);
        }
    }

    fn save_to(&mut self, path: PathBuf, ctx: &egui::Context) -> bool {
        let result = match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
            "md" => std::fs::write(&path, doc::to_markdown(&self.ed.doc)).map_err(|e| e.to_string()),
            "txt" => {
                let text: Vec<String> = self.ed.doc.paras.iter().map(|p| p.text()).collect();
                std::fs::write(&path, text.join("\n") + "\n").map_err(|e| e.to_string())
            }
            _ => docx::save(&self.ed.doc, &path),
        };
        match result {
            Ok(()) => {
                self.path = Some(path);
                self.ed.dirty = false;
                self.say("Saved", ctx);
                true
            }
            Err(e) => {
                self.say(format!("Couldn't save: {e}"), ctx);
                false
            }
        }
    }

    fn new_window() {
        std::process::Command::new(std::env::current_exe().unwrap()).spawn().ok();
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            if let egui::Event::Key { key, pressed: true, modifiers: m, .. } = ev {
                if !m.command {
                    continue;
                }
                match key {
                    Key::S if m.shift => self.save_as(ctx, "Word document", "docx"),
                    Key::S => {
                        self.save(ctx);
                    }
                    Key::O => self.open(),
                    Key::N => Self::new_window(),
                    Key::J if !m.shift => self.claude.toggle(),
                    Key::F | Key::H => self.open_find(),
                    Key::Plus | Key::Equals => self.ed.zoom = (self.ed.zoom + 0.1).min(2.5),
                    Key::Minus => self.ed.zoom = (self.ed.zoom - 0.1).max(0.5),
                    Key::Num0 if !m.alt => self.ed.zoom = 1.0,
                    _ => {}
                }
            }
        }
    }

    fn open_find(&mut self) {
        let (a, b) = self.ed.range();
        let query = if a != b && a.p == b.p { self.ed.doc.text_between(a, b) } else { String::new() };
        self.find = Some(Find { query, replace: String::new(), current: 0, focus: true });
    }

    fn menus(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.menu_button("File", |ui| {
            if ui.button("New").clicked() {
                Self::new_window();
            }
            if ui.button("Open…").clicked() {
                self.open();
            }
            ui.separator();
            if ui.button("Save").clicked() {
                self.save(&ctx);
            }
            if ui.button("Save as Word…").clicked() {
                self.save_as(&ctx, "Word document", "docx");
            }
            if ui.button("Save as Markdown…").clicked() {
                self.save_as(&ctx, "Markdown", "md");
            }
            if ui.button("Save as plain text…").clicked() {
                self.save_as(&ctx, "Text", "txt");
            }
        });
        ui.menu_button("Edit", |ui| {
            ui.add_enabled_ui(!self.claude.busy(), |ui| {
                if ui.add_enabled(self.ed.can_undo(), egui::Button::new("Undo")).clicked() {
                    self.ed.undo();
                }
                if ui.add_enabled(self.ed.can_redo(), egui::Button::new("Redo")).clicked() {
                    self.ed.redo();
                }
                ui.separator();
                if ui.button("Select all").clicked() {
                    self.ed.select(Default::default(), self.ed.doc.end());
                }
                if ui.button("Find and replace").clicked() {
                    self.open_find();
                }
            });
        });
        ui.menu_button("View", |ui| {
            for z in [0.75, 1.0, 1.25, 1.5] {
                if ui.button(format!("{}%", (z * 100.0) as i32)).clicked() {
                    self.ed.zoom = z;
                }
            }
        });
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        let fmt = self.ed.current_fmt();
        let para = self.ed.doc.paras[self.ed.cursor.p].clone();
        ui.add_enabled_ui(!self.claude.busy(), |ui| {
            if icons::button(ui, Icon::Undo, false, "Undo (Ctrl+Z)").clicked() {
                self.ed.undo();
            }
            if icons::button(ui, Icon::Redo, false, "Redo (Ctrl+Y)").clicked() {
                self.ed.redo();
            }
            widgets::separator(ui);
            let names: Vec<&str> = Kind::ALL.iter().map(|(_, n)| *n).collect();
            if let Some(i) = widgets::choice(ui, "kind", para.kind.name(), 120.0, &names) {
                let kind = Kind::ALL[i].0;
                self.ed.set_paras(now, |p| p.kind = kind);
            }
            let font = fmt.font.clone().unwrap_or_else(|| if para.kind == Kind::Code { "Mono".into() } else { "Sans".into() });
            if let Some(i) = widgets::choice(ui, "font", &font, 64.0, &fonts::FAMILIES) {
                let f = fonts::FAMILIES[i];
                self.ed.apply(now, |x| x.font = (f != "Sans").then(|| f.to_owned()));
            }
            widgets::separator(ui);
            let size = fmt.size.unwrap_or(para.kind.size());
            if icons::button(ui, Icon::Minus, false, "Smaller text").clicked() {
                self.ed.apply(now, |x| x.size = Some((x.size.unwrap_or(size) - 1.0).max(6.0)));
            }
            ui.label(RichText::new(format!("{size}")).color(theme::MUTED));
            if icons::button(ui, Icon::Plus, false, "Larger text").clicked() {
                self.ed.apply(now, |x| x.size = Some((x.size.unwrap_or(size) + 1.0).min(96.0)));
            }
            widgets::separator(ui);
            let bold = egui::FontId::new(14.0, fonts::family("Sans", true));
            if icons::text_button(ui, RichText::new("B").font(bold), fmt.bold, "Bold (Ctrl+B)").clicked() {
                self.ed.apply(now, |x| x.bold = !fmt.bold);
            }
            if icons::text_button(ui, RichText::new("I").italics(), fmt.italic, "Italic (Ctrl+I)").clicked() {
                self.ed.apply(now, |x| x.italic = !fmt.italic);
            }
            if icons::text_button(ui, RichText::new("U").underline(), fmt.underline, "Underline (Ctrl+U)").clicked() {
                self.ed.apply(now, |x| x.underline = !fmt.underline);
            }
            if icons::text_button(ui, RichText::new("S").strikethrough(), fmt.strike, "Strikethrough").clicked() {
                self.ed.apply(now, |x| x.strike = !fmt.strike);
            }
            let color = fmt.color.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b));
            if let Some(c) = widgets::color_button(ui, Icon::TextColor, color, "Text colour") {
                self.ed.apply(now, |x| x.color = c.map(|c| [c.r(), c.g(), c.b()]));
            }
            widgets::separator(ui);
            for (icon, al, tip) in [
                (Icon::AlignLeft, Align::Left, "Align left (Ctrl+Shift+L)"),
                (Icon::AlignCenter, Align::Center, "Centre (Ctrl+Shift+E)"),
                (Icon::AlignRight, Align::Right, "Align right (Ctrl+Shift+R)"),
                (Icon::AlignJustify, Align::Justify, "Justify (Ctrl+Shift+J)"),
            ] {
                if icons::button(ui, icon, para.align == al, tip).clicked() {
                    self.ed.set_paras(now, |p| p.align = al);
                }
            }
            widgets::separator(ui);
            for (icon, kind, tip) in [
                (Icon::Bullets, Kind::Bullet, "Bulleted list (Ctrl+Shift+8)"),
                (Icon::Numbers, Kind::Numbered, "Numbered list (Ctrl+Shift+7)"),
                (Icon::Quote, Kind::Quote, "Quote"),
            ] {
                let on = para.kind == kind;
                if icons::button(ui, icon, on, tip).clicked() {
                    self.ed.set_paras(now, |p| p.kind = if on { Kind::Normal } else { kind });
                }
            }
        });
    }

    fn find_bar(&mut self, ui: &mut Ui) {
        let Some(f) = &mut self.find else { return };
        let mut close = false;
        let mut go: Option<isize> = None;
        let mut replace_one = false;
        let mut replace_all = false;
        egui::Panel::top("find")
            .exact_size(40.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    icons::paint(ui.painter(), Icon::Search, ui.cursor().min + egui::vec2(8.0, 14.0), theme::MUTED);
                    ui.add_space(20.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut f.query).hint_text("Find").desired_width(200.0));
                    if f.focus {
                        r.request_focus();
                        f.focus = false;
                    }
                    if r.changed() {
                        f.current = 0;
                        go = Some(0);
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        go = Some(if ui.input(|i| i.modifiers.shift) { -1 } else { 1 });
                        r.request_focus();
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    let hits = self.ed.doc.find(&f.query);
                    let label = if f.query.is_empty() { String::new() } else if hits.is_empty() { "No results".into() } else { format!("{} of {}", f.current.min(hits.len() - 1) + 1, hits.len()) };
                    ui.label(RichText::new(label).color(theme::MUTED));
                    if icons::button(ui, Icon::Up, false, "Previous (Shift+Enter)").clicked() {
                        go = Some(-1);
                    }
                    if icons::button(ui, Icon::Down, false, "Next (Enter)").clicked() {
                        go = Some(1);
                    }
                    widgets::separator(ui);
                    ui.add(egui::TextEdit::singleline(&mut f.replace).hint_text("Replace with").desired_width(180.0));
                    ui.add_enabled_ui(!self.claude.busy(), |ui| {
                        replace_one = ui.button("Replace").clicked();
                        replace_all = ui.button("Replace all").clicked();
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Close, false, "Close (Escape)").clicked() {
                            close = true;
                        }
                    });
                });
            });
        let hits = self.ed.doc.find(&f.query);
        if let Some(step) = go {
            if !hits.is_empty() {
                f.current = (f.current as isize + step).rem_euclid(hits.len() as isize) as usize;
                let (a, b) = hits[f.current];
                self.ed.select(a, b);
            }
        }
        let now = ui.input(|i| i.time);
        if replace_one && !hits.is_empty() {
            let (a, b) = hits[f.current.min(hits.len() - 1)];
            self.ed.select(a, b);
            self.ed.insert_text(&f.replace.clone(), now);
        }
        if replace_all && !hits.is_empty() {
            self.ed.checkpoint(false, now);
            for &(a, b) in hits.iter().rev() {
                let fmt = self.ed.doc.paras[a.p].fmt_at(a.c + 1);
                let at = self.ed.doc.delete(a, b);
                self.ed.doc.insert(at, &f.replace, &fmt);
            }
            let n = hits.len();
            self.say(format!("Replaced {n}"), &ui.ctx().clone());
        }
        self.ed.highlights = if close { Vec::new() } else { self.ed.doc.find(&self.find.as_ref().unwrap().query) };
        if close {
            self.find = None;
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("status")
            .exact_size(28.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin::symmetric(14, 4)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let now = ui.input(|i| i.time);
                    if let Some((msg, until)) = &self.status {
                        if now < *until {
                            ui.label(RichText::new(msg).size(12.0).color(theme::MUTED));
                            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let words = self.ed.doc.word_count();
                        ui.label(RichText::new(format!("{}%   ·   {words} words", (self.ed.zoom * 100.0).round())).size(12.0).color(theme::MUTED));
                    });
                });
            });
    }

    fn context(&self) -> String {
        let (a, b) = self.ed.range();
        let mut s = format!(
            "Document \"{}\", {} paragraphs, cursor in paragraph {}",
            self.name(),
            self.ed.doc.paras.len(),
            self.ed.cursor.p
        );
        if a != b {
            let text: String = self.ed.doc.text_between(a, b).chars().take(300).collect();
            s.push_str(&format!(", selected (paragraphs {} to {}): \"{text}\"", a.p, b.p));
        }
        s
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for req in self.bridge.pending() {
            tools::handle(self, req);
        }
        self.close.check(ctx, self.ed.dirty);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let title = format!("{}{}", self.name(), if self.ed.dirty { " •" } else { "" });
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} – Docs", self.name())));
        let (mut toggle, open) = (false, self.claude.open);
        titlebar::show(ui, &title, |ui| self.menus(ui), |ui| toggle = widgets::claude_toggle(ui, open));
        if toggle {
            self.claude.toggle();
        }
        widgets::toolbar(ui, |ui| self.toolbar(ui));
        self.find_bar(ui);
        self.status_bar(ui);
        let context = self.context();
        self.claude.panel(ui, || context);
        self.shortcuts(&ctx);
        let editable = !self.claude.busy();
        let page_height = ui.available_height() - 80.0;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::BG)).show(ui, |ui| {
            if self.ed.show(ui, editable) {
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
            }
        });
        // The page has the keyboard whenever no text field does.
        if ctx.memory(|m| m.focused().is_none()) {
            self.ed.keys(&ctx, editable, page_height);
        }
        if let Some(Choice::Save) = self.close.dialog(&ctx, &self.name()) {
            if self.save(&ctx) {
                self.close.close_now(&ctx);
            }
        }
    }
}

fn main() -> eframe::Result {
    atelier::run("docs", "Docs", tools::list, App::new)
}
