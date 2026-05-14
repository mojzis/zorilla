def test_uses_mock_side_effect():
    # `.side_effect` is the sibling of `.return_value` — strings on its
    # RHS are mock-setup data, not a request target.
    mock_obj.side_effect = "/var/run/socket"
    assert mock_obj.side_effect
