from unittest import mock


def test_many_mock_patches():
    with mock.patch("mod.a"), mock.patch("mod.b"), mock.patch("mod.c"), mock.patch("mod.d"):
        assert True
