from unittest.mock import patch


class Foo:
    x = 1
    y = 2
    z = 3
    w = 4


def test_patch_object_not_counted():
    with patch.object(Foo, "x"):
        with patch.object(Foo, "y"):
            with patch.object(Foo, "z"):
                with patch.object(Foo, "w"):
                    assert True
