//! Claude's tools in Docs. Claude sees the document as numbered paragraphs
//! in Markdown and writes Markdown back: whole paragraphs replaced or
//! inserted, words found and replaced, formats and paragraph styles set.
//! Each change is one undo step.

use crate::App;
use crate::doc::{self, Align, Doc, Kind, Pos};
use atelier::bridge::Request;
use atelier::mcp::tool;
use serde_json::{Value, json};

const MARKDOWN: &str = "Markdown: \"# \" title, \"## \" heading 1, \"### \" heading 2, \"#### \" heading 3, \"- \" bullets, \"1. \" numbered, \"> \" quote, ``` code; inline **bold**, *italic*, ~~strike~~, <u>underline</u>, `code`. One paragraph per line or blank-line-separated block.";

pub fn list() -> Value {
    json!([
        tool(
            "read_document",
            "The document as numbered paragraphs, each \"[n] markdown\" (n from 0), with the user's cursor and selection. Long documents: read a part with first/last.",
            &[("first", "integer", "First paragraph (default 0).", false), ("last", "integer", "Last paragraph, included.", false)]
        ),
        tool(
            "replace_paragraphs",
            &format!("Replaces paragraphs first..last (included) with new ones written in Markdown; an empty string deletes them. {MARKDOWN}"),
            &[
                ("first", "integer", "First paragraph to replace.", true),
                ("last", "integer", "Last paragraph to replace (included).", true),
                ("markdown", "string", "The new paragraphs.", true),
            ]
        ),
        tool(
            "insert_paragraphs",
            &format!("Inserts paragraphs, written in Markdown, before paragraph `at` (the paragraph count to add at the end). {MARKDOWN}"),
            &[("at", "integer", "Where.", true), ("markdown", "string", "The new paragraphs.", true)]
        ),
        tool(
            "replace_text",
            "Finds text (ignoring case) and replaces it, keeping the formatting around it. Every occurrence, or only the first.",
            &[
                ("find", "string", "Text to find (within one paragraph).", true),
                ("replace", "string", "What to put instead.", true),
                ("all", "boolean", "Every occurrence (default true).", false),
            ]
        ),
        tool(
            "format_text",
            "Sets character formats on paragraphs first..last, or only on the occurrences of `find` in them. Give only what should change.",
            &[
                ("first", "integer", "First paragraph.", true),
                ("last", "integer", "Last paragraph (included; default first).", false),
                ("find", "string", "Only this text in them.", false),
                ("bold", "boolean", "", false),
                ("italic", "boolean", "", false),
                ("underline", "boolean", "", false),
                ("strikethrough", "boolean", "", false),
                ("color", "string", "\"#RRGGBB\", or \"\" for automatic.", false),
                ("size", "number", "Points, or 0 for the paragraph's own.", false),
                ("font", "string", "\"Sans\", \"Serif\" or \"Mono\".", false),
            ]
        ),
        tool(
            "set_paragraphs",
            "Sets the style and/or alignment of paragraphs first..last.",
            &[
                ("first", "integer", "First paragraph.", true),
                ("last", "integer", "Last paragraph (included; default first).", false),
                ("style", "string", "normal, title, heading1, heading2, heading3, bullet, numbered, quote or code.", false),
                ("align", "string", "left, center, right or justify.", false),
            ]
        ),
        tool(
            "select",
            "Selects paragraphs (or a piece of text in them) and scrolls there, to show the user something.",
            &[
                ("first", "integer", "First paragraph.", true),
                ("last", "integer", "Last paragraph (default first).", false),
                ("find", "string", "Select only this text in them.", false),
            ]
        ),
    ])
}

pub fn handle(app: &mut App, req: Request) {
    let result = run(app, &req);
    req.reply(result);
}

fn para_range(doc: &Doc, req: &Request) -> Result<(usize, usize), String> {
    let n = doc.paras.len();
    let first = req.int("first")?.max(0) as usize;
    let last = req.opt_int("last").map(|l| l.max(0) as usize).unwrap_or(first);
    if first >= n {
        return Err(format!("there are only {n} paragraphs (0..{})", n - 1));
    }
    Ok((first.min(last), last.max(first).min(n - 1)))
}

fn hits_in(doc: &Doc, find: Option<&str>, first: usize, last: usize) -> Vec<(Pos, Pos)> {
    match find {
        Some(f) => doc.find(f).into_iter().filter(|(a, _)| a.p >= first && a.p <= last).collect(),
        None => vec![(Pos::new(first, 0), Pos::new(last, doc.paras[last].len()))],
    }
}

fn run(app: &mut App, req: &Request) -> Result<String, String> {
    let ed = &mut app.ed;
    match req.tool.as_str() {
        "read_document" => {
            let n = ed.doc.paras.len();
            let first = req.opt_int("first").unwrap_or(0).max(0) as usize;
            let last = req.opt_int("last").map(|l| l as usize).unwrap_or(n - 1).min(n - 1);
            let (a, b) = ed.range();
            let mut out = format!("{n} paragraphs. Cursor: paragraph {}, character {}.", ed.cursor.p, ed.cursor.c);
            if a != b {
                out.push_str(&format!(" Selected: from paragraph {} to {}: \"{}\".", a.p, b.p, ed.doc.text_between(a, b)));
            }
            out.push_str("\n\n");
            for i in first..=last {
                let line = doc::para_to_markdown(&ed.doc.paras[i], ed.doc.list_number(i));
                let align = match ed.doc.paras[i].align {
                    Align::Center => " {center}",
                    Align::Right => " {right}",
                    Align::Justify => " {justify}",
                    Align::Left => "",
                };
                out.push_str(&format!("[{i}]{align} {line}\n"));
                if out.len() > 80_000 {
                    out.push_str(&format!("… (stopped at paragraph {i}: read on with first={})\n", i + 1));
                    break;
                }
            }
            Ok(out)
        }
        "replace_paragraphs" => {
            let (first, last) = para_range(&ed.doc, req)?;
            let md = req.str("markdown")?;
            ed.checkpoint(false, 0.0);
            let new = if md.trim().is_empty() { Vec::new() } else { doc::from_markdown(md).paras };
            let count = new.len();
            ed.doc.paras.splice(first..=last, new);
            if ed.doc.paras.is_empty() {
                ed.doc.paras.push(Default::default());
            }
            let end = (first + count).saturating_sub(1).min(ed.doc.paras.len() - 1);
            ed.select(Pos::new(first.min(ed.doc.paras.len() - 1), 0), Pos::new(end, ed.doc.paras[end].len()));
            Ok(format!("paragraphs {first}..{last} replaced by {count}"))
        }
        "insert_paragraphs" => {
            let at = (req.int("at")?.max(0) as usize).min(ed.doc.paras.len());
            let new = doc::from_markdown(req.str("markdown")?).paras;
            let count = new.len();
            ed.checkpoint(false, 0.0);
            // An empty document's one empty paragraph is replaced.
            if ed.doc.paras.len() == 1 && ed.doc.paras[0].len() == 0 {
                ed.doc.paras.clear();
            }
            let at = at.min(ed.doc.paras.len());
            ed.doc.paras.splice(at..at, new);
            let end = at + count - 1;
            ed.select(Pos::new(at, 0), Pos::new(end, ed.doc.paras[end].len()));
            Ok(format!("{count} paragraphs inserted at {at}"))
        }
        "replace_text" => {
            let find = req.str("find")?;
            let replace = req.str("replace")?;
            let all = req.args["all"].as_bool().unwrap_or(true);
            let mut hits = ed.doc.find(find);
            if hits.is_empty() {
                return Err(format!("\"{find}\" isn't in the document"));
            }
            if !all {
                hits.truncate(1);
            }
            ed.checkpoint(false, 0.0);
            for &(a, b) in hits.iter().rev() {
                let fmt = ed.doc.paras[a.p].fmt_at(a.c + 1);
                let at = ed.doc.delete(a, b);
                ed.doc.insert(at, replace, &fmt);
            }
            Ok(format!("replaced {} time(s)", hits.len()))
        }
        "format_text" => {
            let (first, last) = para_range(&ed.doc, req)?;
            let hits = hits_in(&ed.doc, req.opt_str("find"), first, last);
            if hits.is_empty() {
                return Err("that text isn't in those paragraphs".to_owned());
            }
            let a = req.args.clone();
            let color = match a["color"].as_str() {
                Some("") => Some(None),
                Some(s) => Some(Some(atelier::theme::parse_hex(s).ok_or("color should be #RRGGBB")?)),
                None => None,
            };
            ed.checkpoint(false, 0.0);
            for (x, y) in hits {
                ed.doc.format(x, y, &|f| {
                    if let Some(b) = a["bold"].as_bool() {
                        f.bold = b;
                    }
                    if let Some(b) = a["italic"].as_bool() {
                        f.italic = b;
                    }
                    if let Some(b) = a["underline"].as_bool() {
                        f.underline = b;
                    }
                    if let Some(b) = a["strikethrough"].as_bool() {
                        f.strike = b;
                    }
                    if let Some(c) = color {
                        f.color = c.map(|c| [c.r(), c.g(), c.b()]);
                    }
                    if let Some(s) = a["size"].as_f64() {
                        f.size = (s > 0.0).then_some(s as f32);
                    }
                    if let Some(font) = a["font"].as_str() {
                        f.font = (font != "Sans").then(|| font.to_owned());
                    }
                });
            }
            Ok("formatted".to_owned())
        }
        "set_paragraphs" => {
            let (first, last) = para_range(&ed.doc, req)?;
            let kind = match req.opt_str("style") {
                None => None,
                Some(s) => Some(match s.to_lowercase().replace([' ', '_', '-'], "").as_str() {
                    "normal" | "text" | "normaltext" => Kind::Normal,
                    "title" => Kind::Title,
                    "heading1" | "h1" => Kind::H1,
                    "heading2" | "h2" => Kind::H2,
                    "heading3" | "h3" => Kind::H3,
                    "bullet" | "bullets" | "bulleted" => Kind::Bullet,
                    "numbered" | "number" | "numbers" => Kind::Numbered,
                    "quote" => Kind::Quote,
                    "code" => Kind::Code,
                    other => return Err(format!("unknown style {other}")),
                }),
            };
            let align = match req.opt_str("align") {
                None => None,
                Some("left") => Some(Align::Left),
                Some("center") | Some("centre") => Some(Align::Center),
                Some("right") => Some(Align::Right),
                Some("justify") => Some(Align::Justify),
                Some(other) => return Err(format!("unknown alignment {other}")),
            };
            ed.checkpoint(false, 0.0);
            for p in &mut ed.doc.paras[first..=last] {
                if let Some(k) = kind {
                    p.kind = k;
                }
                if let Some(a) = align {
                    p.align = a;
                }
            }
            Ok("done".to_owned())
        }
        "select" => {
            let (first, last) = para_range(&ed.doc, req)?;
            let hits = hits_in(&ed.doc, req.opt_str("find"), first, last);
            let (a, b) = *hits.first().ok_or("that text isn't in those paragraphs")?;
            ed.select(a, b);
            Ok("selected".to_owned())
        }
        other => Err(format!("unknown tool {other}")),
    }
}
