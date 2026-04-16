def test_future_suppressed():  # zorilla: ignore[ZR001]
    values = [1, 2, 3]
    if values:
        assert sum(values) == 6
