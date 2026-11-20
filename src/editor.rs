//! The page and its text: laying paragraphs out with egui's text layout,
//! drawing the cursor and selection, and turning keys and clicks into
//! edits. Every edit goes through `checkpoint` first, for undo (typing in
//! a row counts as one step).

use crate::doc::{Align, Doc, Fmt, Kind, Para, Pos};
use atelier::egui::{
    self, Color32, FontId, Galley, Key, Pos2, Rect, Sense, Stroke, Ui, pos2,
    text::{CCursor, LayoutJob, TextFormat},
    vec2,
};
use atelier::{fonts, theme};
use std::sync::Arc;

/// A Letter-size page at 96 dpi with one-inch margins, as Google Docs has.
pub const PAGE_W: f32 = 816.0;
pub const MARGIN: f32 = 96.0;
const PT: f32 = 4.0 / 3.0;

struct Laid {
    galley: Arc<Galley>,
    /// Where the galley's top left is.
    origin: Pos2,
    /// The paragraph's band, spacing included, for clicks between them.
    top: f32,
    bottom: f32,
}

pub struct Editor {
    pub doc: Doc,
    pub cursor: Pos,
    pub anchor: Pos,
    /// A format switched on with nothing selected, for what's typed next.
    pub pending: Option<Fmt>,
    undo: Vec<(Doc, Pos, Pos)>,
    redo: Vec<(Doc, Pos, Pos)>,
    typing_since: Option<f64>,
    goal_x: Option<f32>,
    laid: Vec<Laid>,
    pub zoom: f32,
    pub reveal: bool,
    dragging: bool,
    pub dirty: bool,
    /// What was copied here last, formats and all, used when that same
    /// text is pasted back.
    clip: Option<(String, Doc)>,
    blink: f64,
    /// Search hits to light up.
    pub highlights: Vec<(Pos, Pos)>,
}

impl Editor {
    pub fn new(doc: Doc) -> Editor {
        Editor {
            doc,
            cursor: Pos::default(),
            anchor: Pos::default(),
            pending: None,
            undo: Vec::new(),
            redo: Vec::new(),
            typing_since: None,
            goal_x: None,
            laid: Vec::new(),
            zoom: 1.0,
            reveal: false,
            dragging: false,
            dirty: false,
            clip: None,
            blink: 0.0,
            highlights: Vec::new(),
        }
    }

    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    pub fn range(&self) -> (Pos, Pos) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    pub fn select(&mut self, a: Pos, b: Pos) {
        self.anchor = self.doc.clamp(a);
        self.cursor = self.doc.clamp(b);
        self.pending = None;
        self.goal_x = None;
        self.reveal = true;
    }

    /// Remembers the document before a change, for undo. Typing in a row
    /// (within two seconds of the last key) adds to the same step.
    pub fn checkpoint(&mut self, typing: bool, now: f64) {
        if typing {
            if let Some(t) = self.typing_since {
                if now - t < 2.0 {
                    self.typing_since = Some(now);
                    return;
                }
            }
            self.typing_since = Some(now);
        } else {
            self.typing_since = None;
        }
        self.undo.push((self.doc.clone(), self.cursor, self.anchor));
        if self.undo.len() > 300 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some((doc, c, a)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.doc, doc), self.cursor, self.anchor));
            (self.cursor, self.anchor) = (self.doc.clamp(c), self.doc.clamp(a));
            self.typing_since = None;
            self.dirty = true;
            self.reveal = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some((doc, c, a)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.doc, doc), self.cursor, self.anchor));
            (self.cursor, self.anchor) = (self.doc.clamp(c), self.doc.clamp(a));
            self.dirty = true;
            self.reveal = true;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The format the toolbar shows: the selection's start, or what typing
    /// at the cursor would give.
    pub fn current_fmt(&self) -> Fmt {
        if let Some(f) = &self.pending {
            return f.clone();
        }
        let (a, b) = self.range();
        if a != b {
            let p = &self.doc.paras[a.p];
            if a.c < p.len() {
                return p.fmt_at(a.c + 1);
            }
        }
        self.doc.paras[self.cursor.p].fmt_at(self.cursor.c)
    }

    /// Bold, italic... on the selection, or for what's typed next.
    pub fn apply(&mut self, now: f64, f: impl Fn(&mut Fmt)) {
        if self.has_selection() {
            self.checkpoint(false, now);
            let (a, b) = self.range();
            self.doc.format(a, b, &f);
        } else {
            let mut fmt = self.current_fmt();
            f(&mut fmt);
            self.pending = Some(fmt);
        }
    }

    /// Paragraph kind or alignment for every paragraph the selection touches.
    pub fn set_paras(&mut self, now: f64, f: impl Fn(&mut Para)) {
        self.checkpoint(false, now);
        let (a, b) = self.range();
        for p in &mut self.doc.paras[a.p..=b.p] {
            f(p);
        }
    }

    pub fn insert_text(&mut self, text: &str, now: f64) {
        self.checkpoint(text.len() <= 1, now);
        let (a, b) = self.range();
        let at = self.doc.delete(a, b);
        let fmt = self.pending.take().unwrap_or_else(|| self.doc.paras[at.p].fmt_at(at.c));
        let end = self.doc.insert(at, text, &fmt);
        self.cursor = end;
        self.anchor = end;
        self.goal_x = None;
        self.reveal = true;
    }

    fn delete_selection(&mut self, now: f64) {
        self.checkpoint(false, now);
        let (a, b) = self.range();
        let at = self.doc.delete(a, b);
        self.select(at, at);
    }

    pub fn copy(&mut self, ctx: &egui::Context) {
        let (a, b) = self.range();
        if a == b {
            return;
        }
        let text = self.doc.text_between(a, b);
        ctx.copy_text(text.clone());
        self.clip = Some((text, self.doc.slice(a, b)));
    }

    pub fn paste(&mut self, text: &str, now: f64) {
        let ours = self.clip.as_ref().filter(|(t, _)| t.replace("\r\n", "\n") == text.replace("\r\n", "\n"));
        match ours.map(|(_, d)| d.clone()) {
            Some(frag) => {
                self.checkpoint(false, now);
                let (a, b) = self.range();
                let at = self.doc.delete(a, b);
                let end = self.doc.insert_doc(at, &frag);
                self.select(end, end);
            }
            None => self.insert_text(text, now),
        }
    }

    /// Markdown as you type: "# " makes a heading, "- " a list...
    fn autoformat(&mut self, now: f64) -> bool {
        let p = self.cursor.p;
        let before: String = self.doc.paras[p].text().chars().take(self.cursor.c).collect();
        if self.doc.paras[p].kind != Kind::Normal || self.has_selection() {
            return false;
        }
        let kind = match before.as_str() {
            "#" => Kind::H1,
            "##" => Kind::H2,
            "###" => Kind::H3,
            "-" | "*" => Kind::Bullet,
            "1." => Kind::Numbered,
            ">" => Kind::Quote,
            _ => return false,
        };
        self.checkpoint(false, now);
        self.doc.delete(Pos::new(p, 0), self.cursor);
        self.doc.paras[p].kind = kind;
        self.select(Pos::new(p, 0), Pos::new(p, 0));
        true
    }

    fn enter(&mut self, now: f64) {
        let p = self.cursor.p;
        let para = &self.doc.paras[p];
        // Enter on an empty list item or quote ends the list.
        if !self.has_selection() && para.len() == 0 && matches!(para.kind, Kind::Bullet | Kind::Numbered | Kind::Quote | Kind::Code) {
            self.checkpoint(false, now);
            self.doc.paras[p].kind = Kind::Normal;
            return;
        }
        self.checkpoint(false, now);
        let (a, b) = self.range();
        let at = self.doc.delete(a, b);
        let fmt = self.doc.paras[at.p].fmt_at(at.c);
        let next = self.doc.split(at);
        self.select(next, next);
        // Carry bold and the like on, not a heading's size.
        if fmt != Fmt::default() && self.doc.paras[next.p].len() == 0 {
            self.pending = Some(fmt);
        }
    }

    fn backspace(&mut self, word: bool, now: f64) {
        if self.has_selection() {
            return self.delete_selection(now);
        }
        let pos = self.cursor;
        if pos.c == 0 {
            let para = &self.doc.paras[pos.p];
            if para.kind != Kind::Normal {
                // First out of the list or heading, then join.
                self.checkpoint(false, now);
                self.doc.paras[pos.p].kind = Kind::Normal;
                return;
            }
            if pos.p == 0 {
                return;
            }
            self.checkpoint(false, now);
            let prev = Pos::new(pos.p - 1, self.doc.paras[pos.p - 1].len());
            if self.doc.paras[pos.p - 1].len() == 0 {
                // Joining onto an empty paragraph keeps this one's kind.
                self.doc.paras.remove(pos.p - 1);
                return self.select(Pos::new(pos.p - 1, 0), Pos::new(pos.p - 1, 0));
            }
            let at = self.doc.delete(prev, pos);
            return self.select(at, at);
        }
        self.checkpoint(!word, now);
        let from = if word { self.doc.word_left(pos) } else { Pos::new(pos.p, pos.c - 1) };
        let at = self.doc.delete(from, pos);
        self.select(at, at);
    }

    fn delete_forward(&mut self, word: bool, now: f64) {
        if self.has_selection() {
            return self.delete_selection(now);
        }
        let pos = self.cursor;
        let len = self.doc.paras[pos.p].len();
        if pos.c >= len && pos.p + 1 >= self.doc.paras.len() {
            return;
        }
        self.checkpoint(false, now);
        let to = if word {
            self.doc.word_right(pos)
        } else if pos.c >= len {
            Pos::new(pos.p + 1, 0)
        } else {
            Pos::new(pos.p, pos.c + 1)
        };
        let at = self.doc.delete(pos, to);
        self.select(at, at);
    }

    fn move_to(&mut self, pos: Pos, extend: bool) {
        self.cursor = self.doc.clamp(pos);
        if !extend {
            self.anchor = self.cursor;
        }
        self.pending = None;
        self.typing_since = None;
        self.reveal = true;
    }

    fn vertical(&mut self, down: bool, extend: bool) {
        let Some(l) = self.laid.get(self.cursor.p) else { return };
        let cc = CCursor::new(self.cursor.c);
        let x = self.goal_x.or_else(|| Some(l.galley.pos_from_cursor(cc).center().x));
        let (moved, h) = if down { l.galley.cursor_down_one_row(&cc, x) } else { l.galley.cursor_up_one_row(&cc, x) };
        let target = if h.is_some() {
            Pos::new(self.cursor.p, moved.index.0)
        } else if down && self.cursor.p + 1 < self.laid.len() {
            let n = &self.laid[self.cursor.p + 1];
            Pos::new(self.cursor.p + 1, n.galley.cursor_from_pos(vec2(x.unwrap_or(0.0), 0.0)).index.0)
        } else if !down && self.cursor.p > 0 {
            let n = &self.laid[self.cursor.p - 1];
            Pos::new(self.cursor.p - 1, n.galley.cursor_from_pos(vec2(x.unwrap_or(0.0), n.galley.size().y - 1.0)).index.0)
        } else if down {
            self.doc.end()
        } else {
            Pos::default()
        };
        self.move_to(target, extend);
        self.goal_x = x;
    }

    fn row_edge(&mut self, end: bool, extend: bool) {
        let Some(l) = self.laid.get(self.cursor.p) else { return };
        let cc = CCursor::new(self.cursor.c);
        let to = if end { l.galley.cursor_end_of_row(&cc) } else { l.galley.cursor_begin_of_row(&cc) };
        self.move_to(Pos::new(self.cursor.p, to.index.0), extend);
    }

    fn page(&mut self, down: bool, extend: bool, height: f32) {
        if let Some(r) = self.cursor_rect() {
            let y = r.center().y + if down { height } else { -height };
            let p = self.hit(pos2(r.center().x, y));
            self.move_to(p, extend);
        }
    }

    /// Keys, when the page has the keyboard. `editable` is false while
    /// Claude is at work.
    pub fn keys(&mut self, ctx: &egui::Context, editable: bool, page_height: f32) {
        let now = ctx.input(|i| i.time);
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Text(t) if editable => {
                    if t == " " && self.autoformat(now) {
                        continue;
                    }
                    self.insert_text(&t, now);
                }
                egui::Event::Copy => self.copy(ctx),
                egui::Event::Cut if editable => {
                    self.copy(ctx);
                    if self.has_selection() {
                        self.delete_selection(now);
                    }
                }
                egui::Event::Paste(text) if editable => self.paste(&text, now),
                egui::Event::Key { key, pressed: true, modifiers: m, .. } => {
                    let (cmd, shift) = (m.command, m.shift);
                    match key {
                        Key::ArrowLeft | Key::ArrowRight if !self.has_selection() || shift || cmd => {
                            let right = key == Key::ArrowRight;
                            let to = if cmd {
                                if right { self.doc.word_right(self.cursor) } else { self.doc.word_left(self.cursor) }
                            } else if right {
                                if self.cursor.c < self.doc.paras[self.cursor.p].len() {
                                    Pos::new(self.cursor.p, self.cursor.c + 1)
                                } else if self.cursor.p + 1 < self.doc.paras.len() {
                                    Pos::new(self.cursor.p + 1, 0)
                                } else {
                                    self.cursor
                                }
                            } else if self.cursor.c > 0 {
                                Pos::new(self.cursor.p, self.cursor.c - 1)
                            } else if self.cursor.p > 0 {
                                Pos::new(self.cursor.p - 1, self.doc.paras[self.cursor.p - 1].len())
                            } else {
                                self.cursor
                            };
                            self.move_to(to, shift);
                            self.goal_x = None;
                        }
                        Key::ArrowLeft => {
                            let a = self.range().0;
                            self.move_to(a, false);
                        }
                        Key::ArrowRight => {
                            let b = self.range().1;
                            self.move_to(b, false);
                        }
                        Key::ArrowUp => self.vertical(false, shift),
                        Key::ArrowDown => self.vertical(true, shift),
                        Key::Home if cmd => self.move_to(Pos::default(), shift),
                        Key::End if cmd => self.move_to(self.doc.end(), shift),
                        Key::Home => self.row_edge(false, shift),
                        Key::End => self.row_edge(true, shift),
                        Key::PageDown => self.page(true, shift, page_height),
                        Key::PageUp => self.page(false, shift, page_height),
                        Key::A if cmd => self.select(Pos::default(), self.doc.end()),
                        _ if !editable => {}
                        Key::Enter => self.enter(now),
                        Key::Backspace => self.backspace(m.ctrl || m.alt, now),
                        Key::Delete => self.delete_forward(m.ctrl, now),
                        Key::Tab if !cmd => self.insert_text("    ", now),
                        Key::Z if cmd && shift => self.redo(),
                        Key::Z if cmd => self.undo(),
                        Key::Y if cmd => self.redo(),
                        Key::B if cmd => {
                            let on = !self.current_fmt().bold;
                            self.apply(now, |f| f.bold = on);
                        }
                        Key::I if cmd => {
                            let on = !self.current_fmt().italic;
                            self.apply(now, |f| f.italic = on);
                        }
                        Key::U if cmd => {
                            let on = !self.current_fmt().underline;
                            self.apply(now, |f| f.underline = on);
                        }
                        Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 if cmd && m.alt => {
                            let kind = match key {
                                Key::Num1 => Kind::H1,
                                Key::Num2 => Kind::H2,
                                Key::Num3 => Kind::H3,
                                _ => Kind::Normal,
                            };
                            self.set_paras(now, |p| p.kind = kind);
                        }
                        Key::Num7 | Key::Num8 if cmd && shift => {
                            let kind = if key == Key::Num7 { Kind::Numbered } else { Kind::Bullet };
                            let on = self.doc.paras[self.cursor.p].kind == kind;
                            self.set_paras(now, |p| p.kind = if on { Kind::Normal } else { kind });
                        }
                        Key::L | Key::E | Key::R | Key::J if cmd && shift => {
                            let align = match key {
                                Key::E => Align::Center,
                                Key::R => Align::Right,
                                Key::J => Align::Justify,
                                _ => Align::Left,
                            };
                            self.set_paras(now, |p| p.align = align);
                        }
                        _ => {}
                    }
                    if !matches!(key, Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown) {
                        self.goal_x = None;
                    }
                    self.blink = now;
                }
                _ => {}
            }
        }
    }

    // ---- Layout and drawing ----------------------------------------------

    fn job(&self, para: &Para, width: f32) -> LayoutJob {
        let z = self.zoom;
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        job.halign = match para.align {
            Align::Center => egui::Align::Center,
            Align::Right => egui::Align::Max,
            _ => egui::Align::Min,
        };
        job.justify = para.align == Align::Justify;
        let base_color = if para.kind == Kind::Quote { theme::MUTED } else { theme::TEXT };
        let format = |fmt: &Fmt| {
            let size = fmt.size.unwrap_or(para.kind.size()) * PT * z;
            let family = fmt.font.as_deref().unwrap_or(if para.kind == Kind::Code { "Mono" } else { "Sans" });
            let color = fmt.color.map(|[r, g, b]| Color32::from_rgb(r, g, b)).map_or(base_color, |c| {
                // A colour too dark to read on the black page shows lighter.
                if theme::luminance(c) < 0.25 { theme::TEXT } else { c }
            });
            let mut f = TextFormat {
                font_id: FontId::new(size, fonts::family(family, fmt.bold || para.kind.bold())),
                color,
                italics: fmt.italic || para.kind == Kind::Quote,
                line_height: Some(size * 1.38),
                ..Default::default()
            };
            if fmt.underline {
                f.underline = Stroke::new(1.0 * z, color);
            }
            if fmt.strike {
                f.strikethrough = Stroke::new(1.0 * z, color);
            }
            f
        };
        if para.runs.is_empty() {
            job.append("", 0.0, format(&Fmt::default()));
        }
        for r in &para.runs {
            job.append(&r.text, 0.0, format(&r.fmt));
        }
        job
    }

    /// Space above and below a paragraph, in points.
    fn spacing(kind: Kind) -> (f32, f32) {
        match kind {
            Kind::Title => (0.0, 6.0),
            Kind::H1 => (18.0, 6.0),
            Kind::H2 => (16.0, 4.0),
            Kind::H3 => (14.0, 4.0),
            Kind::Bullet | Kind::Numbered => (0.0, 3.0),
            Kind::Code => (0.0, 0.0),
            _ => (0.0, 8.0),
        }
    }

    fn indent(kind: Kind) -> f32 {
        match kind {
            Kind::Bullet | Kind::Numbered => 36.0,
            Kind::Quote => 24.0,
            Kind::Code => 12.0,
            _ => 0.0,
        }
    }

    pub fn cursor_rect(&self) -> Option<Rect> {
        let l = self.laid.get(self.cursor.p)?;
        let r = l.galley.pos_from_cursor(CCursor::new(self.cursor.c));
        Some(r.translate(l.origin.to_vec2()))
    }

    /// The position under a point on the page.
    pub fn hit(&self, p: Pos2) -> Pos {
        if self.laid.is_empty() {
            return Pos::default();
        }
        let i = self
            .laid
            .iter()
            .position(|l| p.y < l.bottom)
            .unwrap_or(self.laid.len() - 1);
        let l = &self.laid[i];
        let c = l.galley.cursor_from_pos(p - l.origin).index.0;
        Pos::new(i, c)
    }

    /// The page, in a scroll area. Returns whether it was clicked (so the
    /// app can take the keyboard back from a text field).
    pub fn show(&mut self, ui: &mut Ui, editable: bool) -> bool {
        let z = self.zoom;
        let page_w = PAGE_W * z;
        let text_w = (PAGE_W - 2.0 * MARGIN) * z;
        let mut clicked = false;
        let scroll = egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
            let avail = ui.available_width();
            let left = ui.cursor().min.x + ((avail - page_w) / 2.0).max(16.0);
            let top = ui.cursor().min.y + 24.0;

            // Lay every paragraph out, top to bottom.
            let mut y = top + MARGIN * z * 0.75;
            let mut laid = Vec::with_capacity(self.doc.paras.len());
            for para in &self.doc.paras {
                let (before, after) = Self::spacing(para.kind);
                let indent = Self::indent(para.kind) * z;
                let band_top = y;
                y += before * PT * z;
                let galley = ui.fonts_mut(|f| f.layout_job(self.job(para, text_w - indent)));
                let origin = pos2(left + MARGIN * z + indent, y);
                y += galley.size().y.max(1.0) + after * PT * z;
                laid.push(Laid { galley, origin, top: band_top, bottom: y });
            }
            let page_h = (y - top + MARGIN * z * 0.75).max(PAGE_W * 1.294 * z);
            let page = Rect::from_min_size(pos2(left, top), vec2(page_w, page_h));
            let resp = ui.allocate_rect(Rect::from_min_size(ui.cursor().min, vec2(page_w.max(avail), page_h + 48.0)), Sense::click_and_drag());
            self.laid = laid;

            let painter = ui.painter();
            painter.rect_filled(page, 4.0, theme::PANEL);
            painter.rect_stroke(page, 4.0, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);

            // Mouse: click to place the cursor, drag to select, double
            // click a word, triple click a paragraph.
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            if let Some(p) = resp.interact_pointer_pos() {
                let pos = self.hit(p);
                if resp.triple_clicked() {
                    let len = self.doc.paras[pos.p].len();
                    self.select(Pos::new(pos.p, 0), Pos::new(pos.p, len));
                } else if resp.double_clicked() {
                    let (a, b) = self.doc.word_at(pos);
                    self.select(a, b);
                } else if ui.input(|i| i.pointer.primary_pressed()) {
                    clicked = true;
                    let shift = ui.input(|i| i.modifiers.shift);
                    self.move_to(pos, shift);
                    self.dragging = true;
                    self.blink = ui.input(|i| i.time);
                } else if self.dragging && resp.dragged() {
                    self.cursor = pos;
                    self.reveal = false;
                }
            }
            if ui.input(|i| i.pointer.primary_released()) {
                self.dragging = false;
            }

            // Search hits, the selection, the text, list markers.
            let (a, b) = self.range();
            for &(ha, hb) in &self.highlights {
                self.paint_range(ui, ha, hb, Color32::from_rgba_premultiplied(0x50, 0x50, 0x20, 0x70));
            }
            if a != b {
                self.paint_range(ui, a, b, Color32::from_rgb(0x3a, 0x3a, 0x3a));
            }
            let painter = ui.painter();
            for (i, l) in self.laid.iter().enumerate() {
                let para = &self.doc.paras[i];
                if para.kind == Kind::Code {
                    let r = Rect::from_min_max(
                        pos2(left + MARGIN * z, l.top),
                        pos2(left + page_w - MARGIN * z, l.bottom),
                    );
                    painter.rect_filled(r, 0.0, theme::RAISED);
                }
                if para.kind == Kind::Quote {
                    let x = left + MARGIN * z + 6.0 * z;
                    painter.vline(x, l.origin.y..=l.origin.y + l.galley.size().y, Stroke::new(3.0 * z, theme::FAINT));
                }
                if para.kind.is_list() {
                    let first = l.galley.rows.first().map(|r| r.size.y).unwrap_or(16.0);
                    let cy = l.origin.y + first / 2.0;
                    let font = FontId::proportional(Kind::Normal.size() * PT * z);
                    let mark = if para.kind == Kind::Bullet { "•".to_owned() } else { format!("{}.", self.doc.list_number(i)) };
                    painter.text(pos2(l.origin.x - 10.0 * z, cy), egui::Align2::RIGHT_CENTER, mark, font, theme::TEXT);
                }
                painter.galley(l.origin, l.galley.clone(), theme::TEXT);
            }

            // The cursor, blinking.
            if let Some(r) = self.cursor_rect() {
                if editable {
                    let t = ui.input(|i| i.time) - self.blink;
                    if t % 1.0 < 0.6 || self.dragging {
                        painter.vline(r.left(), r.y_range(), Stroke::new(2.0, theme::TEXT));
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                }
                if self.reveal {
                    ui.scroll_to_rect(r.expand(40.0), None);
                    self.reveal = false;
                }
            }
        });
        let _ = scroll;
        clicked
    }

    fn paint_range(&self, ui: &Ui, a: Pos, b: Pos, color: Color32) {
        let painter = ui.painter();
        for p in a.p..=b.p.min(self.laid.len().saturating_sub(1)) {
            let l = &self.laid[p];
            let from = if p == a.p { a.c } else { 0 };
            let to = if p == b.p { b.c } else { usize::MAX };
            let mut start = 0;
            for row in &l.galley.rows {
                let n = row.char_count_excluding_newline().0;
                let (s, e) = (from.max(start), to.min(start + n));
                // An empty or fully crossed line shows a little of the break.
                let crosses = to > start + n && p != b.p;
                if s <= e && (s < e || crosses) {
                    let x1 = row.pos.x + row.x_offset(egui::text::CharIndex(s - start));
                    let mut x2 = row.pos.x + row.x_offset(egui::text::CharIndex(e - start));
                    if crosses && e == start + n {
                        x2 += 6.0 * self.zoom;
                    }
                    let r = Rect::from_min_max(pos2(x1, row.pos.y), pos2(x2, row.pos.y + row.size.y));
                    painter.rect_filled(r.translate(l.origin.to_vec2()), 0.0, color);
                }
                start += row.char_count_including_newline().0;
            }
        }
    }
}
