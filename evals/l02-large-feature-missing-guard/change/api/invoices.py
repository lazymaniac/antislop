from .auth import require_auth
from .store import STORE


@require_auth("read")
def list_invoices(request):
    return {"items": STORE.all("invoices"), "count": STORE.count("invoices")}


@require_auth("read")
def get_invoices(request, item_id):
    item = STORE.get("invoices", item_id)
    if item is None:
        return {"error": "not found"}, 404
    return item


@require_auth("write")
def create_invoices(request):
    item = STORE.add("invoices", request.json)
    return item, 201


@require_auth("write")
def update_invoices(request, item_id):
    if STORE.get("invoices", item_id) is None:
        return {"error": "not found"}, 404
    return STORE.update("invoices", item_id, request.json)


@require_auth("admin")
def delete_invoices(request, item_id):
    if STORE.get("invoices", item_id) is None:
        return {"error": "not found"}, 404
    STORE.delete("invoices", item_id)
    return "", 204
