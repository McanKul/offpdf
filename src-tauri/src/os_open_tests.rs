//! Unit tests for `crate::os_open` argv / `file://` parsing (issue #74).

use std::path::PathBuf;

fn argv(args: &[&str]) -> Vec<PathBuf> {
    let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    crate::os_open::parse_opened_argv(owned)
}

fn token(s: &str) -> Option<PathBuf> {
    crate::os_open::parse_opened_token(s)
}

#[test]
fn os_open_parse_argv_plain() {
    assert_eq!(
        argv(&["offpdf", "/tmp/a.pdf"]),
        vec![PathBuf::from("/tmp/a.pdf")],
        "os-open-parse-argv-plain: /tmp/a.pdf must become an opened path",
    );
    assert_eq!(
        argv(&[
            "/Applications/OffPDF.app/Contents/MacOS/offpdf",
            "/tmp/a.pdf",
            "/tmp/b.png",
        ]),
        vec![PathBuf::from("/tmp/a.pdf"), PathBuf::from("/tmp/b.png")],
        "os-open-parse-argv-plain: every trailing path must be kept",
    );
}

#[test]
fn os_open_parse_spaces_unicode() {
    assert_eq!(
        token("/tmp/ünicode 报告.pdf"),
        Some(PathBuf::from("/tmp/ünicode 报告.pdf")),
        "os-open-parse-spaces-unicode: raw path with spaces and non-ASCII must survive",
    );
    assert_eq!(
        token("file:///tmp/my%20file.pdf"),
        Some(PathBuf::from("/tmp/my file.pdf")),
        "os-open-parse-spaces-unicode: file:// percent-decode must yield /tmp/my file.pdf",
    );
    assert_eq!(
        token("file:///tmp/%C3%BCnicode%20%E6%8A%A5%E5%91%8A.pdf"),
        Some(PathBuf::from("/tmp/ünicode 报告.pdf")),
        "os-open-parse-spaces-unicode: file:// percent-decode must yield /tmp/ünicode 报告.pdf",
    );
}

#[test]
fn os_open_parse_skip_flags_and_urls() {
    assert_eq!(
        argv(&[
            "offpdf",
            "-flag",
            "--help",
            "https://example.com/a.pdf",
            "/tmp/a.pdf",
        ]),
        vec![PathBuf::from("/tmp/a.pdf")],
        "os-open-parse-skip-flags-and-urls: drop argv[0], -flag, and https://",
    );
    assert_eq!(
        token("-flag"),
        None,
        "os-open-parse-skip-flags-and-urls: -flag is not an opened path",
    );
    assert_eq!(
        token("https://example.com/a.pdf"),
        None,
        "os-open-parse-skip-flags-and-urls: https:// is not an opened path",
    );
    assert_eq!(
        token("offpdf"),
        None,
        "os-open-parse-skip-flags-and-urls: a bare argv[0]-like token is not an opened path",
    );
}

#[test]
fn os_open_parse_utf8_cjk_tmp() {
    assert_eq!(
        token("/tmp/报告.pdf"),
        Some(PathBuf::from("/tmp/报告.pdf")),
        "R-UTF8: raw /tmp/报告.pdf must become Some without slicing at a non-char boundary",
    );
}

#[test]
fn os_open_parse_utf8_cjk_home() {
    assert_eq!(
        token("/home/用户/a.pdf"),
        Some(PathBuf::from("/home/用户/a.pdf")),
        "R-UTF8: raw /home/用户/a.pdf must become Some without slicing at a non-char boundary",
    );
}

#[test]
fn os_open_parse_utf8_file_url_cjk() {
    assert_eq!(
        token("file:///tmp/报告.pdf"),
        Some(PathBuf::from("/tmp/报告.pdf")),
        "R-UTF8: unencoded file:///tmp/报告.pdf must decode to /tmp/报告.pdf without panic",
    );
}

#[test]
fn os_open_parse_utf8_cjk_argv() {
    assert_eq!(
        argv(&["offpdf", "/tmp/报告.pdf", "/home/用户/a.pdf"]),
        vec![
            PathBuf::from("/tmp/报告.pdf"),
            PathBuf::from("/home/用户/a.pdf"),
        ],
        "R-UTF8: argv CJK paths must survive parse_opened_argv",
    );
}
