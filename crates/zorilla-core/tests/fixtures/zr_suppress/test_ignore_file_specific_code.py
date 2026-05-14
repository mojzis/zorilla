# zorilla: ignore-file[ZR005]


def test_reads_hosts():
    data = open("/etc/hosts").read()
    assert data


def test_empty():
    pass
