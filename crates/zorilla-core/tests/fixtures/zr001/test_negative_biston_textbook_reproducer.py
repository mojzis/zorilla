class TestTemplates:
    def test_templates_are_long_enough(self):
        _TEMPLATES = ["a\nb\n", "c\nd\n"]
        for i, template in enumerate(_TEMPLATES):
            lines = template.strip().count("\n") + 1
            assert lines >= 10, f"Template {i} has only {lines} lines"
