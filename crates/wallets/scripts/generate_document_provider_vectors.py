import json
from pathlib import Path

from kassiber.ai.client import OpenAICompatibleClient, _names_store
from kassiber.ai.providers import DEFAULT_BOOTSTRAP_PROVIDERS
from kassiber.core import document_import as ref
from kassiber.errors import AppError
from crates.wallets.scripts.generate_document_vectors import error

ROOT = Path(__file__).resolve().parents[1]


def vectors():
    out = []
    installed = [{"id": "text-only"}, {"id": "GLM-OCR"}, {"id": "qwen3-vl:8b"}]
    for default in [None, "text-only", "qwen3-vl:8b", "missing-vision"]:
        for models in [[], installed, [{"id": "llava:latest"}], [{"id": "text-only"}]]:
            for requested in [None, "glm-ocr", " GLM-OCR ", "text-only", "missing-vision", " "]:
                case = dict(operation="choose", provider={"default_model": default}, models=models, requested=requested)
                try:
                    model, ids = ref._choose_model(case["provider"], models, requested)
                    case["result"] = [model, ids]
                except AppError as exc:
                    case["error"] = error(exc)
                out.append(case)
    for text in ["", " ", '{}', '{"rows": []}', '```json\n{"rows":[]}\n```', 'before {"rows":[]} after', '```\n{}\n```', '[]', 'null', '[1]', '{bad}', '{"x":}', '{"rows":[]} {}', '```JSON\n{}\n```', 'ü {"rows":[]}']:
        case = dict(operation="extract", text=text)
        try:
            case["result"] = ref._extract_json_object(text)
        except AppError as exc:
            case["error"] = error(exc)
        out.append(case)
    messages = [{"role": "system", "content": ref.OCR_SYSTEM_PROMPT}, {"role": "user", "content": [
        {"type": "text", "text": "Extract a draft Kassiber transaction table from this local document. Return JSON only and include per-cell confidence plus source regions."},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,ZmFrZQ=="}}]}]
    for wire in ["responses", "chat_completions"]:
        for omit in [False, True]:
            client = OpenAICompatibleClient(base_url="http://localhost:11434/v1", omit_store_field=omit)
            from kassiber.ai.client import responses_request_context
            prepared = client._prepare(wire, context=responses_request_context(messages), model="glm-ocr", stream=False,
                options={"temperature": 0, "response_format": {"type": "json_object"}}, tools=None, tool_choice=None, omit_store=omit)
            out.append(dict(operation="body", wire=wire, omit=omit, result=prepared.body))
    for wire in ["responses", "chat_completions"]:
        for payload in [None, [], {}, {"error": {}}, {"error": "invalid request"},
                {"error": {"code": 400, "message": "bad prompt"}},
                {"error": {"code": 429, "message": "limited"}},
                {"error": {"code": "invalid_api_key"}},
                {"object": "error", "code": 400, "message": "invalid"},
                {"choices": [{"message": {"content": [{"type": "text", "text": "{}"}], "refusal": ""}}]},
                {"output": [{"type": "message", "content": [{"type": "output_text", "text": "{}"}]}]}]:
            case = dict(operation="content", wire=wire, payload=payload)
            try:
                if not isinstance(payload, dict):
                    raise AppError("AI provider returned an invalid response payload", code="ai_request_invalid", retryable=False)
                result = client._responses_result(payload) if wire == "responses" else client._chat_completions_result(payload)
                case["result"] = result["content"]
            except AppError as exc:
                case["error"] = error(exc)
            out.append(case)
    for total in [1, 8, 9, 20]:
        for maximum in [1, 8]:
            for pages in [None, "", "1", "2-6", "9-16", "0", "2-1", "1,2", "١-٢", " 2 - 3 ", " ", "1\u001c-\u001d2", False, 2]:
                case = dict(operation="pages", total=total, maximum=maximum, pages=pages)
                try:
                    selected, explicit = ref._selected_pdf_pages(pages, total_pages=total, max_pages=maximum)
                    case["result"] = [selected, explicit]
                except AppError as exc:
                    case["error"] = error(exc)
                out.append(case)
    for status in [400, 422, 500]:
        for body in ['store', 'store2', '2store', 'restore', '_store', 'store_foo', 'STORE',
                '{"error":{"param":"model","message":"store unsupported"}}',
                '{"error":{"param":"body.store","message":"unsupported"}}',
                '{"error":{"param":"body.options.store","message":"unsupported"}}',
                '{"error":{"loc":["body","store"],"input":{"store":false}}}',
                '{"error":{"input":{"store":false},"message":"unsupported"}}',
                '{"error":{"message":"store2 unsupported"}}', '{"bad":']:
            for truncated in [False, True]:
                details = {"status": status, "body": body, "body_truncated": truncated}
                out.append(dict(operation="store", details=details,
                    result=_names_store(AppError("invalid", code="ai_request_invalid", details=details))))
    return out


if __name__ == "__main__":
    (ROOT / "src/file_import/document/provider_defaults.json").write_text(json.dumps(DEFAULT_BOOTSTRAP_PROVIDERS, indent=2) + "\n")
    (ROOT / "src/file_import/document/ocr_prompt.txt").write_text(ref.OCR_SYSTEM_PROMPT)
    (ROOT / "tests/vectors/python_document_providers.json").write_text(json.dumps(vectors(), indent=2, ensure_ascii=True) + "\n")
