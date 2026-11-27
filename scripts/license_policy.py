"""GPLv3 compatibility policy for dependency license expressions.

EOStage is distributed under GPL-3.0-or-later. Dependencies must be compatible with
GPLv3; a GPL-3.0-only dependency is compatible because the combined work can be
distributed under GPLv3. This module classifies an SPDX-like license
expression, as found in Cargo metadata or an npm package-lock, into one of
three verdicts:

* ``compatible``   -- the dependency may be combined into the GPLv3 work;
* ``incompatible`` -- every alternative carries terms GPL-3.0 does not allow;
* ``needs_review`` -- the expression is empty, unparsable or uses a license
  identifier this policy does not know.

``OR`` is compatible when at least one alternative is compatible; ``AND`` is
compatible only when every part is. ``WITH <exception>`` with a known
permission-granting exception keeps the verdict of the base license; an
unknown exception needs review, and a restriction disguised as an exception
(an incompatible id or ``LicenseRef-*``) is incompatible. Cargo's legacy
``MIT/Apache-2.0`` slash form is read as ``OR``. The classification is engineering metadata for a
merge gate, not legal approval.
"""

from __future__ import annotations

import re
from collections.abc import Iterable

COMPATIBLE = "compatible"
INCOMPATIBLE = "incompatible"
NEEDS_REVIEW = "needs_review"

# Identifiers are compared case-insensitively. A trailing "+" (deprecated SPDX
# "or later" form) is handled in ``classify_license_id``.
COMPATIBLE_LICENSE_IDS = frozenset(
    name.casefold()
    for name in (
        "0BSD",
        "Apache-2.0",
        "Artistic-2.0",
        "BlueOak-1.0.0",
        "BSD-1-Clause",
        "BSD-2-Clause",
        "BSD-2-Clause-Patent",
        "BSD-3-Clause",
        "BSD-3-Clause-Clear",
        # BSD-3-Clause plus a non-restrictive clause licensing voluntarily
        # contributed enhancements back to the authors (e.g. HDF5).
        "BSD-3-Clause-LBNL",
        "BSL-1.0",
        "bzip2-1.0.6",
        "CC0-1.0",
        # Permissive (attribution-only) data license used by webpki-roots.
        "CDLA-Permissive-2.0",
        "ISC",
        "LGPL-2.0-only",
        "LGPL-2.0-or-later",
        "LGPL-2.1-only",
        "LGPL-2.1-or-later",
        "LGPL-3.0-only",
        "LGPL-3.0-or-later",
        "LGPL-2.0",
        "LGPL-2.1",
        "LGPL-3.0",
        "GPL-2.0-or-later",
        "GPL-3.0",
        "GPL-3.0-only",
        "GPL-3.0-or-later",
        "MIT",
        "MIT-0",
        "MPL-2.0",
        "NCSA",
        "PSF-2.0",
        "Python-2.0",
        "Unicode-3.0",
        "Unicode-DFS-2016",
        "Unlicense",
        "UPL-1.0",
        "W3C",
        "WTFPL",
        "X11",
        "Zlib",
        "zlib-acknowledgement",
    )
)

INCOMPATIBLE_LICENSE_IDS = frozenset(
    name.casefold()
    for name in (
        "Apache-1.0",
        "Apache-1.1",
        "BSD-4-Clause",
        "CDDL-1.0",
        "CDDL-1.1",
        "CPAL-1.0",
        "CPL-1.0",
        "EPL-1.0",
        "EPL-2.0",
        "EUPL-1.0",
        "EUPL-1.1",
        "GPL-1.0",
        "GPL-1.0-only",
        "GPL-2.0",
        "GPL-2.0-only",
        "JSON",
        "MPL-1.0",
        "MPL-1.1",
        "OpenSSL",
        "Proprietary",
        "SSLeay",
        "UNLICENSED",
    )
)

# Families matched by prefix (case-insensitive).
INCOMPATIBLE_LICENSE_PREFIXES = tuple(
    prefix.casefold()
    for prefix in (
        "Hippocratic-",
        "SSPL-",
        "BUSL-",
        "CC-BY-NC",
        "CC-BY-ND",
        "Elastic-",
        "Commons-Clause",
        "PolyForm-",
    )
)

_TOKEN = re.compile(r"\s*(\(|\)|/|[^\s()/]+)")
_OPERATORS = {"and", "or", "with"}

# ``WITH`` exceptions known to only grant additional permissions; with one of
# these the expression keeps the verdict of its base license.
PERMISSION_EXCEPTION_IDS = frozenset(
    name.casefold()
    for name in (
        "LLVM-exception",
        "Classpath-exception-2.0",
        "GCC-exception-2.0",
        "GCC-exception-3.1",
        "Bison-exception-2.2",
        "Autoconf-exception-3.0",
        "Linux-syscall-note",
        "mif-exception",
    )
)


class LicenseExpressionError(ValueError):
    """The license expression cannot be parsed."""


def classify_exception_id(identifier: str) -> str:
    """Classify the identifier after ``WITH``.

    ``COMPATIBLE`` here means "keeps the base verdict". A restriction smuggled in
    as an exception (an incompatible license id/family or any ``LicenseRef-*``)
    is incompatible; an exception this policy does not know needs review.
    """
    folded = identifier.strip().casefold()
    if folded in PERMISSION_EXCEPTION_IDS:
        return COMPATIBLE
    if (
        folded.startswith("licenseref-")
        or folded.startswith("documentref-")
        or folded in INCOMPATIBLE_LICENSE_IDS
        or any(folded.startswith(prefix) for prefix in INCOMPATIBLE_LICENSE_PREFIXES)
    ):
        return INCOMPATIBLE
    return NEEDS_REVIEW


def classify_license_id(identifier: str, *, license_ref_allowlist: Iterable[str] = ()) -> str:
    """Classify a single license identifier (no operators)."""
    value = identifier.strip()
    folded = value.casefold()
    if not folded:
        return NEEDS_REVIEW
    if folded.startswith("licenseref-") or folded.startswith("documentref-"):
        allowed = {item.casefold() for item in license_ref_allowlist}
        return COMPATIBLE if folded in allowed else INCOMPATIBLE
    if folded.endswith("+"):
        base = folded[:-1]
        # GPL-2.0+ means "GPL-2.0 or later", which includes GPL-3.0.
        if base in {"gpl-2.0", "gpl-3.0", "lgpl-2.0", "lgpl-2.1", "lgpl-3.0"}:
            return COMPATIBLE
        folded = base
    if folded in COMPATIBLE_LICENSE_IDS:
        return COMPATIBLE
    if folded in INCOMPATIBLE_LICENSE_IDS:
        return INCOMPATIBLE
    if any(folded.startswith(prefix) for prefix in INCOMPATIBLE_LICENSE_PREFIXES):
        return INCOMPATIBLE
    return NEEDS_REVIEW


def _tokenize(expression: str) -> list[str]:
    tokens: list[str] = []
    position = 0
    text = expression.strip()
    while position < len(text):
        match = _TOKEN.match(text, position)
        if match is None:
            raise LicenseExpressionError(f"cannot tokenize license expression: {expression!r}")
        tokens.append(match.group(1))
        position = match.end()
        while position < len(text) and text[position].isspace():
            position += 1
    return tokens


class _Parser:
    """Recursive-descent parser; precedence WITH > AND > OR ("/" is OR)."""

    def __init__(self, tokens: list[str], license_ref_allowlist: Iterable[str]) -> None:
        self.tokens = tokens
        self.index = 0
        self.allowlist = tuple(license_ref_allowlist)

    def _peek(self) -> str | None:
        return self.tokens[self.index] if self.index < len(self.tokens) else None

    def _peek_operator(self) -> str | None:
        token = self._peek()
        if token is None:
            return None
        if token == "/":
            return "or"
        folded = token.casefold()
        return folded if folded in _OPERATORS else None

    def parse(self) -> str:
        if not self.tokens:
            raise LicenseExpressionError("empty license expression")
        verdict = self._or()
        if self._peek() is not None:
            raise LicenseExpressionError(f"unexpected token {self._peek()!r}")
        return verdict

    def _or(self) -> str:
        verdicts = [self._and()]
        while self._peek_operator() == "or":
            self.index += 1
            verdicts.append(self._and())
        if COMPATIBLE in verdicts:
            return COMPATIBLE
        if NEEDS_REVIEW in verdicts:
            return NEEDS_REVIEW
        return INCOMPATIBLE

    def _and(self) -> str:
        verdicts = [self._with()]
        while self._peek_operator() == "and":
            self.index += 1
            verdicts.append(self._with())
        if INCOMPATIBLE in verdicts:
            return INCOMPATIBLE
        if NEEDS_REVIEW in verdicts:
            return NEEDS_REVIEW
        return COMPATIBLE

    def _with(self) -> str:
        verdict = self._atom()
        if self._peek_operator() == "with":
            self.index += 1
            exception = self._peek()
            if exception is None or exception in {"(", ")", "/"} or self._peek_operator():
                raise LicenseExpressionError("WITH must be followed by an exception identifier")
            self.index += 1
            exception_verdict = classify_exception_id(exception)
            if exception_verdict != COMPATIBLE:
                return exception_verdict
        return verdict

    def _atom(self) -> str:
        token = self._peek()
        if token is None:
            raise LicenseExpressionError("unexpected end of license expression")
        if token == "(":
            self.index += 1
            verdict = self._or()
            if self._peek() != ")":
                raise LicenseExpressionError("unbalanced parenthesis")
            self.index += 1
            return verdict
        if token in {")", "/"} or self._peek_operator():
            raise LicenseExpressionError(f"unexpected token {token!r}")
        self.index += 1
        return classify_license_id(token, license_ref_allowlist=self.allowlist)


def evaluate_expression(expression: str, *, license_ref_allowlist: Iterable[str] = ()) -> str:
    """Return the GPLv3 compatibility verdict for a license expression."""
    try:
        return _Parser(_tokenize(expression or ""), license_ref_allowlist).parse()
    except LicenseExpressionError:
        return NEEDS_REVIEW
