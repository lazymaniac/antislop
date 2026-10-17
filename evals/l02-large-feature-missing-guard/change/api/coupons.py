from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_coupons(request):
    return {"items": STORE.all("coupons"), "count": STORE.count("coupons")}


@require_auth("read")
def get_coupons(request, item_id):
    item = STORE.get("coupons", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_coupons(request):
    item = STORE.add("coupons", request.json)
    return item, 201


@require_auth("write")
def update_coupons(request, item_id):
    if STORE.get("coupons", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("coupons", item_id, request.json)


@require_auth("admin")
def delete_coupons(request, item_id):
    if STORE.get("coupons", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("coupons", item_id)
    return "", 204
