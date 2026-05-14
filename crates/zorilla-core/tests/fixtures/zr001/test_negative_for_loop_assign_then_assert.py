def test_each_case_assign_then_assert():
    # SEMANTICS CHANGE from run-2: previously the assignment before the
    # assert broke the for-of-asserts shape. The broadened downgrade now
    # allows simple assignments interleaved with asserts.
    for case in CASES:
        result = transform(case)
        assert result
