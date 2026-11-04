//! pulldown-cmark events → a small block tree tailored for print layout.

use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use std::iter::Peekable;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
}

impl Span {
    fn same_style(&self, o: &Span) -> bool {
        (self.bold, self.italic, self.code, self.strike, self.link)
            == (o.bold, o.italic, o.code, o.strike, o.link)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(Span),
    Image { url: String, alt: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading(u8, Vec<Span>),
    Code(String),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Rule,
    Table {
        aligns: Vec<Alignment>,
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Image {
        url: String,
        alt: String,
    },
}

pub fn parse(markdown: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION;
    let mut p = BlockParser {
        events: Parser::new_ext(markdown, opts).peekable(),
        tasks: Vec::new(),
    };
    p.blocks()
}

struct BlockParser<'a, I: Iterator<Item = Event<'a>>> {
    events: Peekable<I>,
    tasks: Vec<Option<bool>>,
}

fn is_inline(ev: &Event<'_>) -> bool {
    match ev {
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineMath(_)
        | Event::InlineHtml(_)
        | Event::FootnoteReference(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::TaskListMarker(_) => true,
        Event::Start(t) => matches!(
            t,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::Link { .. }
                | Tag::Image { .. }
        ),
        Event::End(t) => matches!(
            t,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::Link
                | TagEnd::Image
        ),
        _ => false,
    }
}

impl<'a, I: Iterator<Item = Event<'a>>> BlockParser<'a, I> {
    /// Parse blocks until the end of the enclosing container (whose `End` is consumed).
    fn blocks(&mut self) -> Vec<Block> {
        let mut out = Vec::new();
        while let Some(ev) = self.events.peek() {
            if is_inline(ev) {
                // Tight list items put inline content directly inside the item.
                let inl = self.inlines();
                push_inlines(&mut out, inl, Block::Paragraph);
                continue;
            }
            let ev = self.events.next().unwrap();
            match ev {
                Event::End(_) => break,
                Event::Rule => out.push(Block::Rule),
                Event::Html(_) | Event::DisplayMath(_) => {}
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {
                        let inl = self.inlines();
                        push_inlines(&mut out, inl, Block::Paragraph);
                    }
                    Tag::Heading { level, .. } => {
                        let spans = spans_only(self.inlines());
                        out.push(Block::Heading(level as u8, spans));
                    }
                    Tag::BlockQuote(_) => out.push(Block::Quote(self.blocks())),
                    Tag::CodeBlock(_) => {
                        let mut text = String::new();
                        for ev in self.events.by_ref() {
                            match ev {
                                Event::Text(t) => text.push_str(&t),
                                Event::End(_) => break,
                                _ => {}
                            }
                        }
                        out.push(Block::Code(text.trim_end_matches('\n').to_string()));
                    }
                    Tag::List(start) => {
                        let mut items = Vec::new();
                        while let Some(ev) = self.events.next() {
                            match ev {
                                Event::Start(Tag::Item) => {
                                    self.tasks.push(None);
                                    let blocks = self.blocks();
                                    items.push(Item {
                                        checked: self.tasks.pop().flatten(),
                                        blocks,
                                    });
                                }
                                Event::End(_) => break,
                                _ => {}
                            }
                        }
                        out.push(Block::List { start, items });
                    }
                    Tag::Table(aligns) => out.push(self.table(aligns)),
                    Tag::HtmlBlock | Tag::MetadataBlock(_) => self.skip_container(),
                    // Footnote bodies, definition lists etc.: render their content plainly.
                    _ => out.extend(self.blocks()),
                },
                _ => {}
            }
        }
        out
    }

    /// Parse inline content. Consumes a closing Paragraph/Heading/TableCell end tag;
    /// stops (without consuming) at any other block-level event.
    fn inlines(&mut self) -> Vec<Inline> {
        let mut out = Vec::new();
        let (mut bold, mut italic, mut strike, mut link) = (0u32, 0u32, 0u32, 0u32);
        let push =
            |out: &mut Vec<Inline>, text: &str, code: bool, b: u32, i: u32, s: u32, l: u32| {
                if text.is_empty() {
                    return;
                }
                let span = Span {
                    text: text.to_string(),
                    bold: b > 0,
                    italic: i > 0,
                    code,
                    strike: s > 0,
                    link: l > 0,
                };
                if let Some(Inline::Text(last)) = out.last_mut()
                    && last.same_style(&span)
                {
                    last.text.push_str(&span.text);
                    return;
                }
                out.push(Inline::Text(span));
            };

        loop {
            match self.events.peek() {
                None => break,
                Some(Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell)) => {
                    self.events.next();
                    break;
                }
                Some(ev) if !is_inline(ev) => break,
                _ => {}
            }
            match self.events.next().unwrap() {
                Event::Text(t) => push(&mut out, &t, false, bold, italic, strike, link),
                Event::Code(t) => push(&mut out, &t, true, bold, italic, strike, link),
                Event::InlineMath(t) => push(&mut out, &t, true, bold, italic, strike, link),
                Event::InlineHtml(_) => {}
                Event::FootnoteReference(r) => push(
                    &mut out,
                    &format!("[{r}]"),
                    false,
                    bold,
                    italic,
                    strike,
                    link,
                ),
                Event::SoftBreak => push(&mut out, " ", false, bold, italic, strike, link),
                Event::HardBreak => push(&mut out, "\n", false, bold, italic, strike, link),
                Event::TaskListMarker(done) => {
                    if let Some(slot) = self.tasks.last_mut() {
                        *slot = Some(done);
                    }
                }
                Event::Start(Tag::Strong) => bold += 1,
                Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
                Event::Start(Tag::Emphasis) => italic += 1,
                Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
                Event::Start(Tag::Strikethrough) => strike += 1,
                Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
                Event::Start(Tag::Link { .. }) => link += 1,
                Event::End(TagEnd::Link) => link = link.saturating_sub(1),
                Event::Start(Tag::Image { dest_url, .. }) => {
                    let mut alt = String::new();
                    for ev in self.events.by_ref() {
                        match ev {
                            Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                            Event::End(TagEnd::Image) => break,
                            _ => {}
                        }
                    }
                    out.push(Inline::Image {
                        url: dest_url.to_string(),
                        alt,
                    });
                }
                _ => {}
            }
        }
        out
    }

    fn table(&mut self, aligns: Vec<Alignment>) -> Block {
        let (mut head, mut rows) = (Vec::new(), Vec::new());
        while let Some(ev) = self.events.next() {
            match ev {
                Event::Start(Tag::TableHead) => head = self.cells(),
                Event::Start(Tag::TableRow) => rows.push(self.cells()),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        Block::Table { aligns, head, rows }
    }

    fn cells(&mut self) -> Vec<Vec<Span>> {
        let mut cells = Vec::new();
        while let Some(ev) = self.events.next() {
            match ev {
                Event::Start(Tag::TableCell) => cells.push(spans_only(self.inlines())),
                Event::End(_) => break,
                _ => {}
            }
        }
        cells
    }

    fn skip_container(&mut self) {
        let mut depth = 1;
        for ev in self.events.by_ref() {
            match ev {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
}

/// Split inline content into text blocks and standalone image blocks.
fn push_inlines(out: &mut Vec<Block>, inl: Vec<Inline>, make: fn(Vec<Span>) -> Block) {
    let mut spans = Vec::new();
    for i in inl {
        match i {
            Inline::Text(s) => spans.push(s),
            Inline::Image { url, alt } => {
                flush(out, &mut spans, make);
                out.push(Block::Image { url, alt });
            }
        }
    }
    flush(out, &mut spans, make);
}

fn flush(out: &mut Vec<Block>, spans: &mut Vec<Span>, make: fn(Vec<Span>) -> Block) {
    if spans.iter().any(|s| !s.text.trim().is_empty()) {
        // Trim the whitespace left behind where an image was split out.
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        out.push(make(std::mem::take(spans)));
    }
    spans.clear();
}

fn spans_only(inl: Vec<Inline>) -> Vec<Span> {
    inl.into_iter()
        .map(|i| match i {
            Inline::Text(s) => s,
            Inline::Image { alt, .. } => Span {
                text: alt,
                italic: true,
                ..Default::default()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(blocks: &[Block]) -> String {
        format!("{blocks:?}")
    }

    #[test]
    fn headings_paragraphs_and_styles() {
        let b = parse("# Title\n\nSome **bold** and *it* and `code` ~~gone~~.");
        assert!(matches!(&b[0], Block::Heading(1, s) if s[0].text == "Title"));
        let Block::Paragraph(spans) = &b[1] else {
            panic!("{}", text(&b))
        };
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        assert!(spans.iter().any(|s| s.italic && s.text == "it"));
        assert!(spans.iter().any(|s| s.code && s.text == "code"));
        assert!(spans.iter().any(|s| s.strike && s.text == "gone"));
    }

    #[test]
    fn tight_and_task_lists() {
        let b = parse("- [x] done\n- [ ] todo\n  - nested\n");
        let Block::List { items, .. } = &b[0] else {
            panic!("{}", text(&b))
        };
        assert_eq!(items[0].checked, Some(true));
        assert_eq!(items[1].checked, Some(false));
        assert!(matches!(items[1].blocks.last(), Some(Block::List { .. })));
    }

    #[test]
    fn images_become_blocks() {
        let b = parse("before ![logo](a.png) after");
        assert!(
            matches!(&b[..], [Block::Paragraph(_), Block::Image { url, alt }, Block::Paragraph(_)] if url == "a.png" && alt == "logo")
        );
    }

    #[test]
    fn tables() {
        let b = parse("| a | b |\n|:--|--:|\n| 1 | 2 |\n");
        let Block::Table { aligns, head, rows } = &b[0] else {
            panic!("{}", text(&b))
        };
        assert_eq!(aligns, &[Alignment::Left, Alignment::Right]);
        assert_eq!(head.len(), 2);
        assert_eq!(rows[0][1][0].text, "2");
    }

    #[test]
    fn quotes_code_and_rules() {
        let b = parse("> quoted\n\n```\nfn x() {}\n```\n\n---\n");
        assert!(matches!(&b[0], Block::Quote(inner) if matches!(inner[0], Block::Paragraph(_))));
        assert_eq!(b[1], Block::Code("fn x() {}".into()));
        assert_eq!(b[2], Block::Rule);
    }
}
