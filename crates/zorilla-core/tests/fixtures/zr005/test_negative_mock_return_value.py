def test_uses_mock_return_value():
    # Configuring `.return_value` with a path-like string is a mock setup
    # pattern (cteni's 48-hit shape), not a request target. ZR005 must
    # not flag the RHS of `.return_value` assignments.
    mock_repo.return_value = "/srv/data/file.txt"
    mock_obj.get.return_value = "/api/v1/users"
    assert mock_repo.return_value
