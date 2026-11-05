use owo_colors::OwoColorize;

use super::{Paint, relative_time};
use crate::core::diff::Size;
use crate::core::show::Shown;

/// Widest +/− bar, in characters.
const BAR: usize = 30;

pub fn print(s: &Shown, p: &Paint) {
    // Same layout as the glog preview: id + refs, author, date, subject, body.
    let id = s.id.style(p.pick(owo_colors::Style::new().yellow().bold()));
    println!("{id} {}", super::log::labels(&s.refs, p).trim_end());
    println!("{} {}", s.author.style(p.blue()), format!("<{}>", s.email).style(p.dim()));
    println!(
        "{} {}",
        relative_time(s.time, crate::core::now()).style(p.green()),
        format!("· {}", date(s.time, s.offset)).style(p.dim())
    );
    if s.parents.len() > 1 {
        println!("{}", format!("merge of {}", s.parents.join(" + ")).style(p.dim()));
    }
    println!("\n{}", s.subject.style(p.pick(owo_colors::Style::new().bold())));
    if !s.body.is_empty() {
        println!("\n{}", s.body);
    }
    if s.files.is_empty() {
        return;
    }

    let names: Vec<String> = s
        .files
        .iter()
        .map(|f| match &f.from {
            Some(from) => format!("{from} → {}", f.path),
            None => f.path.clone(),
        })
        .collect();
    let name_w = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let lines = |sz: &Option<Size>| match sz {
        Some(Size::Lines { added, removed }) => (*added as usize, *removed as usize),
        _ => (0, 0),
    };
    let max = s.files.iter().map(|f| lines(&f.size)).map(|(a, r)| a + r).max().unwrap_or(0);
    let count_w = max.to_string().len();
    let (mut total_a, mut total_r) = (0, 0);

    println!();
    for (f, name) in s.files.iter().zip(&names) {
        let (a, r) = lines(&f.size);
        total_a += a;
        total_r += r;
        let pad = " ".repeat(name_w - name.chars().count());
        let stat = match f.size {
            Some(Size::Binary) => "binary".style(p.dim()).to_string(),
            None => String::new(),
            Some(Size::Lines { .. }) => {
                // Scale like `git show --stat`, but keep at least one mark for small changes.
                let scale = |n: usize| {
                    if max <= BAR { n } else { (n * BAR).div_ceil(max) }
                };
                format!(
                    "{:>count_w$} {}{}",
                    a + r,
                    "+".repeat(scale(a)).style(p.green()),
                    "−".repeat(scale(r)).style(p.red())
                )
            }
        };
        let kind = super::status::mark(f.kind);
        println!(" {} {name}{pad}  {stat}", kind.style(p.dim()));
    }
    let n = s.files.len();
    println!(
        "\n {} {}  {}",
        format!("{n} file{}", if n == 1 { "" } else { "s" }).style(p.dim()),
        format!("+{total_a}").style(p.green()),
        format!("−{total_r}").style(p.red())
    );
}

/// "Wed 05 Nov 2026 14:03" in the author's own time zone.
fn date(secs: i64, offset: i32) -> String {
    let local = secs + offset as i64;
    let days = local.div_euclid(86400);
    let rem = local.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{} {d:02} {} {y} {:02}:{:02}",
        WEEKDAYS[days.rem_euclid(7) as usize],
        MONTHS[(m - 1) as usize],
        rem / 3600,
        rem % 3600 / 60
    )
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
