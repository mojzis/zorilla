from unittest.mock import patch


@patch("mod.a")
@patch("mod.b")
@patch("mod.c")
@patch("mod.d")
def test_many_patches(d, c, b, a):
    assert True
