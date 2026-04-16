def test_try_block():
    try:
        value = int("not-a-number")
    except ValueError:
        value = 0
    assert value == 0
