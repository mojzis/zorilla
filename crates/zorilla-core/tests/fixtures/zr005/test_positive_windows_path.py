def test_reads_windows():
    data = open(r"C:\Users\alice\data.txt").read()
    assert data
