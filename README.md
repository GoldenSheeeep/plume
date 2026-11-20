# plume

**Docs**: a word processor for Linux in Rust ([egui](https://github.com/emilk/egui)).
It opens and saves Word files (`.docx`), and Markdown and plain text.

- A page like Google Docs', black with white text (text shows black on
  paper and in Word).
- Title, three heading levels, bulleted and numbered lists, quotes, code.
- Bold, italic, underline, strikethrough, sans / serif / mono, sizes,
  colours; left, centre, right and justified paragraphs.
- Markdown as you type: `# ` makes a heading, `- ` a list, `1. ` a
  numbered one, `> ` a quote.
- Find and replace, zoom, word count, undo (typing groups into steps).
- Word files keep their headings, lists and formats both ways.
- **Claude** (Ctrl+J): Claude reads the document as numbered paragraphs
  in Markdown and rewrites, inserts, formats and restyles them through
  the app's own tools, live; every change undoes with Ctrl+Z. Needs
  [Claude Code](https://claude.com/claude-code) (`claude`) installed and
  logged in.

The look, title bar and Claude panel come from
[atelier](https://github.com/GoldenSheeeep/atelier), shared with
[tableur](https://github.com/GoldenSheeeep/tableur) (Sheets) and
[diapo](https://github.com/GoldenSheeeep/diapo) (Slides).

## Building

```sh
cargo build --release   # target/release/docs
```

At run time it needs `libGL`, `wayland` and `libxkbcommon`, and
`fc-match` (fontconfig) for fonts.

## Not yet

Tables (they open as lines of text) and pictures (left out), PDF export,
page breaks (it's one continuous page).

## License

MIT
