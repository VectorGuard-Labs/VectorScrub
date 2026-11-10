"""
Structural vocabulary: the only words allowed to reach an LLM from a broker page.

Why a vocabulary instead of only detecting names: name detection (patterns, NER) is
probabilistic. It misses unusual, lowercase, or non-English names. Deny-by-default
inverts the problem: after redaction, any word NOT on this list is dropped, so a
third party's name cannot reach the model unless it happens to be spelled exactly
like one of these words.

Curation rules
  * Words describe the SHAPE of a record or page (labels, status wording, function
    words) so the model can still judge "this listing is about the customer" or
    "no results found".
  * Nothing that is also a common given name or surname. "will", "may", "mark",
    "page", "rose", "hope", "grace", "bill", "lane", "court", "park", ... are kept
    OUT on purpose even though they are ordinary words. When editing this list,
    re-run an intersection against name data from every locale (for example the
    person providers in the Faker library: first_names, last_names for all locales,
    about 49,000 distinct words) and move any collision into NAME_COLLISIONS below.
  * All lowercase, accents folded, apostrophes written as ' .

A side benefit: free text on a hostile page ("ignore previous instructions ...")
is dropped, which shrinks the prompt-injection surface.
"""
from __future__ import annotations

_FUNCTION_WORDS = """
a an the and or but nor so yet of in on at to for from by with without within into onto over under
about above below between among through during before after since until while as than that this
these those there here it its it's he she his her hers him they them their theirs we our ours us
you your yours i me my is are was were be been being am do does did done doing has have had having
not no never also too very more most less least much many few each every any all both either
neither some such only own same other another one two three four five six seven eight nine ten
if then else when where which who whom whose what why how can could should would shall might must
cannot can't couldn't won't don't doesn't isn't aren't wasn't weren't didn't hasn't haven't
yes true false per via etc vs
"""

_RECORD_WORDS = """
record records profile profiles person people persons result results search searches searched
found match matches matched report reports background public private detail details information
info view views listing listings directory directories name names full age aged born birth
birthday date dates dob lives live lived living resides reside resident residents residence
address addresses current currently previous previously past former formerly home homes phone
phones telephone mobile cell cellular wireless landline number numbers email emails mail contact
contacts relatives relative associates associate family members member neighbors neighbours
neighbor neighbour known aka alias aliases marital status married single divorced widowed
spouse household employment employer employed work works worked job occupation education school
college university degree property properties owner owners owned ownership value sold sale
purchase criminal arrest arrests charge charges offense offenses civil judgment judgments
bankruptcy lien liens vehicle vehicles license licenses social media income estimated
associated possible potential likely related location locations city state county area zip code
postal street avenue road line unit apartment apt suite floor box
male female gender
"""

_PAGE_WORDS = """
sorry oops error errors unable unavailable available exist exists existed longer moved expired
invalid request requests submitted submit submission confirmation confirm confirmed thank thanks
please try again later removed remove removal removals delete deleted deletion opt opted out
suppress suppressed suppression optout privacy policy terms conditions service services copyright
rights reserved cookie cookies accept decline agree login log sign signin signup register account
help support faq blog careers press about menu click tap more show hide see learn read free
trial subscription subscribe unlock download app site website web online data broker brokers
update updated updates last year years month months day days week weeks hour hours minute minutes
ago today yesterday now complete completed pending processing process verify verified
verification exact approximate loading wait redirect redirected redirecting continue skip
next prev previous back top bottom left right
"""


def _words(block: str) -> set[str]:
    return {w for w in block.split() if w}


# Words removed after the name-corpus intersection test showed they are also given names
# or surnames in at least one locale (Faker's person data for every locale it ships:
# ~49,000 distinct name words). Mostly short English function words that double as
# surnames elsewhere ("Do", "An", "Mo"...). Dropping them is cosmetic for the model
# (runs of dropped words collapse into "…"); keeping them would break the guarantee
# that no allowed word is a known name. Re-run the intersection when editing the lists.
NAME_COLLISIONS: frozenset[str] = frozenset({
    "age", "am", "among", "an", "as", "back", "can", "date", "day", "do", "each", "else",
    "floor", "hide", "job", "lien", "line", "more", "must", "my", "over", "per", "read",
    "seven", "street", "the", "too", "us", "weeks", "were",
})

ALLOWED_WORDS: frozenset[str] = frozenset(
    _words(_FUNCTION_WORDS) | _words(_RECORD_WORDS) | _words(_PAGE_WORDS)
) - NAME_COLLISIONS

# Placeholder tokens the redactor emits; the only bracketed tokens that may appear.
KNOWN_TOKENS: frozenset[str] = frozenset({
    "CUSTOMER_NAME", "CUSTOMER_ADDRESS", "CUSTOMER_PHONE", "CUSTOMER_EMAIL", "CUSTOMER_DOB",
    "SURNAME", "FIRST_NAME", "NAME", "ADDRESS", "PHONE", "EMAIL", "DATE", "AGE", "ZIP", "SSN",
    "NUMBER",
})
