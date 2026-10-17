import unittest

from shipping import shipping_cost


class ShippingTest(unittest.TestCase):
    def test_base(self):
        self.assertEqual(shipping_cost(2, False, False, False), 7)

    def test_express(self):
        self.assertEqual(shipping_cost(2, True, False, False), 14)

    def test_international(self):
        self.assertEqual(shipping_cost(2, False, True, False), 22)

    def test_member(self):
        self.assertEqual(shipping_cost(2, False, False, True), 3.5)

    def test_express_international(self):
        self.assertEqual(shipping_cost(2, True, True, False), 29)

    def test_express_member(self):
        self.assertEqual(shipping_cost(2, True, False, True), 7)

    def test_international_member(self):
        self.assertEqual(shipping_cost(2, False, True, True), 11)

    def test_heavy(self):
        self.assertEqual(shipping_cost(10, False, False, False), 15)


if __name__ == "__main__":
    unittest.main()
