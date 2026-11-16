"""Autenticação por token: só quem foi iniciado pelo Tauri conhece o token."""

from __future__ import annotations

import secrets
from typing import Annotated

from fastapi import Depends, Header, HTTPException, Request, status

from .config import TOKEN_HEADER, Settings


def get_settings(request: Request) -> Settings:
    return request.app.state.settings


def require_token(
    settings: Annotated[Settings, Depends(get_settings)],
    token: Annotated[str | None, Header(alias=TOKEN_HEADER)] = None,
) -> None:
    if token is None or not secrets.compare_digest(token, settings.token):
        raise HTTPException(status.HTTP_401_UNAUTHORIZED, "Token inválido ou ausente.")
