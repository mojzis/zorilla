def test_try_with_except():
    try:
        do_work()
    except ValueError:
        handle()
