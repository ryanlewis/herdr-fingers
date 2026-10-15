//! Link mode: where labelled links may be on screen, and what a `file://`
//! link names.
//!
//! A pane dump carries no link addresses, only how the text was drawn.
//! Programs that print a link label almost always colour or underline it,
//! so each run of such text is a place to ask the host whether a link is
//! there.

use super::screen::{Screen, Segment};
use super::style::TextStyle;

/// The pattern name given to links found in link mode.
pub const LINK_PATTERN: &str = "link";

/// Every run of cells on one row drawn in the same colour or underline,
/// without its blank ends, in reading order.
pub fn styled_runs(screen: &Screen) -> Vec<Segment> {
    let mut runs = Vec::new();
    for (row_index, row) in screen.rows().iter().enumerate() {
        let mut col: u16 = 0;
        let mut current: Option<(TextStyle, Segment)> = None;
        for cell in &row.cells {
            let width = u16::from(cell.width);
            let marked = cell.style.fg.is_some() || cell.style.underline;
            match &mut current {
                Some((style, run)) if *style == cell.style && !cell.is_blank() => {
                    run.width = col + width - run.col;
                }
                Some((style, _)) if *style == cell.style => {}
                _ => {
                    runs.extend(current.take().map(|(_, run)| run));
                    if marked && !cell.is_blank() {
                        current = Some((
                            cell.style,
                            Segment {
                                row: row_index,
                                col,
                                width,
                            },
                        ));
                    }
                }
            }
            col = col.saturating_add(width);
        }
        runs.extend(current.map(|(_, run)| run));
    }
    runs
}

/// The absolute path a `file://` link names, if it names one on this
/// machine: no host other than `localhost`, query and fragment dropped,
/// percent-escapes decoded.
pub fn file_url_path(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or(path);
    percent_decode(path)
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(ansi: &str) -> Vec<(usize, u16, u16)> {
        styled_runs(&Screen::from_ansi(ansi, 80))
            .iter()
            .map(|run| (run.row, run.col, run.width))
            .collect()
    }

    #[test]
    fn plain_text_has_no_runs() {
        assert!(runs("see PR #42 here\nand more").is_empty());
    }

    #[test]
    fn coloured_and_underlined_text_each_make_a_run() {
        assert_eq!(
            runs("see \x1b[34mPR #42\x1b[0m and \x1b[4mdocs\x1b[24m."),
            vec![(0, 4, 6), (0, 15, 4)]
        );
    }

    #[test]
    fn a_run_ends_where_the_style_changes() {
        assert_eq!(
            runs("\x1b[34mone\x1b[35mtwo\x1b[0m"),
            vec![(0, 0, 3), (0, 3, 3)]
        );
    }

    #[test]
    fn blank_ends_are_not_part_of_a_run() {
        assert_eq!(runs("a\x1b[4m  link  \x1b[0mb"), vec![(0, 3, 4)]);
    }

    #[test]
    fn a_background_alone_does_not_mark_a_link() {
        assert!(runs("\x1b[44mstatus bar\x1b[0m").is_empty());
    }

    #[test]
    fn runs_are_counted_in_columns_and_listed_row_by_row() {
        assert_eq!(
            runs("日本 \x1b[32mx\x1b[0m\n\x1b[32my\x1b[0m"),
            vec![(0, 5, 1), (1, 0, 1)]
        );
    }

    #[test]
    fn a_local_file_link_names_its_decoded_path() {
        assert_eq!(
            file_url_path("file:///Users/me/My%20Notes/a.md").as_deref(),
            Some("/Users/me/My Notes/a.md")
        );
        assert_eq!(
            file_url_path("FILE://localhost/tmp/x.rs?raw#L3").as_deref(),
            Some("/tmp/x.rs")
        );
    }

    #[test]
    fn other_links_name_no_local_path() {
        for url in [
            "https://example.com/a",
            "file://server/share/a",
            "file:relative",
            "file:///bad%zzescape",
            "file:///%FF",
        ] {
            assert_eq!(file_url_path(url), None, "{url}");
        }
    }
}
