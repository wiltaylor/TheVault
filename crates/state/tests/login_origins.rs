use state::canonical_origin;

#[test]
fn login_origins_are_canonical_and_exact() {
    for (input, expected) in [
        ("HTTPS://Example.COM", "https://example.com"),
        ("https://example.com.", "https://example.com"),
        ("https://example.com:443/", "https://example.com"),
        ("http://example.com:80/", "http://example.com"),
        ("https://example.com:80", "https://example.com:80"),
        ("https://example.com:8443", "https://example.com:8443"),
        ("https://аpple.com", "https://xn--pple-43d.com"),
        (
            "https://ＢÜＣＨＥＲ.example",
            "https://xn--bcher-kva.example",
        ),
        ("https://exa%6Dple.com", "https://example.com"),
        ("https://exa\tmple.com", "https://example.com"),
        ("https://exa\nmple.com", "https://example.com"),
        ("http://0x7f.1", "http://127.0.0.1"),
        ("http://2130706433", "http://127.0.0.1"),
        ("http://[::FFFF:127.0.0.1]:80", "http://[::ffff:7f00:1]"),
    ] {
        assert_eq!(
            canonical_origin(input).as_deref(),
            Some(expected),
            "{input}"
        );
    }
}

#[test]
fn login_origins_reject_ambiguous_domains_and_non_origins() {
    for input in [
        "https://example.com..",
        "https://.example.com",
        "https://ex..ample.com",
        "https://*.example.com",
        "https://exa_mple.com",
        "https://user@example.com",
        "https://:@example.com",
        "https://@example.com",
        "https://example.com@evil.test",
        "https://example.com/path",
        "https://example.com/a/..",
        "https://example.com?",
        "https://example.com#",
        "https://example.com\\",
        "https://example.com:443:",
        "https://example.com:99999",
        " https://example.com",
        "https://example.com ",
        "https:example.com",
        "https:///example.com",
        "//example.com",
        "example.com",
        "https://",
        "ftp://example.com",
        "ws://example.com",
        "wss://example.com",
        "javascript:alert(1)",
        "data:text/plain,hello",
        "blob:https://example.com/id",
        "file:///tmp",
        "chrome://settings",
    ] {
        assert!(canonical_origin(input).is_none(), "{input}");
    }
}
