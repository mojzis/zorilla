from unittest.mock import patch


@patch("mod.a")
@patch("mod.b")
@patch("mod.c")
@patch("mod.d")
def _helper(d, c, b, a):
    return a


def test_uses_helper():
    assert _helper is not None
