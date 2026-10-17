from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_returns(request):
    return {"items": STORE.all("returns"), "count": STORE.count("returns")}


@require_auth("read")
def get_returns(request, item_id):
    item = STORE.get("returns", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_returns(request):
    item = STORE.add("returns", request.json)
    return item, 201


@require_auth("write")
def update_returns(request, item_id):
    if STORE.get("returns", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("returns", item_id, request.json)


@require_auth("admin")
def delete_returns(request, item_id):
    if STORE.get("returns", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("returns", item_id)
    return "", 204
