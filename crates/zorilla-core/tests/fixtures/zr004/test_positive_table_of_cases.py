def test_table():
    assert classify(1) == "one"
    assert classify(2) == "two"
    assert classify(3) == "three"
    assert classify(4) == "four"
    assert classify(5) == "five"
