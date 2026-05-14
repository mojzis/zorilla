import unittest


class TestSomething(unittest.TestCase):
    def test_each_case(self):
        cases = [1, 2, 3]
        for case in cases:
            self.assertEqual(case, case)

    def test_each_in_list(self):
        items = [1, 2, 3]
        for item in items:
            self.assertIn(item, items)
