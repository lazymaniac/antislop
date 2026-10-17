from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_refunds(request):
    return {"items": STORE.all("refunds"), "count": STORE.count("refunds")}


@require_auth("read")
def get_refunds(request, item_id):
    item = STORE.get("refunds", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_refunds(request):
    item = STORE.add("refunds", request.json)
    return item, 201


@require_auth("write")
def update_refunds(request, item_id):
    if STORE.get("refunds", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("refunds", item_id, request.json)


def delete_refunds(request, item_id):
    if STORE.get("refunds", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("refunds", item_id)
    return "", 204
