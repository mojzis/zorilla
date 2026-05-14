def test_klice_style_assign_then_assert():
    months = ["2026-01", "2026-02"]
    for month in months:
        s = compute(month)
        assert s == 0


def test_augmented_assignment_then_assert():
    items = [1, 2, 3]
    for item in items:
        total = 0
        total += item
        assert total >= 0


def test_assert_then_assign_then_assert():
    cases = [1, 2, 3]
    for case in cases:
        assert case > 0
        normalized = case * 2
        assert normalized > 0
