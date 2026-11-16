# @karaoke/api-client

Tipos TypeScript gerados a partir do OpenAPI do `music-engine`.

Depois de mudar qualquer rota ou modelo no backend:

```powershell
cd services\music-engine
uv run python scripts/export_openapi.py ../../packages/api-client/openapi.json
cd ..\..
pnpm gen:api
```

`openapi.json` e `src/schema.d.ts` são versionados, para a UI compilar sem subir o backend.
