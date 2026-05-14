def test_reads_files():
    with open("a") as a:
        with open("b") as b:
            with open("c") as c:
                with open("d") as d:
                    assert a and b and c and d
