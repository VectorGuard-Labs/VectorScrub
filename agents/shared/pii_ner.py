"""
Named-entity recognition for third-party names (relatives, neighbours, namesakes).

Role in the PII pipeline (see shared/pii_guard.py):
  1. pattern rules redact the customer's own values and obvious PII formats;
  2. THIS module finds person names the patterns would miss, so they become [NAME];
  3. a deny-by-default vocabulary filter then drops every remaining word that is not
     structural vocabulary. That filter, not NER, is what makes the third-party
     guarantee hold: NER is probabilistic and the small model misses names (it can
     miss "María García" in a capitalized sentence and lowercase "bob" in prose).
     NER earns its place by preserving structure ("Relatives: [NAME], [NAME]") for
     the model instead of leaving a row of "…".

Casing: the model is case-sensitive, so each text is also run through a title-cased
and a lowercased copy of identical length, and the PERSON spans are unioned. Spans
from any variant index the original text.

Everything runs in-process on CPU. No text is sent anywhere for NER.
"""
from __future__ import annotations

import re
import threading

import structlog

from shared.pii_guard import OutboundPiiError

log = structlog.get_logger(__name__)


class NerUnavailableError(OutboundPiiError):
    """
    The NER model could not be loaded and the deployment requires it. Raised before
    any LLM call, so the call is not made (fail closed).
    """


def _map_chars(text: str, fn) -> str:
    """Apply fn per character, keeping a character unchanged if fn would change the length."""
    out = []
    for c in text:
        r = fn(c)
        out.append(r if len(r) == 1 else c)
    return "".join(out)


def _title_variant(text: str) -> str:
    """Uppercase the first letter of every word; same length as the input."""
    chars = list(text)
    for m in re.finditer(r"(?<![^\W\d_])[^\W\d_]", text):
        up = chars[m.start()].upper()
        if len(up) == 1:
            chars[m.start()] = up
    return "".join(chars)


def _variants(text: str) -> list[str]:
    seen: list[str] = []
    for v in (text, _title_variant(text), _map_chars(text, str.lower)):
        if v not in seen:
            seen.append(v)
    return seen


class PersonDetector:
    """Finds PERSON entities with spaCy. Loads lazily; safe to share across threads."""

    def __init__(self, model: str = "en_core_web_sm") -> None:
        self.model = model
        self._nlp = None
        self._load_error: str | None = None
        self._lock = threading.Lock()

    def ensure_loaded(self) -> None:
        """Load the model once. Raises NerUnavailableError if it cannot be loaded."""
        if self._nlp is not None:
            return
        with self._lock:
            if self._nlp is not None:
                return
            if self._load_error is not None:
                raise NerUnavailableError(self._load_error)
            try:
                import spacy  # imported here so deployments without NER can still import the package

                # Only the NER component is needed. tok2vec feeds it; the rest are skipped.
                self._nlp = spacy.load(
                    self.model, exclude=["tagger", "attribute_ruler", "lemmatizer", "parser"]
                )
            except Exception as exc:  # ImportError, OSError (model not installed), ...
                self._load_error = (
                    f"NER model '{self.model}' is not available ({type(exc).__name__}); "
                    "install spacy and the model from requirements.txt, or set "
                    "PII_NER_REQUIRED=false to run on the vocabulary filter alone"
                )
                log.error("pii_ner_unavailable", model=self.model, error_type=type(exc).__name__)
                raise NerUnavailableError(self._load_error) from exc

    @property
    def available(self) -> bool:
        try:
            self.ensure_loaded()
            return True
        except NerUnavailableError:
            return False

    def person_spans(self, text: str) -> list[tuple[int, int]]:
        """(start, end) character offsets of PERSON entities in `text`."""
        self.ensure_loaded()
        spans: set[tuple[int, int]] = set()
        for variant in _variants(text):
            with self._lock:  # spaCy pipelines are not guaranteed thread-safe
                doc = self._nlp(variant)
            for ent in doc.ents:
                if ent.label_ != "PERSON":
                    continue
                start, end = ent.start_char, ent.end_char
                chunk = text[start:end]
                start += len(chunk) - len(chunk.lstrip())
                end -= len(chunk) - len(chunk.rstrip())
                if end > start:
                    spans.add((start, end))
        return sorted(spans)
