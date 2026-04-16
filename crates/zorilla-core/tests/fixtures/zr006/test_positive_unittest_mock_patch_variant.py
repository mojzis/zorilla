import unittest.mock


@unittest.mock.patch("mod.a")
@unittest.mock.patch("mod.b")
@unittest.mock.patch("mod.c")
@unittest.mock.patch("mod.d")
def test_many_unittest_mock_patches(d, c, b, a):
    assert True
