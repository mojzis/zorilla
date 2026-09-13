def test_fetches_synthetic_host():
    resp = get("https://example.invalid/project")
    assert resp.ok
