class TestResult:
    def test_result_contract(self):
        assert self.result.name == "demo"
        assert self.result.enabled is True
        assert self.result.count == 2
        assert self.result.tags == []
        assert self.result.owner is None
