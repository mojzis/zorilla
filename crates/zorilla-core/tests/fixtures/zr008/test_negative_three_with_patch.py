from unittest.mock import patch


def test_three_is_ok():
    with patch("mod.a"), patch("mod.b"), patch("mod.c"):
        assert True
