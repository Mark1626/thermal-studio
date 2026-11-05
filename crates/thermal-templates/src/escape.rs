//! Inserting filled-in values into Markdown so they print exactly as typed.
//!
//! Rendering the template body replaces every `{{ … }}` with a placeholder token made of
//! private-use characters, which Markdown treats as plain text. The body is then parsed
//! once to find out where each token landed, and each one is replaced with its value,
//! escaped for that spot:
//!
//! - in text, Markdown punctuation is backslash-escaped, so `*` and `#` print as typed
//!   and line breaks stay line breaks;
//! - in headings and table cells, line breaks become spaces (they can't hold more than
//!   one line);
//! - in code and ```` ```qr ```` blocks, the value goes in untouched, so a QR code holds
//!   exactly the text it was given.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Starts a placeholder token: `OPEN`, the value's index in decimal, `CLOSE`.
pub(crate) const OPEN: char = '\u{E000}';
pub(crate) const CLOSE: char = '\u{E001}';

pub(crate) fn token(index: usize) -> String {
    format!("{OPEN}{index}{CLOSE}")
}

/// Where a token sits in the Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spot {
    /// Paragraphs, list items, quotes, link text.
    Text,
    /// Headings and table cells: one line only.
    Line,
    /// Inline `code`: verbatim, on one line.
    CodeSpan,
    /// Fenced or indented code blocks (including QR codes), link targets and anything
    /// else that isn't text: verbatim.
    Verbatim,
}

/// Replace every token in `markdown` with its value from `values`, escaped for where it
/// sits.
pub(crate) fn substitute(markdown: &str, values: &[String]) -> String {
    let spots = spots(markdown);
    let mut spots = spots.iter().peekable();
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(open) = rest.find(OPEN) {
        let at = markdown.len() - rest.len() + open;
        push_literal(&mut out, &rest[..open]);
        let after = &rest[open + OPEN.len_utf8()..];
        let Some((index, tail)) = after
            .split_once(CLOSE)
            .and_then(|(n, tail)| Some((n.parse::<usize>().ok()?, tail)))
            .filter(|(i, _)| *i < values.len())
        else {
            // Not one of ours: drop the stray marker.
            rest = after;
            continue;
        };
        while spots.next_if(|(r, _)| r.end <= at).is_some() {}
        let spot = match spots.peek() {
            Some((r, spot)) if r.contains(&at) => *spot,
            _ => Spot::Verbatim,
        };
        escape_into(&mut out, &values[index], spot);
        rest = tail;
    }
    push_literal(&mut out, rest);
    out
}

/// Template text between tokens, minus any stray end-of-token markers.
fn push_literal(out: &mut String, text: &str) {
    out.extend(text.chars().filter(|&c| c != CLOSE));
}

/// The same extensions `thermal-markdown` parses with, so both agree on the structure.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
}

/// Byte ranges of text, code spans and code blocks, in order and not overlapping.
fn spots(markdown: &str) -> Vec<(Range<usize>, Spot)> {
    let mut out = Vec::new();
    let mut one_line = 0u32;
    let mut in_code_block = false;
    for (ev, range) in Parser::new_ext(markdown, options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                out.push((range, Spot::Verbatim));
            }
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            _ if in_code_block => {}
            Event::Start(Tag::Heading { .. } | Tag::TableCell) => one_line += 1,
            Event::End(TagEnd::Heading(_) | TagEnd::TableCell) => one_line -= 1,
            Event::Text(_) if one_line > 0 => out.push((range, Spot::Line)),
            Event::Text(_) => out.push((range, Spot::Text)),
            Event::Code(_) => out.push((range, Spot::CodeSpan)),
            _ => {}
        }
    }
    out
}

fn escape_into(out: &mut String, value: &str, spot: Spot) {
    let value = value.replace("\r\n", "\n");
    match spot {
        Spot::Verbatim => out.push_str(&value),
        Spot::CodeSpan => out.push_str(&value.replace('\n', " ")),
        Spot::Text | Spot::Line => {
            // Surrounding blank space is never meaningful on paper, and leading spaces
            // could turn the line into a code block.
            for (i, line) in value.trim().lines().map(str::trim).enumerate() {
                if i > 0 {
                    // A backslash before the newline is a hard line break.
                    out.push_str(if spot == Spot::Text { "\\\n" } else { " " });
                }
                for c in line.chars() {
                    // Quotes too: curling them would misprint a password.
                    if c.is_ascii_punctuation() {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(markdown: &str, values: &[&str]) -> String {
        let values: Vec<String> = values.iter().map(|v| v.to_string()).collect();
        let mut md = markdown.to_string();
        for i in 0..values.len() {
            md = md.replace(&format!("${i}"), &token(i));
        }
        substitute(&md, &values)
    }

    /// Plain text of the rendered Markdown, the way the printer would see it.
    fn text(markdown: &str) -> String {
        let mut out = String::new();
        for ev in Parser::new_ext(markdown, options()) {
            match ev {
                Event::Text(t) | Event::Code(t) => out.push_str(&t),
                Event::SoftBreak => out.push(' '),
                Event::HardBreak => out.push('|'),
                Event::Start(t) => out.push_str(&format!("<{t:?}>").replace(['(', ')'], "")),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn markdown_in_values_prints_literally() {
        let v = "*not italic* # [no link](x) <b> 1. a_b \\ `tick` | ~~x~~ & AT&amp;T";
        let md = fill("Hello $0!", &[v]);
        assert_eq!(text(&md), format!("<Paragraph>Hello {v}!"));
    }

    #[test]
    fn lines_that_look_like_blocks_stay_text() {
        for v in [
            "# Heading",
            "- item",
            "1. item",
            "> quote",
            "---",
            "    code",
            "===",
            "```",
        ] {
            let md = fill("$0", &[v]);
            assert_eq!(
                text(&md),
                format!("<Paragraph>{}", v.trim()),
                "{v:?} → {md:?}"
            );
        }
    }

    #[test]
    fn line_breaks_in_text_are_kept() {
        let md = fill("- $0\n- next", &["first line\nsecond * line\r\n\n"]);
        assert_eq!(
            text(&md),
            "<ListNone><Item>first line|second * line<Item>next"
        );
        let md = fill("$0", &["a\n\nb"]);
        assert_eq!(text(&md), "<Paragraph>a||b");
    }

    #[test]
    fn headings_and_cells_stay_on_one_line() {
        let md = fill(
            "# $0\n\n| a | b |\n|---|---|\n| $1 | x |\n",
            &["two\nlines", "p|q\nr"],
        );
        let t = text(&md);
        assert!(t.contains("two lines"), "{t}");
        assert!(t.contains("<TableCell>p|q r<TableCell>x"), "{t}");
    }

    #[test]
    fn code_and_qr_blocks_get_values_verbatim() {
        let md = fill(
            "```qr\nWIFI:S:$0;P:$1;;\n```\n\nInline `$1` code.",
            &["My_Net\\;", "p*ss\nword"],
        );
        assert!(
            md.starts_with("```qr\nWIFI:S:My_Net\\;;P:p*ss\nword;;\n```"),
            "{md}"
        );
        assert!(md.contains("`p*ss word`"), "{md}");
    }

    #[test]
    fn quotes_stay_straight() {
        let md = fill("Template's \"$0\"", &["Don't \"panic\""]);
        assert_eq!(text(&md), "<Paragraph>Template’s “Don't \"panic\"”");
    }

    #[test]
    fn stray_markers_are_dropped() {
        let md = substitute(&format!("a{OPEN}b{OPEN}99{CLOSE}c"), &[]);
        assert_eq!(md, "ab99c");
    }
}
