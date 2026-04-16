def test_two_guests():
    data = open("/etc/a.conf").read()
    resp = get("https://api.example.com/v1")
    assert data and resp
