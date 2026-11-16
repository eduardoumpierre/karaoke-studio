"""Cliente do LRCLIB (https://lrclib.net): letras sincronizadas gratuitas, sem chave de API."""

from __future__ import annotations

import json
import re
import unicodedata
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass

BASE_URL = "https://lrclib.net/api"
USER_AGENT = "KaraokeStudio/0.1 (uso pessoal)"
DURATION_TOLERANCE_S = 3.0

# fetch(url) -> (status HTTP, corpo). Levanta OfflineError se não houver rede.
Fetch = Callable[[str], tuple[int, bytes]]


class OfflineError(Exception):
    """Sem conexão com o LRCLIB."""


def default_fetch(url: str) -> tuple[int, bytes]:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:  # noqa: S310 (URL fixa, https)
            return resp.status, resp.read()
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read()
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        raise OfflineError(str(exc)) from exc


@dataclass(frozen=True)
class Found:
    synced: str | None
    plain: str | None
    title: str
    artist: str
    duration: float | None


def _fold(text: str) -> str:
    d = unicodedata.normalize("NFKD", text.casefold())
    return "".join(c for c in d if not unicodedata.combining(c)).strip()


_NOISE_WORDS = (
    r"official|oficial|audio|áudio|video|vídeo|lyric|lyrics|letra|legendado|hd|hq|4k|1080p|"
    r"remaster(?:ed)?|visuali[sz]er|clipe?|ao vivo|live|music video|mv|explicit"
)
_BRACKET_NOISE = re.compile(rf"\s*[\(\[][^)\]]*\b(?:{_NOISE_WORDS})\b[^)\]]*[\)\]]", re.IGNORECASE)


def clean_title(title: str, artist: str) -> tuple[str, str]:
    """Tira o "lixo" típico de nomes de arquivo/YouTube: "Artista - Título (Official Audio)"."""
    title = _BRACKET_NOISE.sub("", title)
    title = title.split(" | ")[0].strip()
    artist = re.sub(r"\s*-\s*topic$|vevo$", "", artist.strip(), flags=re.IGNORECASE).strip()
    if " - " in title:
        left, right = title.split(" - ", 1)
        if not artist:
            artist, title = left.strip(), right.strip()
        elif _fold(left) == _fold(artist):
            title = right.strip()
    return title.strip(), artist


def _clean_lyrics(value: object) -> str | None:
    return value.strip() if isinstance(value, str) and value.strip() else None


def _to_found(item: dict) -> Found | None:
    if not isinstance(item, dict) or item.get("instrumental"):
        return None
    synced = _clean_lyrics(item.get("syncedLyrics"))
    plain = _clean_lyrics(item.get("plainLyrics"))
    if not synced and not plain:
        return None
    duration = item.get("duration")
    return Found(
        synced=synced,
        plain=plain,
        title=str(item.get("trackName") or ""),
        artist=str(item.get("artistName") or ""),
        duration=float(duration) if isinstance(duration, int | float) else None,
    )


def pick_best(items: list[dict], duration: float) -> Found | None:
    """Prefere letra sincronizada com a duração mais próxima (e dentro da tolerância)."""
    candidates = [f for f in (_to_found(i) for i in items) if f]

    def distance(f: Found) -> float:
        return abs(f.duration - duration) if f.duration is not None else 999.0

    def rank(f: Found) -> tuple[int, float]:
        return (0 if f.synced else 1, distance(f))

    close = [f for f in candidates if distance(f) <= DURATION_TOLERANCE_S]
    pool = close or [f for f in candidates if f.duration is None]
    return min(pool, key=rank) if pool else None


class LrclibClient:
    def __init__(self, fetch: Fetch = default_fetch) -> None:
        self._fetch = fetch

    def _json(self, path: str, **params: str) -> object:
        url = f"{BASE_URL}/{path}?{urllib.parse.urlencode(params)}"
        status, body = self._fetch(url)
        if status == 404:
            return None
        if status != 200:
            raise OfflineError(f"LRCLIB respondeu {status}")
        try:
            return json.loads(body)
        except ValueError as exc:
            raise OfflineError("resposta inválida do LRCLIB") from exc

    def find(self, title: str, artist: str, album: str, duration: float) -> Found | None:
        """Busca exata (com duração) e, se falhar, busca ampla filtrada pela duração."""
        if title and artist:
            exact = self._json(
                "get",
                track_name=title,
                artist_name=artist,
                album_name=album,
                duration=str(round(duration)),
            )
            if isinstance(exact, dict):
                found = pick_best([exact], duration)
                if found:
                    return found
        params = {"track_name": title}
        if artist:
            params["artist_name"] = artist
        results = self._json("search", **params)
        if isinstance(results, list):
            found = pick_best(results, duration)
            if found:
                return found
        if artist:  # último recurso: busca livre
            results = self._json("search", q=f"{artist} {title}")
            if isinstance(results, list):
                return pick_best(results, duration)
        return None
