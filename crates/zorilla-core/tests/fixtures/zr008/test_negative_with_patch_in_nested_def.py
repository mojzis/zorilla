from unittest.mock import patch


def test_outer_with_one_patch():
    with patch("mod.a"):
        pass

    def helper():
        with patch("mod.b"):
            with patch("mod.c"):
                with patch("mod.d"):
                    pass

    helper()
