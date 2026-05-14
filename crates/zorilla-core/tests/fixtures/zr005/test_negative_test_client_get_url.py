def test_lists_users():
    resp = client.get("/api/v1/users")
    assert resp.ok
