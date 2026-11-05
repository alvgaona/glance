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

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_words() {
        assert_eq!(wrap("ci: build the shared packages once", 12), ["ci: build", "the shared", "packages", "once"]);
        assert_eq!(wrap("short", 20), ["short"]);
        assert_eq!(wrap("", 10), [""]);
        // A word longer than the line is split.
        assert_eq!(wrap("abcdefghij xy", 4), ["abcd", "efgh", "ij", "xy"]);
    }
}
