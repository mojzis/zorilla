import time


def test_sleeps_twice():
    time.sleep(1)
    trigger()
    time.sleep(2)
    assert done()
