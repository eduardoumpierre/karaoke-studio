"""Leitura de letras no formato LRC: `[mm:ss.xx] texto`."""

from __future__ import annotations

import re
from dataclasses import dataclass

_TIME = re.compile(r"\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]")
_META = re.compile(r"^\[([a-zA-Z#]+):(.*)\]\s*$")
_WORD_TAG = re.compile(r"<\d{1,3}:\d{2}(?:[.:]\d{1,3})?>")


@dataclass(frozen=True)
class Line:
    time_ms: int
    text: str


def _to_ms(minutes: str, seconds: str, fraction: str | None) -> int:
    frac = 0
    if fraction:
        frac = int(fraction.ljust(3, "0")[:3])  # "5" = 500 ms, "50" = 500 ms, "500" = 500 ms
    return (int(minutes) * 60 + int(seconds)) * 1000 + frac


def parse_lrc(text: str) -> list[Line]:
    """Linhas com tempo, ordenadas. Aceita várias marcas por linha (refrão repetido), a marca
    `[offset:+-ms]` do próprio arquivo e ignora metadados ([ar:], [ti:]...) e tags por palavra."""
    lines: list[Line] = []
    offset = 0
    for raw in text.splitlines():
        raw = raw.strip()
        if not raw:
            continue
        meta = _META.match(raw)
        if meta and not _TIME.match(raw):
            if meta.group(1).lower() == "offset":
                try:
                    offset = int(meta.group(2).strip())
                except ValueError:
                    pass
            continue
        stamps = list(_TIME.finditer(raw))
        if not stamps:
            continue
        content = _WORD_TAG.sub("", raw[stamps[-1].end() :]).strip()
        for m in stamps:
            lines.append(Line(_to_ms(*m.groups()), content))
    # No LRC, offset positivo significa que a letra deve aparecer ANTES.
    result = [Line(max(0, ln.time_ms - offset), ln.text) for ln in lines]
    result.sort(key=lambda ln: ln.time_ms)  # estável: mantém a ordem de linhas no mesmo instante
    return result


def looks_synced(text: str) -> bool:
    return len(parse_lrc(text)) >= 2


def strip_to_plain(text: str) -> str:
    """Letra sem tempos nem metadados (para exibir como texto corrido)."""
    out: list[str] = []
    for raw in text.splitlines():
        raw = raw.rstrip()
        if _META.match(raw.strip()) and not _TIME.match(raw.strip()):
            continue
        out.append(_WORD_TAG.sub("", _TIME.sub("", raw)).strip())
    return "\n".join(out).strip()
