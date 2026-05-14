import unittest


class TestSomething(unittest.TestCase):
    def test_each_case_with_branching_inside_with(self):
        cases = [1, 2, 3]
        for case in cases:
            with self.subTest(case=case):
                if case > 0:
                    self.assertEqual(case, case)
