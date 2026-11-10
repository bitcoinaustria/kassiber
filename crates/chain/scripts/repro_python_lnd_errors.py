import io
import json
from pathlib import Path
import sys
from unittest.mock import patch
from urllib.error import HTTPError

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.lightning import lnd
from kassiber.errors import AppError


def reproduce():
    marker = "SYNTHETIC_LND_RESPONSE_SECRET"
    client = lnd.LndRestClient({"url": "http://127.0.0.1:1", "token": "aabb"})
    error = HTTPError(client.base_url, 401, "Unauthorized", {}, io.BytesIO(marker.encode()))
    with patch.object(lnd, "urlopen_with_proxy", side_effect=error):
        try:
            client.get("/v1/getinfo")
        except AppError as caught:
            assert marker in str(caught)
            http_leak = True
        else:
            raise AssertionError("expected HTTP refusal")

    class Response(io.BytesIO):
        status = 200

    with patch.object(lnd, "urlopen_with_proxy", return_value=Response(marker.encode())):
        try:
            client.get("/v1/getinfo")
        except AppError as caught:
            assert marker in json.dumps(caught.details)
            json_leak = True
        else:
            raise AssertionError("expected JSON refusal")
    return {"http_error_discloses_response": http_leak, "json_error_discloses_response": json_leak}


if __name__ == "__main__":
    print(json.dumps(reproduce(), sort_keys=True))
