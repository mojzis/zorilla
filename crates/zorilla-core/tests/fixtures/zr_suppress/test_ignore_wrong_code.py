def test_wrong_code_does_not_suppress():
    if True:  # zorilla: ignore[ZR002]
        assert True
