def test_membership_assert_dynamic_rhs():
    # Entry #86: the literal is the LHS needle of an `in` comparison
    # asserted against a dynamic runtime value (a response body / header).
    # It is a substring expectation, not a request target. esl's
    # `assert "/login" in response.text` is the rollout shape.
    resp = client.get("/page")
    assert "/login" in resp.text
    assert "/dashboard" in resp.headers["location"]
