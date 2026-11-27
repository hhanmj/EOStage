//! A local directory named in the user's message (A1 design §3.1.1, W3g).
//! The host takes it out of the text before anything leaves the machine and
//! keeps it in the run; the model sees [`PLACEHOLDER`] instead and may name it
//! as `{"ref": "message_dir"}` in a create_task's `options.output_root`,
//! which the host resolves back to the directory (`output_dir_source =
//! from_message`, which the gate treats as known, so `auto_approve` may use
//! it without a card: every rule below that cannot be sure of the name
//! takes nothing and leaves the choice to the card).
use crate::permission;

/// The reference the model writes for the message's directory.
pub const REF: &str = "message_dir";
/// What the model sees in place of the directory.
pub const PLACEHOLDER: &str = "[local-reference:message_dir]";

/// Opening quotes and the closing quote each one expects.
const QUOTES: [(char, char); 6] = [
    ('"', '"'),
    ('\'', '\''),
    ('“', '”'),
    ('‘', '’'),
    ('「', '」'),
    ('『', '』'),
];

/// A character an unquoted path can hold: printable ASCII except the regex
/// stops `"'<>|?*` (`[A-Za-z]:\\[^\s"'<>|?*]+`). Whitespace, any non-ASCII
/// character (CJK, `ü`, full-width punctuation) and the stops end it.
fn in_path(c: char) -> bool {
    c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '<' | '>' | '|' | '?' | '*')
}

/// File extensions that make a path a file, not a directory (L4 r9 P2-8,
/// r10 P2-7: `…\aoi.geojson` was taken as the message's directory). A
/// Sentinel `.SAFE` product is a directory and is not listed.
const FILE_EXTENSIONS: [&str; 41] = [
    "geojson", "json", "shp", "shx", "dbf", "prj", "kml", "kmz", "gpkg", "gml", "zip", "7z", "rar",
    "tar", "gz", "tgz", "tif", "tiff", "h5", "he5", "hdf", "nc", "txt", "csv", "tsv", "xml",
    "yaml", "yml", "toml", "ini", "cfg", "log", "md", "pdf", "png", "jpg", "jpeg", "ps1", "py",
    "bat", "exe",
];

/// The path's last segment ends in a known file extension.
fn names_a_file(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty() && FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

/// ASCII sentence punctuation that ends a sentence rather than the path.
const TRAILING: [char; 6] = ['.', ',', ';', ':', ')', ']'];

/// Characters that join a name to what follows (`D:\data_数据`): a
/// non-ASCII letter right after one is part of the directory's name.
pub(crate) fn connector(c: char) -> bool {
    matches!(
        c,
        '\\' | '/'
            | '_'
            | '-'
            | '.'
            | '~'
            | '+'
            | '#'
            | '&'
            | '@'
            | '$'
            | '%'
            | '='
            | '^'
            | '!'
            | '`'
            | '('
            | '['
            | '{'
    )
}

/// The first local directory in `text` — a drive-letter path as reviewed
/// 2026-10-27 (rulings below), or a `/...` path (review 2026-11-02) — and
/// the text with each whole occurrence of it replaced by [`PLACEHOLDER`].
/// Drive-letter rulings:
/// - the drive letter must not follow an ASCII letter or digit (`re:\d+`,
///   `TODO:\n`); such a candidate is skipped and the search goes on;
/// - after an opening quote (`" ' “ ‘ 「 『`) the path runs to the matching
///   closing quote, spaces and non-ASCII names included; no closing quote,
///   or a stop character inside, takes nothing;
/// - an unquoted path ends at whitespace, at a stop character or at the
///   first non-ASCII character (`D:\data里` names `D:\data`); a space
///   followed by more ASCII text (`D:\My Data`, `D:\data please`) may be
///   part of the name, so nothing is taken rather than cutting there; a
///   non-ASCII letter right after a connector (`D:\数据`, `D:\data_.数据`)
///   is part of the name, so nothing is taken; trailing `.,;:)]` is
///   stripped only after these checks;
/// - a bare drive root and any path `permission::valid_drive_letter_dir`
///   refuses (`.` or `..` segments, over 1024 bytes) are not taken;
/// - a path whose last segment ends in a known file extension
///   (`FILE_EXTENSIONS`: `.geojson`, `.json`, `.shp`, `.zip`, `.tif`, …) is a
///   file, not a directory, and is not taken.
///
/// Unix-path rulings (2026-11-02): a candidate starts at a `/` whose
/// previous character is not an ASCII letter, digit, `.`, `~` or `:` (so
/// `https://a/b`, `TCP/IP`, `math.pi/2` are not paths) and whose next
/// character is neither `/` (so `//server` is not) nor whitespace (a lone
/// `/` is not), and whose first segment starts with a letter, ASCII or
/// not (so `1/2`, `2026/11/02`, `24/7` are not; `/home`, `/data` are).
/// After the start the candidate follows the drive-letter rulings quoted
/// or unquoted (it ends at whitespace, a stop character, CJK text or
/// full-width punctuation; `下载到/data里` names `/data`,
/// `下载到/home/张三/下载` is taken whole, `/data/S1.zip。` drops the
/// `。`). The same `.,;:)]` is stripped, and any path
/// `permission::valid_unix_dir` refuses (`.` or `..` segments, a
/// backslash, over 1024 bytes) is not taken. `~/...`, `./...` and
/// `../...` are never taken (relative or home-relative, not a directory
/// the host can vouch for); the outbound scrub still replaces them.
///
/// Drive-letter and `/` candidates are one ordered list: the first with a
/// clean left boundary wins. Whatever is not taken is left for the
/// outbound scrub (which replaces it with `[local-reference]`); a
/// `message_dir` reference then resolves to no directory and the card
/// asks for one.
pub fn extract(text: &str) -> Option<(String, String)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let drive_start = (0..chars.len().saturating_sub(2)).find(|&k| {
        chars[k].1.is_ascii_alphabetic()
            && chars[k + 1].1 == ':'
            && chars[k + 2].1 == '\\'
            && (k == 0 || !chars[k - 1].1.is_ascii_alphanumeric())
    });
    let unix_start = (0..chars.len()).find(|&k| {
        chars[k].1 == '/'
            && (k == 0 || {
                let previous = chars[k - 1].1;
                !previous.is_ascii_alphanumeric() && !matches!(previous, '.' | '~' | ':' | '/')
            })
            && !matches!(chars.get(k + 1).map(|(_, c)| *c), None | Some('/'))
            && !chars.get(k + 1).is_some_and(|(_, c)| c.is_whitespace())
            && (chars[k + 1].1.is_ascii_alphabetic()
                || (chars[k + 1].1.is_alphabetic()
                    && chars[k + 2..].iter().any(|(_, c)| *c == '/')))
    });
    // The first candidate with a clean left boundary: earlier drive-letter
    // candidates with a dirty boundary are skipped, a `/` with one is not.
    let start = match (drive_start, unix_start) {
        (Some(drive), Some(unix)) => Some(drive.min(unix)),
        (drive, unix) => drive.or(unix),
    }?;
    let (begin, is_unix) = (chars[start].0, chars[start].1 == '/');
    let quote = start
        .checked_sub(1)
        .and_then(|k| QUOTES.iter().find(|(open, _)| *open == chars[k].1));
    let valid = if is_unix {
        permission::valid_unix_dir
    } else {
        permission::valid_drive_letter_dir
    };
    let path = match quote {
        Some(&(_, close)) => {
            let from = if is_unix { start + 1 } else { start + 3 };
            let (end, _) = chars[from..].iter().find(|(_, c)| *c == close)?;
            let path = &text[begin..*end];
            if path
                .chars()
                .any(|c| matches!(c, '"' | '<' | '>' | '|' | '?' | '*'))
            {
                return None;
            }
            path
        }
        None => {
            let from = if is_unix { start + 1 } else { start + 3 };
            let cut = chars[from..].iter().find(|(_, c)| !in_path(*c));
            let end = cut.map_or(text.len(), |(index, _)| *index);
            let raw = &text[begin..end];
            let last = raw.chars().next_back()?;
            match cut {
                Some(&(index, c)) if c.is_whitespace() && !TRAILING.contains(&last) => {
                    let next = text[index..].chars().find(|c| !c.is_whitespace());
                    if next.is_some_and(|c| c.is_ascii_graphic()) {
                        return None;
                    }
                }
                // A non-ASCII letter after a separator or connector is part
                // of a Unix path's name and takes the rest of the word with
                // it (`下载到/home/张三/下载`); a drive-letter path instead
                // refuses, as ruled 2026-10-27 (`D:\数据`).
                Some(&(index, c))
                    if is_unix
                        && c.is_alphanumeric()
                        && !c.is_ascii()
                        && (connector(last)
                            || text[index..]
                                .chars()
                                .take_while(|c| !c.is_whitespace())
                                .any(|c| c == '/')) =>
                {
                    // The word runs on to its next boundary: whitespace or
                    // a stop (non-ASCII letters are the name, non-ASCII
                    // punctuation ends it: `，谢谢` stays).
                    let word_end = text[index..]
                        .char_indices()
                        .find(|(_, c)| {
                            c.is_whitespace()
                                || (c.is_ascii() && !in_path(*c))
                                || (!c.is_ascii() && !c.is_alphanumeric())
                        })
                        .map_or(text.len(), |(offset, _)| index + offset);
                    let whole = &text[begin..word_end];
                    return if valid(whole.trim_end_matches('/')) {
                        Some((whole.to_owned(), replace_bounded(text, whole)))
                    } else {
                        None
                    };
                }
                Some(&(_, c)) if c.is_alphanumeric() && connector(last) => return None,
                _ => {}
            }
            // A Unix path's trailing separator and sentence punctuation
            // are not part of it (`/mnt/d/S1/`, `/data/S1.zip。`).
            if is_unix {
                raw.trim_end_matches(|c: char| TRAILING.contains(&c) || c == '/')
            } else {
                raw.trim_end_matches(TRAILING)
            }
        }
    };
    let minimum = if is_unix { 2 } else { 4 };
    if path.chars().count() < minimum || !valid(path) || names_a_file(path) {
        return None;
    }
    Some((path.to_owned(), replace_bounded(text, path)))
}

/// Replaces each occurrence of `path` that ends where the path would end
/// (so `D:\a` in `D:\abc` stays for the outbound scrub).
fn replace_bounded(text: &str, path: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(found) = rest.find(path) {
        let after = &rest[found + path.len()..];
        let bounded = after
            .chars()
            .next()
            .is_none_or(|c| !in_path(c) || TRAILING.contains(&c));
        output.push_str(&rest[..found]);
        output.push_str(if bounded { PLACEHOLDER } else { path });
        rest = after;
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L4 r9 P2-8 / r10 P2-7: a file path (`…\aoi.geojson`) was taken as the
    /// message's directory. A path ending in a known file extension is not a
    /// directory; it is left for the outbound scrub.
    #[test]
    fn a_file_path_is_not_the_message_directory() {
        for text in [
            "AOI 文件在 C:\\example\\fixtures\\aoi.geojson。先复述",
            "配置在 C:\\example\\fixtures\\config.json。",
            "脚本在 C:\\example\\fixtures\\run.ps1。",
            "把 \"D:\\My Data\\aoi.shp\" 读进来",
            "数据在 D:\\data\\S1A_IW_SLC.zip 里",
            "D:\\data\\scene.TIF 这个文件",
        ] {
            assert_eq!(extract(text), None, "{text}");
        }
        assert_eq!(
            crate::history::outbound_text("AOI 文件在 C:\\example\\fixtures\\aoi.geojson。先复述"),
            "AOI 文件在 [local-reference]。先复述"
        );
    }

    /// Directories, including one named like a SAFE product or with a dot
    /// in a segment, are still taken.
    #[test]
    fn a_directory_path_is_still_the_message_directory() {
        for (text, path) in [
            (
                "下载到 D:\\EOStage-Test\\r10 这个目录",
                "D:\\EOStage-Test\\r10",
            ),
            (
                "放到 D:\\data\\S1A_IW_SLC.SAFE 里",
                "D:\\data\\S1A_IW_SLC.SAFE",
            ),
            ("放到 D:\\data\\v1.2 下", "D:\\data\\v1.2"),
            ("放到 \"D:\\My Data\\out\" 里", "D:\\My Data\\out"),
        ] {
            let (found, _) = extract(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(found, path, "{text}");
        }
    }
}
