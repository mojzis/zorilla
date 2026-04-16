def _helper(x):
    if x > 0:
        return x
    return -x


def test_uses_helper():
    assert _helper(-2) == 2


if __name__ == "__main__":
    test_uses_helper()
