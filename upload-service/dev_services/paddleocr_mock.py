#!/usr/bin/env python3
"""
Lightweight PaddleOCR mock service for local development.

API:
  GET /health -> {"status": "ok"}
  POST /ocr   -> {"success": true, "code": 0, "message": "...", "data": {...}}

The /ocr endpoint accepts multipart/form-data with a `file` field. The mock
service performs a best-effort UTF-8 decode of the bytes and returns simple
statistics so that upload-service can continue its document pipeline while
the real OCR service is unavailable.
"""

from __future__ import annotations

import asyncio
from typing import List

from fastapi import FastAPI, File, UploadFile
from fastapi.responses import JSONResponse


app = FastAPI(title="Mock PaddleOCR Service", version="0.1.0")


def _decode_text(data: bytes) -> str:
    if not data:
        return ""
    # Try UTF-8 first. If that fails, replace undecodable bytes.
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return data.decode("utf-8", errors="replace")


def _split_lines(text: str, max_lines: int = 200) -> List[str]:
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    if len(lines) > max_lines:
        return lines[:max_lines]
    return lines


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.post("/ocr")
async def ocr(file: UploadFile = File(...)):
    data = await file.read()
    text = _decode_text(data)
    lines = _split_lines(text)

    response = {
        "success": True,
        "code": 0,
        "message": "Mock OCR success",
        "data": {
            "text": text,
            "text_length": len(text),
            "line_count": len(lines),
            "lines": lines,
        },
    }
    return JSONResponse(response)


def main():
    import uvicorn

    uvicorn.run(
        "dev_services.paddleocr_mock:app",
        host="0.0.0.0",
        port=8080,
        reload=False,
        log_level="info",
    )


if __name__ == "__main__":
    try:
        asyncio.run(asyncio.sleep(0))
    except RuntimeError:
        # Event loop already running (e.g., in some IDEs); fall back to uvicorn.
        pass
    main()

