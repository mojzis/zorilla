def test_for_loop_with_if_inside_still_fires():
    cases = [1, 2, 3]
    for case in cases:
        if case > 0:
            assert case == 1
