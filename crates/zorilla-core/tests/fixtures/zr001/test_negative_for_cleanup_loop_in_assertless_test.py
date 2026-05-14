def test_env():
    # Mis-named fixture: function starts with `test_` but does no asserting,
    # uses a cleanup loop to tear down state. Should not fire ZR001.
    import os

    yield
    for key in ["SECRET_KEY", "GOOGLE_CLIENT_ID", "DEBUG"]:
        os.environ.pop(key, None)


def test_cleanup_with_calls_and_assignments():
    paths = []
    for p in paths:
        unlink(p)
        marker = compute(p)
