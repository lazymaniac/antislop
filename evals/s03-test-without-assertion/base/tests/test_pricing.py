import unittest
from pricing import apply_discount


class DiscountTest(unittest.TestCase):
    def test_half_off(self):
        self.assertEqual(apply_discount(10, 50), 5)


if __name__ == "__main__":
    unittest.main()
