def test_reads_fixture():
    data = open("fixtures/sample.json").read()
    assert data
