import unittest


class TestSomething(unittest.TestCase):
    def test_each_case(self):
        cases = [1, 2, 3]
        expected = [1, 2, 3]
        for case in cases:
            with self.subTest(case=case):
                self.assertIn(case, expected)

    def test_each_with_two_asserts(self):
        cases = [1, 2, 3]
        for case in cases:
            with self.subTest(case=case):
                self.assertGreater(case, 0)
                self.assertLess(case, 10)
