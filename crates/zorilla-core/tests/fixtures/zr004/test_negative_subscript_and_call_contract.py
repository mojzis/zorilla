def test_rollup_contract(conn, verdict):
    row = conn.execute("SELECT n_requests, n_gaps, cost FROM rollup").fetchone()
    assert row[0] == verdict.n_requests
    assert row[1] == verdict.n_gaps
    assert float(row[2]) == pytest.approx(verdict.cost)
    assert len(row) == 3
    assert row.count(None) == 0
    assert isinstance(row, tuple)
