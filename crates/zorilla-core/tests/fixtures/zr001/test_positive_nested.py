def test_with_inner_helper():
    def inner(x):
        if x > 0:
            return x
        return -x

    assert inner(-3) == 3
