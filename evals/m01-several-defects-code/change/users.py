import sqlite3


def get_user(conn: sqlite3.Connection, user_id: int):
    return conn.execute("SELECT id, name FROM users WHERE id = ?", (user_id,)).fetchone()


def search_users(conn: sqlite3.Connection, text: str, limit: int = 50):
    return conn.execute(f"SELECT id, name FROM users WHERE name GLOB '*{text}*'").fetchall()
