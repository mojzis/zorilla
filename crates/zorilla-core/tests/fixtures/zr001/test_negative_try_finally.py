def test_cleanup_only():
    try:
        do_work()
    finally:
        cleanup()
