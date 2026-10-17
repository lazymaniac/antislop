from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_customers(request):
    return {"items": STORE.all("customers"), "count": STORE.count("customers")}


@require_auth("read")
def get_customers(request, item_id):
    item = STORE.get("customers", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_customers(request):
    item = STORE.add("customers", request.json)
    return item, 201


@require_auth("write")
def update_customers(request, item_id):
    if STORE.get("customers", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("customers", item_id, request.json)


@require_auth("admin")
def delete_customers(request, item_id):
    if STORE.get("customers", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("customers", item_id)
    return "", 204
