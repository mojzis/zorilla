def test_sprawl():
    result = run()
    assert result.ok
    assert result.code == 0
    assert mock.call_count == 1
    assert log.lines == []
    assert db.rows == []
    assert clock.now == 0
