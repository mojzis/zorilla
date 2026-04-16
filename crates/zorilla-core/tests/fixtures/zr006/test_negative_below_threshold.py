from unittest.mock import patch


@patch("mod.a")
@patch("mod.b")
@patch("mod.c")
def test_three_is_ok(c, b, a):
    assert True
