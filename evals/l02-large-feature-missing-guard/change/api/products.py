from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_products(request):
    return {"items": STORE.all("products"), "count": STORE.count("products")}


@require_auth("read")
def get_products(request, item_id):
    item = STORE.get("products", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_products(request):
    item = STORE.add("products", request.json)
    return item, 201


@require_auth("write")
def update_products(request, item_id):
    if STORE.get("products", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("products", item_id, request.json)


@require_auth("admin")
def delete_products(request, item_id):
    if STORE.get("products", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("products", item_id)
    return "", 204
