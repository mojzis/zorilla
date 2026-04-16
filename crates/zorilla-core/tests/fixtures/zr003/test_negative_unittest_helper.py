class TestUnitStyle:
    def test_equal(self):
        self.assertEqual(compute(), 1)

    def test_true(self):
        self.assertTrue(flag())

    def test_fail_helper(self):
        self.fail("oops")
