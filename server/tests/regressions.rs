use chrono::{DateTime, Utc};
use emby_manager::{auth, c115, catalog, media_fs, scheduler};

#[test]
fn legacy_pbkdf2_matches_python_hashlib_vector() {
    let stored = concat!(
        "pbkdf2_sha256$200000$",
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f$",
        "1165616ecb836bedee778fa9b2a68c99e00119c9a60e0dbfe5da266a7a7d42d0"
    );
    assert!(auth::verify_password("hunter2", stored));
    assert!(!auth::verify_password("hunter3", stored));
    assert!(!auth::verify_password("hunter2", "pbkdf2_sha256$abc$ff$ff"));
}

#[test]
fn media_fs_rejects_traversal_and_keeps_valid_names() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("sub")).unwrap();

    let valid = media_fs::safe_under(tmp.path(), "sub/电影.mkv").unwrap();
    assert!(valid.ends_with("sub/电影.mkv"));

    for bad in [
        "",
        ".",
        "..",
        "../etc",
        "sub/../../etc",
        "..\\etc",
        "/etc/passwd",
    ] {
        assert!(media_fs::safe_under(tmp.path(), bad).is_err(), "{bad}");
    }
    assert!(media_fs::safe_under(tmp.path(), "evil\0.txt").is_err());
}

#[cfg(unix)]
#[test]
fn media_fs_rejects_symlink_escape() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("link")).unwrap();

    assert!(media_fs::safe_under(tmp.path(), "link/file.mkv").is_err());
}

#[test]
fn scheduler_monthly_day_31_clamps_to_month_end() {
    let schedule = serde_json::json!({"mode":"monthly","hour":3,"minute":0,"day":31});
    let now = DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    assert_eq!(
        scheduler::next_run(&schedule, now).unwrap().to_rfc3339(),
        "2026-02-28T03:00:00+00:00"
    );
}

#[test]
fn c115_parses_share_links_and_manual_codes() {
    assert_eq!(
        c115::parse_115_url(" https://115.com/s/swABC?password=YYY#anchor ", None),
        (Some("swABC".to_string()), Some("YYY".to_string()))
    );
    assert_eq!(
        c115::parse_115_url("https://115cdn.com/s/swXYZ?pwd=ABC", Some(" OVERRIDE ")),
        (Some("swXYZ".to_string()), Some("OVERRIDE".to_string()))
    );
    assert_eq!(
        c115::parse_115_url("swABC YYY", None),
        (Some("swABC".to_string()), Some("YYY".to_string()))
    );
    assert_eq!(c115::parse_115_url("!!!@@@", None), (None, None));
}

#[test]
fn catalog_parses_115_share_links() {
    assert_eq!(
        catalog::infer_type("https://115cdn.com/s/swabc"),
        "share115"
    );
    assert_eq!(catalog::infer_type("ed2k://|file|x.mkv|1|hash|/"), "ed2k");

    let (share, rc) = catalog::parse_share("https://anxia.com/s/swABC?pwd=YYY&#frag");
    assert_eq!(share.as_deref(), Some("swABC"));
    assert_eq!(rc.as_deref(), Some("YYY"));
}
