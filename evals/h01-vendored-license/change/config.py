from third_party.fastjson import loads


def load(path):
    with open(path) as handle:
        return loads(handle.read())
