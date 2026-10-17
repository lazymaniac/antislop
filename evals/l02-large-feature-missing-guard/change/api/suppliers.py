from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_suppliers(request):
    return {"items": STORE.all("suppliers"), "count": STORE.count("suppliers")}


@require_auth("read")
def get_suppliers(request, item_id):
    item = STORE.get("suppliers", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_suppliers(request):
    item = STORE.add("suppliers", request.json)
    return item, 201


@require_auth("write")
def update_suppliers(request, item_id):
    if STORE.get("suppliers", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("suppliers", item_id, request.json)


@require_auth("admin")
def delete_suppliers(request, item_id):
    if STORE.get("suppliers", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("suppliers", item_id)
    return "", 204
