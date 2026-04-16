class TestBattery:
    def test_many_fields(self):
        obj = build()
        assert obj.a
        assert obj.b
        assert obj.c
        assert obj.d
        assert obj.e
