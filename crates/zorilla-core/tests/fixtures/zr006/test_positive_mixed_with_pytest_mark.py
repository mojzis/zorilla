import pytest
from unittest.mock import patch


@pytest.mark.slow
@patch("mod.a")
@patch("mod.b")
@patch("mod.c")
@patch("mod.d")
def test_mark_plus_four_patches(d, c, b, a):
    assert True
