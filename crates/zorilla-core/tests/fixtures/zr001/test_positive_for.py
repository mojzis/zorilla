def test_for_loop():
    total = 0
    for i in range(3):
        total += i
    assert total == 3
