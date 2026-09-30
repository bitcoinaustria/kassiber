"""Work out where a BTCPay invoice came from.

Origin is provenance for review, never a matching or tax signal. The
classifier recognises BTCPay's built-in apps, payment requests, and the
common e-commerce plugins from the metadata those apps actually write:

- Point of Sale and Crowdfund invoices carry an ``orderUrl`` of
  ``/apps/{appId}/pos`` or ``/apps/{appId}/crowdfund``; PoS invoices also carry
  ``posData`` with the cart.
- Payment-request invoices carry ``metadata.paymentRequestId`` and an order
  id of ``PAY_REQUEST_{id}``.
- WooCommerce's Greenfield plugin nests its order under ``posData`` and links
  back to ``wp-admin``; Shopify invoices use ``shopify*`` metadata keys or a
  ``myshopify.com`` order URL.
- Other plugins that register an app route (``/apps/{appId}/{type}``) or set
  ``appId``/``appName`` are kept generically as ``app``.
"""

from __future__ import annotations

import re
from typing import Any, Mapping
from urllib import parse as urlparse

PAYMENT_REQUEST_ORDER_PREFIX = "PAY_REQUEST_"
_APP_ROUTE = re.compile(r"/apps/(?P<app_id>[^/?#]+)/(?P<app_type>[^/?#]+)")
_LEGACY_APP_ROUTE = re.compile(
    r"/apps/(?P<app_type>pos|crowdfund)(?:/(?P<app_id>[^/?#]+))?(?=[/?#]|$)",
    re.IGNORECASE,
)

ORIGIN_KINDS = (
    "pos",
    "crowdfund",
    "payment_request",
    "ecommerce",
    "app",
    "external_order",
    "unknown",
)


def _text(value: Any) -> str | None:
    if value in (None, ""):
        return None
    text = str(value).strip()
    return text or None


def _url_path(url: str | None) -> str:
    if not url:
        return ""
    try:
        return urlparse.urlsplit(url).path or ""
    except ValueError:
        return ""


def _app_route(order_url: str | None) -> tuple[str | None, str | None]:
    path = _url_path(order_url)
    legacy = _LEGACY_APP_ROUTE.search(path)
    if legacy:
        return legacy.group("app_id"), legacy.group("app_type").lower()
    match = _APP_ROUTE.search(path)
    if match:
        return urlparse.unquote(match.group("app_id")), match.group("app_type").lower()
    return None, None


def _pos_data(metadata: Mapping[str, Any]) -> Any:
    return metadata.get("posData")


def _pos_data_label(pos_data: Any) -> str | None:
    if not isinstance(pos_data, Mapping):
        return None
    for key in ("title", "name", "itemDesc", "itemDescription", "description"):
        value = _text(pos_data.get(key))
        if value:
            return value
    cart = pos_data.get("cart")
    if isinstance(cart, list):
        titles = [
            _text(item.get("title")) or _text(item.get("id"))
            for item in cart
            if isinstance(item, Mapping)
        ]
        titles = [title for title in titles if title]
        if titles:
            return ", ".join(titles[:3]) + (" …" if len(titles) > 3 else "")
    return None


def _ecommerce_source(metadata: Mapping[str, Any], order_url: str | None) -> str | None:
    lowered_url = (order_url or "").lower()
    pos_data = _pos_data(metadata)
    pos_keys = {str(key).lower() for key in pos_data} if isinstance(pos_data, Mapping) else set()
    if "woocommerce" in pos_keys or "wp-admin" in lowered_url or "wc-api" in lowered_url or "woocommerce" in lowered_url:
        return "woocommerce"
    metadata_keys = {str(key).lower() for key in metadata}
    if any(key.startswith("shopify") for key in metadata_keys) or "myshopify.com" in lowered_url or "shopify" in pos_keys:
        return "shopify"
    if any(key.startswith("magento") for key in metadata_keys) or "magento" in pos_keys:
        return "magento"
    if any(key.startswith("prestashop") for key in metadata_keys) or "prestashop" in pos_keys:
        return "prestashop"
    source = _text(metadata.get("source") or metadata.get("plugin"))
    if source and source.lower() in {"woocommerce", "shopify", "magento", "prestashop", "odoo", "drupal", "whmcs", "wix", "ecwid"}:
        return source.lower()
    return None


def payment_request_id_for(invoice: Mapping[str, Any], metadata: Mapping[str, Any], order_id: str | None) -> str | None:
    explicit = _text(
        metadata.get("paymentRequestId")
        or metadata.get("payment_request_id")
        or invoice.get("paymentRequestId")
    )
    if explicit:
        return explicit
    if order_id and order_id.upper().startswith(PAYMENT_REQUEST_ORDER_PREFIX):
        return _text(order_id[len(PAYMENT_REQUEST_ORDER_PREFIX):])
    return None


def classify_invoice_origin(
    invoice: Mapping[str, Any],
    metadata: Mapping[str, Any],
    order_id: str | None,
    *,
    payment_request_titles: Mapping[str, str] | None = None,
) -> dict[str, Any]:
    order_url = _text(metadata.get("orderUrl") or invoice.get("orderUrl"))
    app_id = _text(metadata.get("appId") or metadata.get("app_id") or metadata.get("applicationId"))
    app_name = _text(metadata.get("appName") or metadata.get("app_name") or metadata.get("applicationName"))
    item_desc = _text(metadata.get("itemDesc") or metadata.get("itemDescription"))
    pos_data = _pos_data(metadata)
    pos_label = _pos_data_label(pos_data)
    route_app_id, route_app_type = _app_route(order_url)
    payment_request_id = payment_request_id_for(invoice, metadata, order_id)
    lowered_url = (order_url or "").lower()
    lowered_order = (order_id or "").lower()
    lowered_app = f"{app_name or ''} {app_id or ''}".lower()
    ecommerce = _ecommerce_source(metadata, order_url)

    def result(kind, *, label, app=None, app_type=None, source=None):
        return {
            "kind": kind,
            "app_id": app,
            "app_type": app_type,
            "label": label,
            "url": order_url,
            "source": source,
        }

    if ecommerce:
        order_number = _text(metadata.get("orderNumber") or order_id)
        label = item_desc or (f"Order {order_number}" if order_number else None)
        return result("ecommerce", label=label, source=ecommerce)
    if route_app_type == "pos" or pos_data is not None or "/pos" in lowered_url or lowered_order.startswith("pos"):
        return result(
            "pos",
            label=app_name or item_desc or pos_label or order_id,
            app=app_id or route_app_id,
            app_type="pos",
            source="btcpay_pos",
        )
    if (
        route_app_type == "crowdfund"
        or "crowdfund" in lowered_app
        or "/crowdfund" in lowered_url
        or lowered_order.startswith("crowdfund")
    ):
        return result(
            "crowdfund",
            label=app_name or item_desc or order_id,
            app=app_id or route_app_id,
            app_type="crowdfund",
            source="btcpay_crowdfund",
        )
    if payment_request_id:
        title = (payment_request_titles or {}).get(payment_request_id)
        return result(
            "payment_request",
            label=item_desc or title or payment_request_id,
            source="btcpay_payment_request",
        )
    if app_id or app_name or route_app_id:
        return result(
            "app",
            label=app_name or item_desc or order_id,
            app=app_id or route_app_id,
            app_type=route_app_type,
            source="btcpay_app",
        )
    if order_url or order_id:
        return result("external_order", label=item_desc or order_id)
    return result("unknown", label=item_desc)


__all__ = [
    "ORIGIN_KINDS",
    "PAYMENT_REQUEST_ORDER_PREFIX",
    "classify_invoice_origin",
    "payment_request_id_for",
]
