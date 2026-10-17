class Store:
    def __init__(self):
        self.tables = {}

    def all(self, table):
        return list(self.tables.get(table, {}).values())

    def count(self, table):
        return len(self.tables.get(table, {}))

    def get(self, table, item_id):
        return self.tables.get(table, {}).get(item_id)

    def add(self, table, item):
        rows = self.tables.setdefault(table, {})
        item = dict(item, id=len(rows) + 1)
        rows[item["id"]] = item
        return item

    def update(self, table, item_id, changes):
        self.tables[table][item_id].update(changes)
        return self.tables[table][item_id]

    def delete(self, table, item_id):
        del self.tables[table][item_id]


STORE = Store()
