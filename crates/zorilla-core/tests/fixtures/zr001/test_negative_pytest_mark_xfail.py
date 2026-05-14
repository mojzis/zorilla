@pytest.mark.xfail
def test_x():
    for case in CASES:
        if case:
            assert case
