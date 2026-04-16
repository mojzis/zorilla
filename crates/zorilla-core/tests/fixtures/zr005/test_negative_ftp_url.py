def test_ftp_url():
    # ftp:// is outside the rule's scope in v0.1 — only http/https fire.
    data = fetch("ftp://files.example.com/data")
    assert data
