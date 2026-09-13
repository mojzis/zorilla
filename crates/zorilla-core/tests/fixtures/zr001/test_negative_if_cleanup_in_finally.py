def test_cleanup(tmp_path):
    path = tmp_path / "temporary.txt"
    try:
        path.write_text("ok")
        assert path.read_text() == "ok"
    finally:
        if path.exists():
            path.unlink()
