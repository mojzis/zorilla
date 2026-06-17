def test_attribute_rhs_paths():
    # Entry #86: a path/URL assigned to any object attribute is setup data
    # attached to an object (often a mock), not a resource the test opens.
    # Both the direct form and the one-call-deep helper-wrapped form are
    # carved out. esl's `obj.pdf_path` and introspect's helper-wrapped
    # `mock.return_value` are the rollout shapes.
    obj.pdf_path = "/etc/hosts"
    mock_run.return_value = _make_completed_process("/home/user/.git")
    assert obj.pdf_path and mock_run.return_value
