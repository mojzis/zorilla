import time


def test_ignore_zr001_only():
    if True:  # zorilla: ignore[ZR001]
        assert True
    time.sleep(1)
    assert True
