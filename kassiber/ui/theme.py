from __future__ import annotations

from PySide6.QtCore import QObject, Property


COLORS = {
    "bg": "#F5F1E8",
    "card": "#FFFFFF",
    "card_border": "#EDEAE0",
    "ink": "#1A1916",
    "ink_muted": "#6B6862",
    "accent": "#8B0000",
    "accent_dim": "#C08A8A",
    "chip_border": "#C9C4B8",
    "ok": "#166534",
    "warn": "#B45309",
    "err": "#991B1B",
}

FONTS = {
    "display": "Source Serif 4",
    "body": "Courier Prime",
}

SPACING = {"sm": 8, "md": 16, "lg": 24, "xl": 40}
RADIUS = {"sm": 4, "md": 8, "lg": 12}


class Theme(QObject):
    @Property(str, constant=True)
    def bg(self) -> str:
        return COLORS["bg"]

    @Property(str, constant=True)
    def card(self) -> str:
        return COLORS["card"]

    @Property(str, constant=True)
    def cardBorder(self) -> str:
        return COLORS["card_border"]

    @Property(str, constant=True)
    def ink(self) -> str:
        return COLORS["ink"]

    @Property(str, constant=True)
    def inkMuted(self) -> str:
        return COLORS["ink_muted"]

    @Property(str, constant=True)
    def accent(self) -> str:
        return COLORS["accent"]

    @Property(str, constant=True)
    def accentDim(self) -> str:
        return COLORS["accent_dim"]

    @Property(str, constant=True)
    def chipBorder(self) -> str:
        return COLORS["chip_border"]

    @Property(str, constant=True)
    def ok(self) -> str:
        return COLORS["ok"]

    @Property(str, constant=True)
    def warn(self) -> str:
        return COLORS["warn"]

    @Property(str, constant=True)
    def err(self) -> str:
        return COLORS["err"]

    @Property(str, constant=True)
    def displayFont(self) -> str:
        return FONTS["display"]

    @Property(str, constant=True)
    def bodyFont(self) -> str:
        return FONTS["body"]

    @Property(int, constant=True)
    def spacingSm(self) -> int:
        return SPACING["sm"]

    @Property(int, constant=True)
    def spacingMd(self) -> int:
        return SPACING["md"]

    @Property(int, constant=True)
    def spacingLg(self) -> int:
        return SPACING["lg"]

    @Property(int, constant=True)
    def spacingXl(self) -> int:
        return SPACING["xl"]

    @Property(int, constant=True)
    def radiusSm(self) -> int:
        return RADIUS["sm"]

    @Property(int, constant=True)
    def radiusMd(self) -> int:
        return RADIUS["md"]

    @Property(int, constant=True)
    def radiusLg(self) -> int:
        return RADIUS["lg"]


__all__ = ["Theme", "COLORS", "FONTS", "SPACING", "RADIUS"]
