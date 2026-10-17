import sqlite3
import unittest

from users import get_user


class UsersTest(unittest.TestCase):
    def test_get_user(self):
        conn = sqlite3.connect(":memory:")
        conn.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
        conn.execute("INSERT INTO users (name) VALUES ('Ada')")
        self.assertEqual(get_user(conn, 1), (1, "Ada"))
