#!/usr/bin/env python3
"""
Minimal pyresparser mock service for local development.

API:
  GET  /health -> {"status": "ok"}
  POST /parse  -> {"success": true, "code": 0, "message": "...", "data": {...}}

The /parse endpoint accepts either multipart/form-data (field name `file`) or a
JSON body `{"text": "..."}`.
"""

from __future__ import annotations

import json
import uuid
from datetime import datetime
from typing import Optional

from fastapi import Body, FastAPI, File, Request, UploadFile
from fastapi.responses import JSONResponse


app = FastAPI(title="Mock PyResParser Service", version="0.1.0")


def _normalize_text(raw: bytes, fallback: str = "") -> str:
    if not raw:
        return fallback
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        return raw.decode("utf-8", errors="replace")


def _build_response(text: str, filename: Optional[str]) -> dict:
    skills = []
    if text:
        # naive skill extraction: words longer than 3 characters
        tokens = {token.strip(",. ") for token in text.split() if len(token) > 3}
        skills = sorted(list(tokens))[:10]

    data = {
        "name": None,
        "email": None,
        "mobile_number": None,
        "skills": skills,
        "education": [],
        "text_length": len(text),
        "filename": filename,
    }
    meta = {
        "processing_time_ms": 5,
        "parser": "mock-pyresparser",
        "version": "0.1.0",
        "libraries": ["fastapi"],
    }
    return {
        "success": True,
        "code": 0,
        "message": "Mock parse success",
        "data": data,
        "meta": meta,
        "trace_id": str(uuid.uuid4()),
        "timestamp": datetime.utcnow().isoformat() + "Z",
    }


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.post("/parse")
async def parse(
    request: Request,
    file: UploadFile = File(None),
    json_body: Optional[dict] = Body(None),
):
    text = ""
    filename = None

    if file is not None:
        payload = await file.read()
        filename = file.filename
        text = _normalize_text(payload)
    elif json_body and "text" in json_body:
        text = str(json_body.get("text") or "")
    else:
        # Attempt to load raw body as JSON (for curl w/o explicit content-type)
        raw = await request.body()
        if raw:
            try:
                data = json.loads(raw.decode("utf-8"))
                text = str(data.get("text") or "")
            except Exception:
                text = _normalize_text(raw)

    response = _build_response(text, filename)
    return JSONResponse(response)


def main():
    import uvicorn

    uvicorn.run(
        "dev_services.pyresparser_mock:app",
        host="0.0.0.0",
        port=5001,
        reload=False,
        log_level="info",
    )


if __name__ == "__main__":
    main()

