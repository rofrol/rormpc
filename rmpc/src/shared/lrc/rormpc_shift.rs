//! rormpc: a per-song shift of synced lyrics, kept in the song's own `.lrc` as its `[offset:]` tag.
//!
//! The LRC convention (and rmpc's parser, `parse_timestamp`) reads a positive `[offset:]` as "the lines come that
//! many milliseconds earlier". The shift here counts the other way, in the direction the keys move the lyrics: a
//! positive shift shows every line later, so it is stored as a negative tag. Shift 0 removes the tag, which brings
//! back the file as `musicdb lyrics` fetched it from LRCLIB (those files carry no `[offset:]`; a fetched file that
//! did carry one shows it as a shift, and reset drops it). Every line but the tag is kept byte for byte.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::lrc_parser::parse_metadata_only;

/// The shift a file's `[offset:]` tag applies, in milliseconds, positive meaning later.
pub(crate) fn shift_ms(content: &str) -> i64 {
    parse_metadata_only(content).0.offset.map_or(0, i64::saturating_neg)
}

/// A standalone header line holding a valid `[offset:]` tag (the parser ignores an unparsable value).
fn is_offset_line(line: &str) -> bool {
    line.trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .and_then(|tag| tag.split_once(':'))
        .is_some_and(|(key, value)| key.trim() == "offset" && value.trim().parse::<i64>().is_ok())
}

/// `content` with `shift_ms` as its only `[offset:]` tag, or without one for 0. The tag goes in the header,
/// where the parser reads it: in place of the first offset line, else right before the first timed line.
pub(crate) fn with_shift(content: &str, shift_ms: i64) -> Result<String> {
    let (_, lyrics_start) = parse_metadata_only(content);
    let newline = if content.contains("\r\n") { "\r\n" } else { "\n" };
    let tag = (shift_ms != 0).then(|| format!("[offset:{}]{newline}", shift_ms.saturating_neg()));

    let mut out = String::with_capacity(content.len() + 20);
    let mut tag = tag.as_deref();
    let mut lines = content.split_inclusive('\n').enumerate().peekable();
    while let Some((idx, line)) = lines.next() {
        if idx < lyrics_start && is_offset_line(line) {
            out.push_str(tag.take().unwrap_or_default());
            continue;
        }
        if idx == lyrics_start
            && let Some(tag) = tag.take()
        {
            out.push_str(tag);
            out.push_str(line);
            continue;
        }
        out.push_str(line);
        if lines.peek().is_none() && tag.is_some() && !line.ends_with('\n') {
            out.push_str(newline);
        }
    }
    if let Some(tag) = tag {
        // no timed line: the tag goes last
        out.push_str(tag);
    }

    if self::shift_ms(&out) != shift_ms {
        // a tag on a shared line (`[ar:x][offset:5]`) is left alone and still sets the offset
        bail!(
            "cannot reset the lyrics offset: the file has an [offset:] tag beside another tag on one line"
        );
    }
    Ok(out)
}

/// Writes `shift_ms` into the `.lrc` at `path`: a temporary file in the same directory, renamed over it (through a
/// symlink to its target), so a reader never sees half a file.
pub(crate) fn write_shift(path: &Path, shift_ms: i64) -> Result<()> {
    let path = std::fs::canonicalize(path)
        .with_context(|| format!("no lyrics file {}", path.display()))?;
    let content = std::fs::read_to_string(&path).context("failed to read the lyrics file")?;
    let new = with_shift(&content, shift_ms)?;
    if new == content {
        return Ok(());
    }
    let name = path.file_name().context("the lyrics file has no name")?.to_string_lossy();
    // not *.lrc, so the lyrics index and its hot reload never pick it up
    let tmp = path.with_file_name(format!(".{name}.rormpc-{}.tmp", std::process::id()));
    let written = std::fs::write(&tmp, new.as_bytes())
        .and_then(|()| std::fs::set_permissions(&tmp, std::fs::metadata(&path)?.permissions()))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.context("failed to replace the lyrics file")?;
    Ok(())
}

/// "+12.4 s" / "-0.5 s"
pub(crate) fn format_shift(shift_ms: i64) -> String {
    format!("{:+.1} s", shift_ms as f64 / 1000.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{format_shift, shift_ms, with_shift, write_shift};
    use crate::shared::lrc::{Lrc, LrcOffset};

    const FETCHED: &str = "[id: lrclib-1]\n[ar:Lil Nas X]\n[ti:MONTERO]\n[length: 02:17]\n\n[00:01.00]one\n[00:02.50] two \n";

    /// The first line's time in milliseconds with the global `lyrics_offset`.
    fn first_ms(content: &str, global: i64) -> u128 {
        let lrc: Lrc = content.parse().unwrap();
        lrc.lines[0].time(LrcOffset::from_millis(global)).as_millis()
    }

    #[test]
    fn a_file_without_a_tag_has_no_shift() {
        assert_eq!(shift_ms(FETCHED), 0);
        assert_eq!(first_ms(FETCHED, 0), 1000);
    }

    #[test]
    fn later_is_a_negative_tag_before_the_first_timed_line() {
        let shifted = with_shift(FETCHED, 12_400).unwrap();
        assert_eq!(
            shifted,
            "[id: lrclib-1]\n[ar:Lil Nas X]\n[ti:MONTERO]\n[length: 02:17]\n\n[offset:-12400]\n[00:01.00]one\n[00:02.50] two \n"
        );
        assert_eq!(shift_ms(&shifted), 12_400);
        assert_eq!(first_ms(&shifted, 0), 13_400);
    }

    #[test]
    fn earlier_is_a_positive_tag() {
        let shifted = with_shift(FETCHED, -500).unwrap();
        assert!(shifted.contains("[offset:500]\n"));
        assert_eq!(shift_ms(&shifted), -500);
        assert_eq!(first_ms(&shifted, 0), 500);
    }

    #[test]
    fn a_second_shift_replaces_the_tag_in_place() {
        let once = with_shift(FETCHED, 100).unwrap();
        let twice = with_shift(&once, 600).unwrap();
        assert_eq!(twice, once.replace("[offset:-100]", "[offset:-600]"));
        assert_eq!(twice.matches("offset").count(), 1);
    }

    #[test]
    fn reset_gives_back_the_fetched_file() {
        let shifted = with_shift(&with_shift(FETCHED, 12_400).unwrap(), -300).unwrap();
        assert_eq!(with_shift(&shifted, 0).unwrap(), FETCHED);
        assert_eq!(with_shift(FETCHED, 0).unwrap(), FETCHED);
    }

    #[test]
    fn a_fetched_tag_counts_as_a_shift_and_reset_drops_it() {
        let fetched = "[ar:A]\n[offset: +250]\n[ti:T]\n[00:01.00]one\n";
        assert_eq!(shift_ms(fetched), -250);
        assert_eq!(with_shift(fetched, 0).unwrap(), "[ar:A]\n[ti:T]\n[00:01.00]one\n");
        assert_eq!(
            with_shift(fetched, 1000).unwrap(),
            "[ar:A]\n[offset:-1000]\n[ti:T]\n[00:01.00]one\n"
        );
    }

    #[test]
    fn crlf_and_a_missing_final_newline_are_kept() {
        let fetched = "[ar:A]\r\n[00:01.00]one\r\n[00:02.00]two";
        let shifted = with_shift(fetched, 100).unwrap();
        assert_eq!(shifted, "[ar:A]\r\n[offset:-100]\r\n[00:01.00]one\r\n[00:02.00]two");
        assert_eq!(with_shift(&shifted, 0).unwrap(), fetched);
    }

    #[test]
    fn a_file_with_only_timed_lines_gets_the_tag_first() {
        let fetched = "[00:01.00]one\n";
        let shifted = with_shift(fetched, 100).unwrap();
        assert_eq!(shifted, "[offset:-100]\n[00:01.00]one\n");
        assert_eq!(with_shift(&shifted, 0).unwrap(), fetched);
    }

    #[test]
    fn an_offset_beside_another_tag_cannot_be_reset() {
        let fetched = "[ar:A][offset:5]\n[00:01.00]one\n";
        // the later tag wins, so a shift still works
        assert_eq!(shift_ms(&with_shift(fetched, 100).unwrap()), 100);
        assert!(with_shift(fetched, 0).is_err());
    }

    #[test]
    fn the_global_offset_adds_on_top() {
        let shifted = with_shift(FETCHED, 2000).unwrap();
        // lyrics_offset: 500 shows lines 0.5 s earlier, the song's shift 2 s later
        assert_eq!(first_ms(&shifted, 500), 2500);
        assert_eq!(first_ms(&shifted, -500), 3500);
    }

    #[test]
    fn the_write_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("rormpc-lyrics-shift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.lrc");
        std::fs::write(&path, FETCHED).unwrap();

        write_shift(&path, 12_400).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(shift_ms(&written), 12_400);
        assert_eq!(first_ms(&written, 0), 13_400);

        write_shift(&path, 0).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FETCHED);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shifts_read_as_signed_seconds() {
        assert_eq!(format_shift(12_400), "+12.4 s");
        assert_eq!(format_shift(-500), "-0.5 s");
    }
}
