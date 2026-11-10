from shared.page_text import (
    extract_text, looks_blocked, surname_of, window_around_surname,
)


def test_extract_text_drops_markup_scripts_and_styles():
    html = "<html><head><style>.x{}</style><script>var secret=1</script></head><body><p>Hello</p>  <b>world</b><noscript>no</noscript></body></html>"
    assert extract_text(html) == "Hello world"


def test_blocked_pages_are_only_short_ones():
    assert looks_blocked("Please complete the CAPTCHA to continue")
    assert looks_blocked("Access Denied")
    assert not looks_blocked("word " * 1000 + " captcha")
    assert not looks_blocked("Jane Smith, Austin TX")


def test_surname_handling():
    assert surname_of("Jane Smith") == "Smith"
    assert surname_of("Jane Marie Smith Jr.") == "Smith"
    assert surname_of("Cher") == "Cher"
    assert surname_of("") == ""


def test_window_is_centred_on_the_surname_and_keeps_original_case():
    text = "x" * 2000 + " Jane SMITH lives here " + "y" * 5000
    w = window_around_surname(text, "Smith", 1000)
    assert "SMITH" in w and len(w) <= 1000 and w.startswith("x")
    assert window_around_surname("Perfil de Maria Garcia", "García", 100) is not None, "accent-insensitive"
    assert window_around_surname("The Smithson family", "Smith", 100) is None, "whole words only"
    assert window_around_surname("nothing here", "Smith", 100) is None
    assert window_around_surname("anything", "", 100) is None
