def test_reads_home():
    data = open("~/notes.md").read()
    assert data
