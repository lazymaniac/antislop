from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_orders(request):
    return {"items": STORE.all("orders"), "count": STORE.count("orders")}


@require_auth("read")
def get_orders(request, item_id):
    item = STORE.get("orders", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_orders(request):
    item = STORE.add("orders", request.json)
    return item, 201


@require_auth("write")
def update_orders(request, item_id):
    if STORE.get("orders", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("orders", item_id, request.json)


@require_auth("admin")
def delete_orders(request, item_id):
    if STORE.get("orders", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("orders", item_id)
    return "", 204
