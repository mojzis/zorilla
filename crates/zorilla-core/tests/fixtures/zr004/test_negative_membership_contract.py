def test_report_contract(result):
    assert "--- Tokens & cost ---" in result
    assert "Cost: $0.18" in result
    assert "output $0.06" in result
    assert "cache read $0.05" in result
    assert "Tokens: 118,500 total" in result
    assert "input 1,500" in result
    assert "sidechain" not in result
