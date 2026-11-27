//! W3g: the directory a user names in a message stays local (A1 design
//! §3.1.1); the model sees a placeholder. Extraction rulings (review
//! 2026-10-27): a quoted path is taken up to its closing quote; an unquoted
//! path never contains a space; a non-ASCII letter ends the path, or
//! refuses it right after a separator or connector; a bare drive root and a
//! drive letter glued to a preceding letter or digit are not directories.
use insar_agent::message_dir::{extract, PLACEHOLDER};

#[test]
fn the_first_drive_letter_path_is_taken_out_and_replaced_everywhere() {
    let (dir, text) =
        extract(r"把结果下载到 D:\data\sentinel 里，再复制一份到 D:\data\sentinel").unwrap();
    assert_eq!(dir, r"D:\data\sentinel");
    assert_eq!(
        text,
        format!("把结果下载到 {PLACEHOLDER} 里，再复制一份到 {PLACEHOLDER}")
    );
    assert!(!text.contains(r":\"));
}

#[test]
fn a_path_ends_at_the_first_cjk_character() {
    for (message, expected, scrubbed) in [
        (
            r"下载到D:\data里",
            r"D:\data",
            format!("下载到{PLACEHOLDER}里"),
        ),
        (
            r"放到 D:\data\s1目录下",
            r"D:\data\s1",
            format!("放到 {PLACEHOLDER}目录下"),
        ),
        (r"D:\out，谢谢", r"D:\out", format!("{PLACEHOLDER}，谢谢")),
    ] {
        let (dir, text) = extract(message).unwrap_or_else(|| panic!("{message}"));
        assert_eq!(dir, expected, "{message}");
        assert_eq!(text, scrubbed, "{message}");
    }
}

#[test]
fn a_path_ends_at_the_regex_stops_and_at_full_width_punctuation() {
    for (message, expected) in [
        (r"输出到 E:\out\s1，谢谢", r"E:\out\s1"),
        (r"输出到E:\out\s1。", r"E:\out\s1"),
        (r"(输出到 E:\out\s1).", r"E:\out\s1"),
        (r"输出到 e:\OUT\a|b", r"e:\OUT\a"),
    ] {
        let (dir, text) = extract(message).unwrap_or_else(|| panic!("{message}"));
        assert_eq!(dir, expected, "{message}");
        assert!(text.contains(PLACEHOLDER), "{message}");
        assert!(!text.contains(expected), "{message}");
    }
}

/// Ruling (a): after an opening quote the path runs to the matching closing
/// quote, spaces and non-ASCII names included; without one nothing is taken.
#[test]
fn a_quoted_path_is_taken_up_to_its_closing_quote() {
    for (message, expected, scrubbed) in [
        (
            r#"输出到 "E:\out dir" 吧"#,
            r"E:\out dir",
            format!(r#"输出到 "{PLACEHOLDER}" 吧"#),
        ),
        (r"放到 'D:\a b'", r"D:\a b", format!("放到 '{PLACEHOLDER}'")),
        (
            r"放到 “D:\数据\s1”里",
            r"D:\数据\s1",
            format!("放到 “{PLACEHOLDER}”里"),
        ),
        (r"放到‘D:\x y’", r"D:\x y", format!("放到‘{PLACEHOLDER}’")),
        (
            r"放到「D:\My Data」里",
            r"D:\My Data",
            format!("放到「{PLACEHOLDER}」里"),
        ),
        (
            r"放到『D:\Müller』",
            r"D:\Müller",
            format!("放到『{PLACEHOLDER}』"),
        ),
    ] {
        let (dir, text) = extract(message).unwrap_or_else(|| panic!("{message}"));
        assert_eq!(dir, expected, "{message}");
        assert_eq!(text, scrubbed, "{message}");
    }
    for message in [
        r#"输出到 "E:\out dir 吧"#,
        r"放到「D:\My Data",
        r#"放到 "D:\""#,
        r#"放到 "D:\a|b""#,
        r#"放到 "D:\a\..\b""#,
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
}

/// Ruling (b): an unquoted path is never cut at a space. A space followed
/// by more ASCII text may be part of the name, so nothing is taken; a space
/// followed by the end, CJK text or full-width punctuation ends the path.
#[test]
fn an_unquoted_path_with_a_space_is_not_taken() {
    for message in [
        r"下载到 E:\out dir",
        r"下载到 D:\Program Files\x",
        r"save to D:\data please",
        r"下载到 D:\data (备份)",
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
    for (message, expected) in [
        (r"下载到 D:\data 里", r"D:\data"),
        (r"下载到 D:\data", r"D:\data"),
        (r"下载到 D:\data ，谢谢", r"D:\data"),
        (r"save to D:\data, please", r"D:\data"),
    ] {
        let (dir, _) = extract(message).unwrap_or_else(|| panic!("{message}"));
        assert_eq!(dir, expected, "{message}");
    }
}

/// Rulings (c) and (f): any non-ASCII letter (CJK, `ü`, ...) ends an
/// unquoted path; right after a separator or connector (`\ / _ - .` ...)
/// it is part of the name and nothing is taken. The connector rule looks
/// at the path before trailing punctuation is stripped.
#[test]
fn a_non_ascii_letter_ends_the_path_or_refuses_it_after_a_connector() {
    for message in [
        r"下载到 D:\数据\s1",
        r"下载到 D:\data\哨兵一号 里",
        r"下载到 D:\data_数据",
        r"下载到 D:\s1-数据 里",
        r"下载到 D:\data_.数据",
        r"下载到 D:\data.数据",
        r"download to D:\data\Ünits",
        r"下载到 D:\data(备份)",
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
    for (message, expected) in [
        (r"(放到 D:\data)里", r"D:\data"),
        (r"download to D:\Müller", r"D:\M"),
        (r"下载到 D:\s1数据", r"D:\s1"),
    ] {
        let (dir, _) = extract(message).unwrap_or_else(|| panic!("{message}"));
        assert_eq!(dir, expected, "{message}");
    }
}

/// Ruling (d): a bare drive root is not a download directory.
#[test]
fn a_bare_drive_root_is_not_taken() {
    for message in [
        "输出到 F:\\",
        "输出到 F:\\，谢谢",
        "输出到 F:\\.",
        "输出到 D:/",
        r#"输出到 "F:\""#,
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
}

/// Ruling (e): a drive letter glued to a preceding ASCII letter or digit is
/// not a drive (`abc:\x`, a regex `re:\d+`, `TODO:\n`); a later path with
/// a clean boundary is still found.
#[test]
fn a_drive_letter_must_not_follow_an_ascii_letter_or_digit() {
    for message in [r"abc:\x", r"用 re:\d+ 匹配", r"TODO:\n 写完", r"9:\x"] {
        assert_eq!(extract(message), None, "{message}");
    }
    let (dir, text) = extract(r"用 re:\d+ 匹配后下载到 D:\data").unwrap();
    assert_eq!(dir, r"D:\data");
    assert_eq!(text, format!(r"用 re:\d+ 匹配后下载到 {PLACEHOLDER}"));
    let (dir, _) = extract(r"_D:\x").unwrap();
    assert_eq!(dir, r"D:\x");
}

#[test]
fn text_without_a_usable_directory_is_left_alone() {
    for message in [
        "检索西宁北的 Sentinel-1 数据",
        r"下载到 \\server\share\out",
        r"下载到 relative\out",
        r"下载到 D:\data\..\secret",
        "下载到 D:/data/forward",
        "比例是 3:2",
        "见 https://example.com/a",
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
}

#[test]
fn only_the_first_directory_is_the_message_directory() {
    let (dir, text) = extract(r"下载到 D:\a 然后看看 C:\Users\me\b").unwrap();
    assert_eq!(dir, r"D:\a");
    assert_eq!(
        text,
        format!(r"下载到 {PLACEHOLDER} 然后看看 C:\Users\me\b")
    );
}

/// Only whole occurrences are replaced: a longer path that merely starts
/// with the directory is left for the outbound scrub.
#[test]
fn a_longer_path_starting_with_the_directory_is_not_cut() {
    let (dir, text) = extract(r"下载到 D:\a 别用 D:\abc").unwrap();
    assert_eq!(dir, r"D:\a");
    assert_eq!(text, format!(r"下载到 {PLACEHOLDER} 别用 D:\abc"));
}

/// A Unix absolute path is taken out the same way a drive-letter path is
/// (review 2026-11-02): the syntax is recognised on every platform; whether
/// one may be an output directory is `permission::valid_output_dir`'s call.
#[test]
fn a_unix_path_in_the_message_is_taken_and_stays_local() {
    for (message, expected) in [
        ("下载到 /home/me/data 里", "/home/me/data"),
        ("存到 /media/backup 里", "/media/backup"),
        ("下载到 /root/x里", "/root/x"),
        ("路径：/srv/geo；帮忙", "/srv/geo"),
        ("下载到 /data 里", "/data"),
    ] {
        let (dir, text) = extract(message).unwrap_or_else(|| panic!("{message}"));
        assert_eq!(dir, expected, "{message}");
        assert!(text.contains(PLACEHOLDER), "{message}: {text}");
        assert!(!text.contains(&expected[1..]), "{message}: {text}");
    }
    // A path that is the whole message: only the directory matters (the
    // outbound scrub replaces whatever remains).
    let (dir, _) = extract("/mnt/d/S1/").unwrap();
    assert_eq!(dir, "/mnt/d/S1");
    // A quoted path runs to its closing quote; a path in parentheses and
    // one ending a sentence drop the punctuation that follows it.
    let (dir, text) = extract(r#"存到 "/var/tmp/out""#).unwrap();
    assert_eq!(dir, "/var/tmp/out");
    assert_eq!(text, format!(r#"存到 "{PLACEHOLDER}""#));
    let (dir, _) = extract("(/etc/config)").unwrap();
    assert_eq!(dir, "/etc/config");
    let (dir, _) = extract("/data/S1。").unwrap();
    assert_eq!(dir, "/data/S1");
    // A file is never the message's directory (integration ruling 8fcabc8a,
    // which applies to Unix paths as it does to drive-letter paths).
    assert!(extract("/data/S1.zip。").is_none());
    // The first character of the message is a clean left boundary.
    let (dir, _) = extract("/opt/x").unwrap();
    assert_eq!(dir, "/opt/x");
}

/// A Unix path glued to Chinese text is taken whole when the text follows a
/// separator or connector (the connector rule, merged in as the scrub does),
/// and cut before it when it does not.
#[test]
fn a_unix_path_glued_to_chinese_text_is_taken_whole() {
    let (dir, text) = extract("下载到/home/张三/下载").unwrap();
    assert_eq!(dir, "/home/张三/下载");
    assert_eq!(text, format!("下载到{PLACEHOLDER}"));
    let (dir, text) = extract("下载到/home/me/数据里").unwrap();
    assert_eq!(dir, "/home/me/数据里");
    assert_eq!(text, format!("下载到{PLACEHOLDER}"));
    let (dir, text) = extract("下载到/root/x里").unwrap();
    assert_eq!(dir, "/root/x");
    assert_eq!(text, format!("下载到{PLACEHOLDER}里"));
    let (dir, text) = extract("存到/media/backup 里").unwrap();
    assert_eq!(dir, "/media/backup");
    assert_eq!(text, format!("存到{PLACEHOLDER} 里"));
}

/// A single-segment-relative candidate (`./x`, `../x`, `~/x`) is never the
/// message directory: the host cannot vouch for where it points, and the
/// outbound scrub still replaces it.
#[test]
fn a_single_segment_or_relative_or_tilde_path_is_not_taken() {
    for message in [
        "下载到 ./out 里",
        "下载到 ../out 里",
        "下载到 ~/out 里",
        "下载到~/out",
        "/",
        "存到 // 里",
        "下载到 /a/../b",
        "下载到 /./b",
        r"下载到 /home\me",
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
}

/// Words, dates and addresses that merely hold a `/` are not paths: the
/// character before the candidate rules them out, or the first segment
/// does not start with a letter.
#[test]
fn slash_words_dates_and_urls_are_not_paths() {
    for message in [
        "见 https://a/b 下载",
        "and/or 一下",
        "TCP/IP 配置",
        "km/h 速度",
        "比例 1/2",
        "math.pi/2 计算",
        "日期 2026/11/02",
        "24/7",
        "他/她",
        "a/b/c",
        "“/ home”",
        "//server",
        "就一个 / 字符",
    ] {
        assert_eq!(extract(message), None, "{message}");
    }
    // A `/` after a drive-letter candidate with a dirty boundary is still
    // found; a later Unix path is found after a dirty earlier one.
    let (dir, _) = extract(r"用 re:\d+ 匹配后下载到 /data").unwrap();
    assert_eq!(dir, "/data");
}
