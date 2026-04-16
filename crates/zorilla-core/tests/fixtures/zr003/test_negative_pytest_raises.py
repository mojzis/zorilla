import pytest


def test_raises_counts_as_assert():
    with pytest.raises(ValueError):
        boom()


def test_warns_counts_as_assert():
    with pytest.warns(DeprecationWarning):
        legacy()
