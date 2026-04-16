import time


def test_ignore_only_the_if():
    if True:  # zorilla: ignore
        assert True
    time.sleep(1)
    assert True
