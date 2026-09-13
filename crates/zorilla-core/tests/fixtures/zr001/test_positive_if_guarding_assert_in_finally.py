def test_cleanup(path):
    try:
        run(path)
    finally:
        if path.exists():
            assert path.read_text() == "ok"
            path.unlink()
