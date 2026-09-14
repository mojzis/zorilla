from types import SimpleNamespace


def test_record_contract():
    record = SimpleNamespace(
        name="demo", enabled=True, count=2, tags=[], mode="safe", owner=None
    )
    assert record.name == "demo"
    assert record.enabled is True
    assert record.count == 2
    assert record.tags == []
    assert record.mode == "safe"
    assert record.owner is None
