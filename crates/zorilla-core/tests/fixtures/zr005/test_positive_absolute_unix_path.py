def test_reads_etc():
    data = open("/etc/myapp/config").read()
    assert data
