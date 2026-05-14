def test_with_inline_helper():
    def helper(x):
        if x:
            return 1
        return 0

    helper(True)
    assert True
