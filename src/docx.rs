//! Word files. Saving writes our paragraphs with Word's own styles
//! (Title, Heading 1..., lists as real lists), so they open as such in
//! Word and Google Docs. Opening reads text, headings, lists, alignment and
//! the usual character formats; tables come in as lines of text with their
//! cells separated by tabs, and pictures are left out.

use crate::doc::{Align, Doc, Fmt, Kind, Para, Run};
use docx_rs::{
    AbstractNumbering, AlignmentType, Docx, IndentLevel, Level, LevelJc, LevelText, NumberFormat, Numbering,
    NumberingId, Paragraph, RunFonts, SpecialIndentType, Start, Style, StyleType,
};
use serde_json::Value;

const BULLETS: usize = 101;
const NUMBERS: usize = 102;

fn font_name(f: &str) -> &'static str {
    match f {
        "Serif" => "Liberation Serif",
        "Mono" => "Liberation Mono",
        _ => "Arial",
    }
}

pub fn save(doc: &Doc, path: &std::path::Path) -> Result<(), String> {
    let mut docx = Docx::new().default_fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial")).default_size(22);
    for (id, name, size, bold) in [
        ("Title", "Title", 52, false),
        ("Heading1", "heading 1", 40, true),
        ("Heading2", "heading 2", 32, true),
        ("Heading3", "heading 3", 28, true),
        ("Quote", "Quote", 22, false),
        ("Code", "Code", 20, false),
    ] {
        let mut s = Style::new(id, StyleType::Paragraph).name(name).based_on("Normal").next("Normal").size(size);
        if bold {
            s = s.bold();
        }
        if id == "Quote" {
            s = s.italic().color("595959");
        }
        docx = docx.add_style(s);
    }
    let level = |format: &str, text: &str| {
        Level::new(0, Start::new(1), NumberFormat::new(format), LevelText::new(text), LevelJc::new("left"))
            .indent(Some(720), Some(SpecialIndentType::Hanging(360)), None, None)
    };
    docx = docx
        .add_abstract_numbering(AbstractNumbering::new(BULLETS).add_level(level("bullet", "•")))
        .add_numbering(Numbering::new(BULLETS, BULLETS))
        .add_abstract_numbering(AbstractNumbering::new(NUMBERS).add_level(level("decimal", "%1.")))
        .add_numbering(Numbering::new(NUMBERS, NUMBERS));

    for para in &doc.paras {
        let mut p = Paragraph::new();
        p = match para.kind {
            Kind::Title => p.style("Title"),
            Kind::H1 => p.style("Heading1"),
            Kind::H2 => p.style("Heading2"),
            Kind::H3 => p.style("Heading3"),
            Kind::Quote => p.style("Quote"),
            Kind::Code => p.style("Code"),
            Kind::Bullet => p.numbering(NumberingId::new(BULLETS), IndentLevel::new(0)),
            Kind::Numbered => p.numbering(NumberingId::new(NUMBERS), IndentLevel::new(0)),
            Kind::Normal => p,
        };
        p = match para.align {
            Align::Center => p.align(AlignmentType::Center),
            Align::Right => p.align(AlignmentType::Right),
            Align::Justify => p.align(AlignmentType::Both),
            Align::Left => p,
        };
        for r in &para.runs {
            let f = &r.fmt;
            let mut run = docx_rs::Run::new();
            for (i, part) in r.text.split('\t').enumerate() {
                if i > 0 {
                    run = run.add_tab();
                }
                if !part.is_empty() {
                    run = run.add_text(part);
                }
            }
            if f.bold {
                run = run.bold();
            }
            if f.italic {
                run = run.italic();
            }
            if f.underline {
                run = run.underline("single");
            }
            if f.strike {
                run = run.strike();
            }
            if let Some([r, g, b]) = f.color {
                run = run.color(format!("{r:02X}{g:02X}{b:02X}"));
            }
            if let Some(size) = f.size {
                run = run.size((size * 2.0).round() as usize);
            }
            let font = f.font.as_deref().or((para.kind == Kind::Code).then_some("Mono"));
            if let Some(font) = font {
                let name = font_name(font);
                run = run.fonts(RunFonts::new().ascii(name).hi_ansi(name));
            }
            p = p.add_run(run);
        }
        docx = docx.add_paragraph(p);
    }
    let tmp = path.with_extension("docx.saving");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    docx.build().pack(file).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ---- Opening ---------------------------------------------------------------

pub fn load(bytes: &[u8]) -> Result<Doc, String> {
    let docx = docx_rs::read_docx(bytes).map_err(|e| e.to_string())?;
    let styles = serde_json::to_value(&docx.styles).unwrap_or(Value::Null);
    let numberings = serde_json::to_value(&docx.numberings).unwrap_or(Value::Null);
    let doc = serde_json::to_value(&docx.document).map_err(|e| e.to_string())?;
    let mut paras = Vec::new();
    for child in doc["children"].as_array().into_iter().flatten() {
        match child["type"].as_str() {
            Some("paragraph") => paras.push(paragraph(&child["data"], &styles, &numberings)),
            Some("table") => table(&child["data"], &styles, &numberings, &mut paras),
            _ => {}
        }
    }
    if paras.is_empty() {
        paras.push(Para::default());
    }
    Ok(Doc { paras })
}

fn table(t: &Value, styles: &Value, nums: &Value, out: &mut Vec<Para>) {
    for row in t["rows"].as_array().into_iter().flatten() {
        let mut cells = Vec::new();
        for cell in row["data"]["cells"].as_array().into_iter().flatten() {
            let text: Vec<String> = cell["data"]["children"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["type"] == "paragraph")
                .map(|c| paragraph(&c["data"], styles, nums).text())
                .collect();
            cells.push(text.join(" "));
        }
        out.push(Para::plain(Kind::Normal, &cells.join("\t")));
    }
}

/// A paragraph style's kind, by its id or name (Word names them
/// "Heading1", "heading 1", "Title"...).
fn style_kind(id: &str, styles: &Value) -> Kind {
    let name = styles["styles"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["styleId"] == id)
        .and_then(|s| s["name"].as_str())
        .unwrap_or(id)
        .to_lowercase()
        .replace(' ', "");
    let id = id.to_lowercase();
    let is = |s: &str| name == s || id == s;
    if is("title") {
        Kind::Title
    } else if is("heading1") {
        Kind::H1
    } else if is("heading2") {
        Kind::H2
    } else if is("heading3") || is("heading4") || is("heading5") || is("heading6") {
        Kind::H3
    } else if name.contains("quote") {
        Kind::Quote
    } else if name.contains("code") || name.contains("source") {
        Kind::Code
    } else if name.contains("listbullet") {
        Kind::Bullet
    } else if name.contains("listnumber") {
        Kind::Numbered
    } else {
        Kind::Normal
    }
}

/// Whether a list numbering is bullets (rather than numbers).
fn is_bullet(num_id: i64, nums: &Value) -> bool {
    let abs = nums["numberings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|n| n["id"].as_i64() == Some(num_id))
        .and_then(|n| n["abstractNumId"].as_i64());
    let format = nums["abstractNums"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"].as_i64() == abs)
        .and_then(|a| a["levels"].get(0))
        .and_then(|l| l["format"].as_str())
        .unwrap_or("bullet");
    format == "bullet" || format == "none"
}

fn paragraph(p: &Value, styles: &Value, nums: &Value) -> Para {
    let prop = &p["property"];
    let mut kind = prop["style"].as_str().map(|s| style_kind(s, styles)).unwrap_or_default();
    if let Some(id) = prop["numberingProperty"]["id"].as_i64() {
        if id > 0 && !matches!(kind, Kind::Title | Kind::H1 | Kind::H2 | Kind::H3) {
            kind = if is_bullet(id, nums) { Kind::Bullet } else { Kind::Numbered };
        }
    }
    let align = match prop["alignment"].as_str().unwrap_or_default() {
        "center" => Align::Center,
        "right" | "end" => Align::Right,
        "both" | "justified" | "distribute" => Align::Justify,
        _ => Align::Left,
    };
    let mut runs = Vec::new();
    collect_runs(&p["children"], &mut runs);
    let mut para = Para { kind, align, runs };
    para.normalize();
    para
}

fn collect_runs(children: &Value, out: &mut Vec<Run>) {
    for c in children.as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("run") => out.push(run(&c["data"])),
            // Links, tracked insertions and the like hold runs of their own.
            Some("hyperlink") | Some("insert") | Some("structuredDataTag") => collect_runs(&c["data"]["children"], out),
            _ => {}
        }
    }
}

fn on(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Object(o) => o.get("val").map(on).unwrap_or(true),
        Value::String(s) => s != "false" && s != "0" && s != "none",
        _ => true,
    }
}

fn run(r: &Value) -> Run {
    let mut text = String::new();
    for c in r["children"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => text.push_str(c["data"]["text"].as_str().unwrap_or_default()),
            Some("tab") => text.push('\t'),
            Some("break") => text.push(' '),
            _ => {}
        }
    }
    let rp = &r["runProperty"];
    let color = rp["color"]
        .as_str()
        .or(rp["color"]["val"].as_str())
        .and_then(atelier::theme::parse_hex)
        .filter(|c| c.r() != 0 || c.g() != 0 || c.b() != 0)
        .map(|c| [c.r(), c.g(), c.b()]);
    let size = rp["sz"].as_f64().or(rp["sz"]["val"].as_f64()).map(|half| (half / 2.0) as f32);
    let face = rp["fonts"]["ascii"].as_str().unwrap_or_default().to_lowercase();
    let font = if face.contains("mono") || face.contains("courier") || face.contains("consol") {
        Some("Mono".to_owned())
    } else if face.contains("times") || face.contains("serif") && !face.contains("sans") || face.contains("georgia") || face.contains("garamond") || face.contains("cambria") {
        Some("Serif".to_owned())
    } else {
        None
    };
    let underline = match &rp["underline"] {
        Value::String(s) => s != "none",
        v => on(v),
    };
    Run {
        text,
        fmt: Fmt {
            bold: on(&rp["bold"]),
            italic: on(&rp["italic"]),
            underline,
            strike: on(&rp["strike"]),
            color,
            size: size.filter(|s| (*s - 11.0).abs() > 0.1),
            font,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::from_markdown;

    #[test]
    fn round_trip() {
        let md = "# A title\n\n## Heading\n\nSome **bold**, *italic*, ~~gone~~ and <u>under</u> text.\n\n- one\n- two\n\n1. first\n2. second\n\n> a quote\n";
        let doc = from_markdown(md);
        let dir = std::env::temp_dir().join("docs-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.docx");
        save(&doc, &path).unwrap();
        let back = load(&std::fs::read(&path).unwrap()).unwrap();
        if std::env::var("DUMP").is_ok() {
            let docx = docx_rs::read_docx(&std::fs::read(&path).unwrap()).unwrap();
            println!("{}", serde_json::to_string(&docx.numberings).unwrap());
        }
        assert_eq!(crate::doc::to_markdown(&back), md);
    }
}
