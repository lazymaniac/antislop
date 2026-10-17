from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_warehouses(request):
    return {"items": STORE.all("warehouses"), "count": STORE.count("warehouses")}


@require_auth("read")
def get_warehouses(request, item_id):
    item = STORE.get("warehouses", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_warehouses(request):
    item = STORE.add("warehouses", request.json)
    return item, 201


@require_auth("write")
def update_warehouses(request, item_id):
    if STORE.get("warehouses", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("warehouses", item_id, request.json)


@require_auth("admin")
def delete_warehouses(request, item_id):
    if STORE.get("warehouses", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("warehouses", item_id)
    return "", 204
