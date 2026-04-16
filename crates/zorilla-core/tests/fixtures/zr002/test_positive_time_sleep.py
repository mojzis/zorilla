import time


def test_waits_a_second():
    trigger()
    time.sleep(1)
    assert done()
