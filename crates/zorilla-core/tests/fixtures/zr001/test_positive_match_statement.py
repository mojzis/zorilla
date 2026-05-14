def test_top_match():
    match x:
        case 1:
            assert x == 1
        case _:
            assert False
