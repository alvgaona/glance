//! Fitting text into columns, and small styled pieces.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// Cuts a line to `width` columns, ending in "…" when it doesn't fit.
pub fn truncate(line: Line<'static>, width: u16) -> Line<'static> {
    let width = width as usize;
    if line.width() <= width {
        return line;
    }
    let mut out = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let w = span.width();
        if used + w < width {
            used += w;
            out.push(span);
            continue;
        }
        let keep: String = span.content.chars().take(width.saturating_sub(used + 1)).collect();
        out.push(Span::styled(format!("{keep}…"), span.style));
        break;
    }
    Line::from(out)
}

/// A commit message body wrapped to `width`: the lines of each paragraph are joined
/// first (messages are broken at ~72 columns already). Blank lines stay, and a list
/// item (`- `, `* `, `1. `) starts a new paragraph.
pub fn reflow(text: &str, width: usize) -> Vec<String> {
    let item = |l: &str| {
        let l = l.trim_start();
        l.starts_with("- ") || l.starts_with("* ") || l.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok())
    };
    let mut paragraphs: Vec<String> = Vec::new();
    let mut open = false;
    for l in text.lines() {
        if l.trim().is_empty() {
            paragraphs.push(String::new());
            open = false;
        } else if open && !item(l) {
            let p = paragraphs.last_mut().unwrap();
            p.push(' ');
            p.push_str(l.trim());
        } else {
            paragraphs.push(l.trim_end().to_string());
            open = true;
        }
    }
    paragraphs.iter().flat_map(|p| wrap(p, width)).collect()
}

/// Word-wraps `text` to `width` columns. Words longer than a line are split.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let used = line.chars().count();
            let need = word.chars().count() + usize::from(used > 0);
            if used + need <= width {
                if used > 0 {
                    line.push(' ');
                }
                line.push_str(&word);
                break;
            }
            if used > 0 {
                out.push(std::mem::take(&mut line));
                continue;
            }
            // A single word wider than the line.
            let head: String = word.chars().take(width).collect();
            word = word.chars().skip(width).collect();
            out.push(head);
            if word.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
}

/// A coloured tag like ` fixup `.
pub fn pill(text: &str, bg: Color, fg: Color) -> Span<'static> {
    Span::styled(format!(" {text} "), Style::new().bg(bg).fg(fg))
}

/// Drops a line's first `cols` columns, to scroll it sideways.
pub fn skip(line: Line<'static>, cols: u16) -> Line<'static> {
    use unicode_width::UnicodeWidthChar;
    let style = line.style;
    let mut left = cols as usize;
    let mut out = Vec::new();
    for span in line.spans {
        let w = span.width();
        if left >= w {
            left -= w;
            continue;
        }
        // A wide character cut in half goes whole.
        let kept: String = span
            .content
            .chars()
            .skip_while(|c| {
                let gone = left > 0;
                left = left.saturating_sub(c.width().unwrap_or(0));
                gone
            })
            .collect();
        out.push(Span::styled(kept, span.style));
    }
    Line::from(out).style(style)
}

#[cfg(test)]
mod tests {
    use super::{reflow, skip, wrap};
    use ratatui::text::{Line, Span};

    #[test]
    fn skips_columns_across_spans() {
        let line = Line::from(vec![Span::raw("  12 "), Span::raw("hello"), Span::raw(" world")]);
        assert_eq!(skip(line.clone(), 7).to_string(), "llo world");
        assert_eq!(skip(line.clone(), 0).to_string(), "  12 hello world");
        assert_eq!(skip(line, 99).to_string(), "");
        assert_eq!(skip(Line::raw("日本語"), 1).to_string(), "本語");
    }

    #[test]
    fn reflows_paragraphs_and_keeps_lists() {
        let body = "The sidebar is now\na shared piece.\n\n- one\n- two\n  more";
        assert_eq!(reflow(body, 40), ["The sidebar is now a shared piece.", "", "- one", "- two more"]);
    }

    #[test]
    fn wraps_words() {
        assert_eq!(wrap("ci: build the shared packages once", 12), ["ci: build", "the shared", "packages", "once"]);
        assert_eq!(wrap("short", 20), ["short"]);
        assert_eq!(wrap("", 10), [""]);
        // A word longer than the line is split.
        assert_eq!(wrap("abcdefghij xy", 4), ["abcd", "efgh", "ij", "xy"]);
    }
}
