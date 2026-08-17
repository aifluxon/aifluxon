# Adding a Python API

Checklist:

- [ ] Rust stable API exists in `aifluxon-api`
- [ ] Python wrapper implemented
- [ ] `__all__` updated
- [ ] typing / `.pyi` updated if needed
- [ ] exception mapping updated
- [ ] `docs/python/api-reference.md` updated
- [ ] quickstart/example updated if relevant
- [ ] tests added
- [ ] docs consistency test passes

## Rules

1. Put new orchestration seams in `aifluxon-api`, not in `aifluxon-runtime` imports from Python.
2. Convert PyO3 values in `bindings/python/src/lib.rs` only when the Python package cannot express the call.
3. Keep user types in `python/aifluxon/`.
4. Never expose `_native.NativeAgent` as a supported user API.
5. Do not document EasyPhy Default/Managed/Trusted as the generic Python policy model.
