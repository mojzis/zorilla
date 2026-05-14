def test_misc():
    resp = random_thing.get("/x")
    assert resp.ok
