from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_shipments(request):
    return {"items": STORE.all("shipments"), "count": STORE.count("shipments")}


@require_auth("read")
def get_shipments(request, item_id):
    item = STORE.get("shipments", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_shipments(request):
    item = STORE.add("shipments", request.json)
    return item, 201


@require_auth("write")
def update_shipments(request, item_id):
    if STORE.get("shipments", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("shipments", item_id, request.json)


@require_auth("admin")
def delete_shipments(request, item_id):
    if STORE.get("shipments", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("shipments", item_id)
    return "", 204
