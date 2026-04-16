import time


def test_uses_monotonic():
    t = time.monotonic()
    assert t > 0
