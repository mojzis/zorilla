def test_with_headers():
    resp = client.get("/healthz", headers={"x": "https://leak.example/"})
    assert resp.ok
