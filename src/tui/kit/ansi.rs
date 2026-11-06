//! Text with terminal colour codes (delta's output) as styled lines. Only colours and
//! bold/italic/… are kept; any other escape is dropped, since the screen does the layout.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub fn lines(text: &str) -> Vec<Line<'static>> {
    // A style can carry over a line break.
    let mut style = Style::new();
    text.lines().map(|l| line(l, &mut style)).collect()
}

fn line(text: &str, style: &mut Style) -> Line<'static> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            buf.push(c);
            continue;
        }
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut buf), *style));
        }
        match chars.next() {
            // CSI: parameters, then one final byte; only `m` (colours) is used.
            Some('[') => {
                let mut params = String::new();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        if n == 'm' {
                            sgr(style, &params);
                        }
                        break;
                    }
                    params.push(n);
                }
            }
            // OSC (a hyperlink, a title): up to BEL or ESC \.
            Some(']') => {
                while let Some(n) = chars.next() {
                    if n == '\x07' {
                        break;
                    }
                    if n == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, *style));
    }
    Line::from(spans)
}

/// Applies one `ESC[…m`.
fn sgr(style: &mut Style, params: &str) {
    let mut p = params.split(';').map(|n| n.parse::<u8>().unwrap_or(0));
    while let Some(n) = p.next() {
        match n {
            0 => *style = Style::new(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            3 => *style = style.add_modifier(Modifier::ITALIC),
            4 => *style = style.add_modifier(Modifier::UNDERLINED),
            7 => *style = style.add_modifier(Modifier::REVERSED),
            9 => *style = style.add_modifier(Modifier::CROSSED_OUT),
            22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *style = style.remove_modifier(Modifier::ITALIC),
            24 => *style = style.remove_modifier(Modifier::UNDERLINED),
            27 => *style = style.remove_modifier(Modifier::REVERSED),
            29 => *style = style.remove_modifier(Modifier::CROSSED_OUT),
            30..=37 => style.fg = Some(Color::Indexed(n - 30)),
            90..=97 => style.fg = Some(Color::Indexed(n - 90 + 8)),
            40..=47 => style.bg = Some(Color::Indexed(n - 40)),
            100..=107 => style.bg = Some(Color::Indexed(n - 100 + 8)),
            // The default colour: the panel's own shows through.
            39 => style.fg = None,
            49 => style.bg = None,
            38 => style.fg = extended(&mut p),
            48 => style.bg = extended(&mut p),
            _ => {}
        }
    }
}

/// The colour after a 38 or 48: `5;n` (256 colours) or `2;r;g;b`.
fn extended(p: &mut impl Iterator<Item = u8>) -> Option<Color> {
    match p.next()? {
        5 => p.next().map(Color::Indexed),
        2 => Some(Color::Rgb(p.next()?, p.next()?, p.next()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_colours_and_drops_other_escapes() {
        let l = lines("\x1b[1;38;2;1;2;3mab\x1b[0mc\x1b[0K\x1b]8;;http://x\x07d\n\x1b[48;5;88me\x1b[49mf");
        assert_eq!(l.len(), 2);
        let spans: Vec<(&str, Style)> = l[0].spans.iter().map(|s| (s.content.as_ref(), s.style)).collect();
        let bold_rgb = Style::new().add_modifier(Modifier::BOLD).fg(Color::Rgb(1, 2, 3));
        assert_eq!(spans, [("ab", bold_rgb), ("c", Style::new()), ("d", Style::new())]);
        let spans: Vec<(&str, Style)> = l[1].spans.iter().map(|s| (s.content.as_ref(), s.style)).collect();
        assert_eq!(spans, [("e", Style::new().bg(Color::Indexed(88))), ("f", Style::new())]);
    }
}
