class TestBattery:
    def test_many_subjects(self):
        obj = build()
        assert obj.a
        assert self.db.rows
        assert self.log.empty
        assert obj.d
        assert clock.ticks
