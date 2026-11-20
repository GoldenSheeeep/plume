//! A document: paragraphs, each a kind (heading, list item...), an
//! alignment and runs of text sharing one format. Positions are a
//! paragraph and a character offset in it. Also its Markdown form, which is
//! what Claude reads and writes, and what .md files are.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Normal,
    Title,
    H1,
    H2,
    H3,
    Bullet,
    Numbered,
    Quote,
    Code,
}

impl Kind {
    pub const ALL: [(Kind, &'static str); 9] = [
        (Kind::Normal, "Normal text"),
        (Kind::Title, "Title"),
        (Kind::H1, "Heading 1"),
        (Kind::H2, "Heading 2"),
        (Kind::H3, "Heading 3"),
        (Kind::Bullet, "Bulleted list"),
        (Kind::Numbered, "Numbered list"),
        (Kind::Quote, "Quote"),
        (Kind::Code, "Code"),
    ];

    pub fn name(self) -> &'static str {
        Kind::ALL.iter().find(|(k, _)| *k == self).map(|(_, n)| *n).unwrap_or("Normal text")
    }

    /// Its text size, in points (Google Docs' defaults).
    pub fn size(self) -> f32 {
        match self {
            Kind::Title => 26.0,
            Kind::H1 => 20.0,
            Kind::H2 => 16.0,
            Kind::H3 => 14.0,
            Kind::Code => 10.0,
            _ => 11.0,
        }
    }

    pub fn bold(self) -> bool {
        matches!(self, Kind::H1 | Kind::H2 | Kind::H3)
    }

    pub fn is_list(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Numbered)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Fmt {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// None: automatic (white on screen, black on paper).
    pub color: Option<[u8; 3]>,
    /// Points; None: the paragraph's own size.
    pub size: Option<f32>,
    /// "Sans", "Serif" or "Mono"; None: the document's (Sans).
    pub font: Option<String>,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub fmt: Fmt,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Para {
    pub kind: Kind,
    pub align: Align,
    pub runs: Vec<Run>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Pos {
    pub p: usize,
    pub c: usize,
}

impl Pos {
    pub fn new(p: usize, c: usize) -> Pos {
        Pos { p, c }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Doc {
    pub paras: Vec<Para>,
}

impl Default for Doc {
    fn default() -> Doc {
        Doc { paras: vec![Para::default()] }
    }
}

fn byte_at(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map(|(b, _)| b).unwrap_or(s.len())
}

impl Para {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.text.chars().count()).sum()
    }

    pub fn plain(kind: Kind, text: &str) -> Para {
        Para { kind, align: Align::Left, runs: vec![Run { text: text.to_owned(), fmt: Fmt::default() }] }
    }

    /// Merges neighbouring runs with the same format and drops empty ones.
    pub fn normalize(&mut self) {
        let mut out: Vec<Run> = Vec::new();
        for r in self.runs.drain(..) {
            if r.text.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.fmt == r.fmt => last.text.push_str(&r.text),
                _ => out.push(r),
            }
        }
        self.runs = out;
    }

    /// Splits runs so a run boundary falls at character `c`; returns the
    /// index of the first run at or after it.
    fn split_at(&mut self, c: usize) -> usize {
        let mut seen = 0;
        for i in 0..self.runs.len() {
            let n = self.runs[i].text.chars().count();
            if c == seen {
                return i;
            }
            if c < seen + n {
                let b = byte_at(&self.runs[i].text, c - seen);
                let tail = self.runs[i].text.split_off(b);
                let fmt = self.runs[i].fmt.clone();
                self.runs.insert(i + 1, Run { text: tail, fmt });
                return i + 1;
            }
            seen += n;
        }
        self.runs.len()
    }

    /// The format of the character before `c` (what typing there continues).
    pub fn fmt_at(&self, c: usize) -> Fmt {
        let mut seen = 0;
        let mut last = None;
        for r in &self.runs {
            let n = r.text.chars().count();
            if c > seen && c <= seen + n {
                return r.fmt.clone();
            }
            if c == 0 && n > 0 {
                return r.fmt.clone();
            }
            seen += n;
            last = Some(&r.fmt);
        }
        last.cloned().unwrap_or_default()
    }
}

impl Doc {
    pub fn end(&self) -> Pos {
        let p = self.paras.len() - 1;
        Pos::new(p, self.paras[p].len())
    }

    pub fn clamp(&self, pos: Pos) -> Pos {
        let p = pos.p.min(self.paras.len() - 1);
        Pos::new(p, pos.c.min(self.paras[p].len()))
    }

    /// Inserts text (with line breaks starting new paragraphs) in one format.
    pub fn insert(&mut self, at: Pos, text: &str, fmt: &Fmt) -> Pos {
        let mut pos = at;
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                pos = self.split(pos);
            }
            let line = line.replace('\r', "");
            if line.is_empty() {
                continue;
            }
            let para = &mut self.paras[pos.p];
            let idx = para.split_at(pos.c);
            para.runs.insert(idx, Run { text: line.clone(), fmt: fmt.clone() });
            para.normalize();
            pos.c += line.chars().count();
        }
        pos
    }

    /// A paragraph break at `at`: the rest moves to a new paragraph of the
    /// same kind (a heading's next paragraph is normal text).
    pub fn split(&mut self, at: Pos) -> Pos {
        let para = &mut self.paras[at.p];
        let idx = para.split_at(at.c);
        let tail: Vec<Run> = para.runs.drain(idx..).collect();
        let kind = match para.kind {
            Kind::Title | Kind::H1 | Kind::H2 | Kind::H3 if tail.is_empty() => Kind::Normal,
            k => k,
        };
        let new = Para { kind, align: para.align, runs: tail };
        para.normalize();
        self.paras.insert(at.p + 1, new);
        Pos::new(at.p + 1, 0)
    }

    /// Deletes between two positions (in any order); returns where the
    /// cursor goes.
    pub fn delete(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = (a.min(b), a.max(b));
        if a == b {
            return a;
        }
        if a.p == b.p {
            let para = &mut self.paras[a.p];
            let i = para.split_at(a.c);
            let j = para.split_at(b.c);
            para.runs.drain(i..j);
            para.normalize();
            return a;
        }
        let last = &mut self.paras[b.p];
        let j = last.split_at(b.c);
        let tail: Vec<Run> = last.runs.drain(j..).collect();
        let first = &mut self.paras[a.p];
        let i = first.split_at(a.c);
        first.runs.truncate(i);
        first.runs.extend(tail);
        first.normalize();
        self.paras.drain(a.p + 1..=b.p);
        a
    }

    /// Changes the format of everything between two positions.
    pub fn format(&mut self, a: Pos, b: Pos, f: &dyn Fn(&mut Fmt)) {
        let (a, b) = (a.min(b), a.max(b));
        for p in a.p..=b.p {
            let para = &mut self.paras[p];
            let from = if p == a.p { a.c } else { 0 };
            let to = if p == b.p { b.c } else { para.len() };
            let i = para.split_at(from);
            let j = para.split_at(to);
            for r in &mut para.runs[i..j] {
                f(&mut r.fmt);
            }
            para.normalize();
        }
    }

    /// The text between two positions, paragraphs joined by line breaks.
    pub fn text_between(&self, a: Pos, b: Pos) -> String {
        let (a, b) = (a.min(b), a.max(b));
        let mut out = String::new();
        for p in a.p..=b.p {
            let t = self.paras[p].text();
            let from = if p == a.p { a.c } else { 0 };
            let to = if p == b.p { b.c } else { t.chars().count() };
            out.extend(t.chars().skip(from).take(to.saturating_sub(from)));
            if p != b.p {
                out.push('\n');
            }
        }
        out
    }

    /// A copy of what's between two positions, as a document of its own.
    pub fn slice(&self, a: Pos, b: Pos) -> Doc {
        let (a, b) = (a.min(b), a.max(b));
        let mut paras = self.paras[a.p..=b.p].to_vec();
        let n = paras.len();
        let last = &mut paras[n - 1];
        let j = last.split_at(if a.p == b.p { b.c } else { b.c });
        last.runs.truncate(j);
        let first = &mut paras[0];
        let i = first.split_at(a.c);
        first.runs.drain(..i);
        for p in &mut paras {
            p.normalize();
        }
        Doc { paras }
    }

    /// Pastes a document in at a position, keeping its formats; returns the
    /// position after it.
    pub fn insert_doc(&mut self, at: Pos, other: &Doc) -> Pos {
        let mut pos = at;
        for (i, para) in other.paras.iter().enumerate() {
            if i > 0 {
                pos = self.split(pos);
                self.paras[pos.p].kind = para.kind;
                self.paras[pos.p].align = para.align;
            } else if self.paras[pos.p].len() == 0 {
                self.paras[pos.p].kind = para.kind;
                self.paras[pos.p].align = para.align;
            }
            let target = &mut self.paras[pos.p];
            let idx = target.split_at(pos.c);
            for (k, r) in para.runs.iter().enumerate() {
                target.runs.insert(idx + k, r.clone());
            }
            pos.c += para.len();
            target.normalize();
        }
        pos
    }

    /// Word boundaries, for Ctrl+arrows and double clicks.
    pub fn word_left(&self, pos: Pos) -> Pos {
        if pos.c == 0 {
            return if pos.p > 0 { Pos::new(pos.p - 1, self.paras[pos.p - 1].len()) } else { pos };
        }
        let chars: Vec<char> = self.paras[pos.p].text().chars().collect();
        let mut c = pos.c;
        while c > 0 && !chars[c - 1].is_alphanumeric() {
            c -= 1;
        }
        while c > 0 && chars[c - 1].is_alphanumeric() {
            c -= 1;
        }
        Pos::new(pos.p, c)
    }

    pub fn word_right(&self, pos: Pos) -> Pos {
        let chars: Vec<char> = self.paras[pos.p].text().chars().collect();
        if pos.c >= chars.len() {
            return if pos.p + 1 < self.paras.len() { Pos::new(pos.p + 1, 0) } else { pos };
        }
        let mut c = pos.c;
        while c < chars.len() && chars[c].is_alphanumeric() {
            c += 1;
        }
        while c < chars.len() && !chars[c].is_alphanumeric() {
            c += 1;
        }
        Pos::new(pos.p, c)
    }

    pub fn word_at(&self, pos: Pos) -> (Pos, Pos) {
        let chars: Vec<char> = self.paras[pos.p].text().chars().collect();
        let (mut a, mut b) = (pos.c.min(chars.len()), pos.c.min(chars.len()));
        while a > 0 && chars[a - 1].is_alphanumeric() {
            a -= 1;
        }
        while b < chars.len() && chars[b].is_alphanumeric() {
            b += 1;
        }
        (Pos::new(pos.p, a), Pos::new(pos.p, b))
    }

    /// The number shown before a numbered list item (counting the items
    /// right before it).
    pub fn list_number(&self, p: usize) -> usize {
        let mut n = 1;
        let mut i = p;
        while i > 0 && self.paras[i - 1].kind == Kind::Numbered {
            n += 1;
            i -= 1;
        }
        n
    }

    /// Every place a piece of text occurs (case-insensitive).
    pub fn find(&self, needle: &str) -> Vec<(Pos, Pos)> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle: Vec<char> = needle.to_lowercase().chars().collect();
        let mut hits = Vec::new();
        for (p, para) in self.paras.iter().enumerate() {
            let hay: Vec<char> = para.text().to_lowercase().chars().collect();
            let mut c = 0;
            while c + needle.len() <= hay.len() {
                if hay[c..c + needle.len()] == needle[..] {
                    hits.push((Pos::new(p, c), Pos::new(p, c + needle.len())));
                    c += needle.len();
                } else {
                    c += 1;
                }
            }
        }
        hits
    }

    pub fn word_count(&self) -> usize {
        self.paras.iter().map(|p| p.text().split_whitespace().count()).sum()
    }
}

// ---- Markdown ------------------------------------------------------------

fn md_escape(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &ch) in chars.iter().enumerate() {
        // An underscore inside a word (snake_case) can't start italics.
        let in_word = ch == '_'
            && i > 0
            && chars[i - 1].is_alphanumeric()
            && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
        if matches!(ch, '*' | '_' | '`' | '~' | '\\') && !in_word {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

pub fn para_to_markdown(para: &Para, number: usize) -> String {
    let mut body = String::new();
    for r in &para.runs {
        if para.kind == Kind::Code {
            body.push_str(&r.text);
            continue;
        }
        let f = &r.fmt;
        // Markers hug the words: spaces at the ends stay outside them.
        let t = &r.text;
        let lead = t.len() - t.trim_start().len();
        let trail = t.len() - t.trim_end().len();
        let core = t.trim();
        if core.is_empty() {
            body.push_str(t);
            continue;
        }
        let mut s = md_escape(core);
        if f.font.as_deref() == Some("Mono") {
            s = format!("`{core}`");
        }
        if f.strike {
            s = format!("~~{s}~~");
        }
        if f.italic {
            s = format!("*{s}*");
        }
        if f.bold {
            s = format!("**{s}**");
        }
        if f.underline {
            s = format!("<u>{s}</u>");
        }
        body.push_str(&t[..lead]);
        body.push_str(&s);
        body.push_str(&t[t.len() - trail..]);
    }
    match para.kind {
        Kind::Title => format!("# {body}"),
        Kind::H1 => format!("## {body}"),
        Kind::H2 => format!("### {body}"),
        Kind::H3 => format!("#### {body}"),
        Kind::Bullet => format!("- {body}"),
        Kind::Numbered => format!("{number}. {body}"),
        Kind::Quote => format!("> {body}"),
        Kind::Code => format!("```\n{body}\n```"),
        Kind::Normal => body,
    }
}

pub fn to_markdown(doc: &Doc) -> String {
    let mut out = String::new();
    for (i, p) in doc.paras.iter().enumerate() {
        out.push_str(&para_to_markdown(p, doc.list_number(i)));
        out.push('\n');
        // A blank line between paragraphs, except inside a list.
        let next_list = doc.paras.get(i + 1).is_some_and(|n| n.kind == p.kind && p.kind.is_list());
        if !next_list {
            out.push('\n');
        }
    }
    out.trim_end().to_owned() + "\n"
}

/// Inline Markdown (**bold**, *italic*, ~~strike~~, <u>underline</u>,
/// `code`) to runs.
pub fn inline(text: &str, base: &Fmt) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut fmt = base.clone();
    let mut cur = String::new();
    let mut i = 0;
    let flush = |cur: &mut String, runs: &mut Vec<Run>, fmt: &Fmt| {
        if !cur.is_empty() {
            runs.push(Run { text: std::mem::take(cur), fmt: fmt.clone() });
        }
    };
    let at = |i: usize, s: &str| -> bool {
        let s: Vec<char> = s.chars().collect();
        chars.len() >= i + s.len() && chars[i..i + s.len()] == s[..]
    };
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            cur.push(chars[i + 1]);
            i += 2;
        } else if at(i, "**") || at(i, "__") {
            flush(&mut cur, &mut runs, &fmt);
            fmt.bold = !fmt.bold;
            i += 2;
        } else if at(i, "~~") {
            flush(&mut cur, &mut runs, &fmt);
            fmt.strike = !fmt.strike;
            i += 2;
        } else if at(i, "<u>") || at(i, "</u>") {
            flush(&mut cur, &mut runs, &fmt);
            fmt.underline = at(i, "<u>");
            i += if fmt.underline { 3 } else { 4 };
        } else if ch == '`' {
            flush(&mut cur, &mut runs, &fmt);
            let end = chars[i + 1..].iter().position(|&c| c == '`').map(|e| i + 1 + e);
            if let Some(end) = end {
                let mut f = fmt.clone();
                f.font = Some("Mono".to_owned());
                runs.push(Run { text: chars[i + 1..end].iter().collect(), fmt: f });
                i = end + 1;
            } else {
                cur.push(ch);
                i += 1;
            }
        } else if (ch == '*' || ch == '_')
            && (fmt.italic || chars.get(i + 1).is_some_and(|c| !c.is_whitespace()))
            && !(ch == '_' && i > 0 && chars[i - 1].is_alphanumeric() && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric()))
        {
            flush(&mut cur, &mut runs, &fmt);
            fmt.italic = !fmt.italic;
            i += 1;
        } else {
            cur.push(ch);
            i += 1;
        }
    }
    flush(&mut cur, &mut runs, &fmt);
    runs
}

pub fn from_markdown(md: &str) -> Doc {
    let mut paras = Vec::new();
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l.to_owned());
            }
            for l in code {
                paras.push(Para::plain(Kind::Code, &l));
            }
            continue;
        }
        let s = t.trim_start();
        if s.is_empty() {
            continue;
        }
        let (kind, rest) = if let Some(r) = s.strip_prefix("#### ") {
            (Kind::H3, r)
        } else if let Some(r) = s.strip_prefix("### ") {
            (Kind::H2, r)
        } else if let Some(r) = s.strip_prefix("## ") {
            (Kind::H1, r)
        } else if let Some(r) = s.strip_prefix("# ") {
            (Kind::Title, r)
        } else if let Some(r) = s.strip_prefix("- ").or(s.strip_prefix("* ")).or(s.strip_prefix("+ ")) {
            (Kind::Bullet, r)
        } else if let Some(r) = s.strip_prefix("> ") {
            (Kind::Quote, r)
        } else if let Some(dot) = s.find(". ").filter(|&d| d > 0 && d <= 3 && s[..d].chars().all(|c| c.is_ascii_digit())) {
            (Kind::Numbered, &s[dot + 2..])
        } else {
            (Kind::Normal, s)
        };
        let mut para = Para { kind, align: Align::Left, runs: inline(rest, &Fmt::default()) };
        para.normalize();
        paras.push(para);
    }
    if paras.is_empty() {
        paras.push(Para::default());
    }
    Doc { paras }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits() {
        let mut d = Doc::default();
        let end = d.insert(Pos::new(0, 0), "Hello world\nSecond", &Fmt::default());
        assert_eq!(end, Pos::new(1, 6));
        assert_eq!(d.paras.len(), 2);
        d.format(Pos::new(0, 6), Pos::new(0, 11), &|f| f.bold = true);
        assert_eq!(d.paras[0].runs.len(), 2);
        assert_eq!(para_to_markdown(&d.paras[0], 1), "Hello **world**");
        let at = d.delete(Pos::new(0, 5), Pos::new(1, 0));
        assert_eq!(at, Pos::new(0, 5));
        assert_eq!(d.paras[0].text(), "HelloSecond");
        d.format(Pos::new(0, 0), Pos::new(0, 5), &|f| f.bold = true);
        let s = d.slice(Pos::new(0, 1), Pos::new(0, 7));
        assert_eq!(s.paras[0].text(), "elloSe");
        assert!(s.paras[0].runs[0].fmt.bold && !s.paras[0].runs[1].fmt.bold);
    }

    #[test]
    fn markdown_round_trip() {
        let md = "# Title\n\n## Part *one*\n\nSome **bold** and `code`, a_b_c.\n\n- a\n- b\n\n1. x\n2. y\n\n> quoted\n";
        let d = from_markdown(md);
        assert_eq!(d.paras.len(), 8);
        assert_eq!(d.paras[0].kind, Kind::Title);
        assert_eq!(d.paras[2].text(), "Some bold and code, a_b_c.");
        assert_eq!(to_markdown(&d), md);
    }
}
