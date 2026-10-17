def require_auth(scope):
    """Rejects a request whose token lacks `scope`."""
    def wrap(handler):
        def checked(request, *args):
            if scope not in request.scopes:
                return {"error": "forbidden"}, 403
            return handler(request, *args)
        return checked
    return wrap
