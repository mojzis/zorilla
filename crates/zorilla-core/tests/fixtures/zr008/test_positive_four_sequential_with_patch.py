from unittest.mock import patch


def test_many_with_patches():
    with patch("mod.a"):
        with patch("mod.b"):
            with patch("mod.c"):
                with patch("mod.d"):
                    assert True
