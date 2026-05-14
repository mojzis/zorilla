from unittest.mock import patch


def test_many_in_one_clause():
    with patch("mod.a"), patch("mod.b"), patch("mod.c"), patch("mod.d"):
        assert True
