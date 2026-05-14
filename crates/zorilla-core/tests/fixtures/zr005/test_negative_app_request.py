def test_app_get_path():
    resp = app.get("/x")
    assert resp.ok
