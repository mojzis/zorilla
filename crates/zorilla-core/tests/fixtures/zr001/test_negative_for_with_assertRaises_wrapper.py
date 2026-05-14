import unittest


class TestSomething(unittest.TestCase):
    def test_each_case_raises(self):
        cases = [1, 2, 3]
        for case in cases:
            with self.assertRaises(ValueError):
                do_something(case)

    def test_each_case_raises_with_helper(self):
        cases = [1, 2, 3]
        for case in cases:
            with self.assertRaises(ValueError):
                self.assertEqual(case, case)
