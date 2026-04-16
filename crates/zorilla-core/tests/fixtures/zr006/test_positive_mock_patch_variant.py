from unittest import mock


@mock.patch("mod.a")
@mock.patch("mod.b")
@mock.patch("mod.c")
@mock.patch("mod.d")
def test_many_mock_patches(d, c, b, a):
    assert True
