def test_response_headers_membership():
    # `assert "..." in response.headers[...]` is alma's pattern: the
    # literal on the LHS of `in` is the expected substring/value, not a
    # mystery-guest request target.
    response = client.get("/foo")
    assert "/api/v1/users" in response.headers["location"]
    assert "/login" in resp.headers["Location"]
