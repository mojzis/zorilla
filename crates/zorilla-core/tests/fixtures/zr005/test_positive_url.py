def test_fetches():
    resp = get("https://api.example.com/v1/items")
    assert resp.ok
