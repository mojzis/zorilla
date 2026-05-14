def test_each_case_logic():
    for case in CASES:
        result = transform(case)
        assert result
