from __future__ import annotations

from PySide6.QtCore import QObject, Property


COLORS = {
    "bg": "#F5F1E8",
    "card": "#FFFFFF",
    "card_border": "#EDEAE0",
    "card_alt": "#FAF7F0",
    "warm_bg": "#EFE6D8",
    "warm_paper": "#F6EEDD",
    "warm_border": "#D8CDBD",
    "warm_grid": "#E7DDCF",
    "warm_grid_strong": "#DDD1C0",
    "warm_shadow": "#1E1916",
    "warm_accent": "#D4A3A3",
    "ink": "#1A1916",
    "ink_muted": "#6B6862",
    "accent": "#8B0000",
    "accent_dim": "#C08A8A",
    "chip_border": "#C9C4B8",
    "pill_amber": "#D97706",
    "pill_yellow": "#CA8A04",
    "pill_teal": "#0F766E",
    "pill_green": "#166534",
    "pill_olive": "#65A30D",
    "pill_indigo": "#4338CA",
    "ok": "#166534",
    "warn": "#B45309",
    "err": "#991B1B",
}

FONTS = {
    "display": "Baskerville",
    "body": "Menlo",
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
    def cardAlt(self) -> str:
        return COLORS["card_alt"]

    @Property(str, constant=True)
    def warmBg(self) -> str:
        return COLORS["warm_bg"]

    @Property(str, constant=True)
    def warmPaper(self) -> str:
        return COLORS["warm_paper"]

    @Property(str, constant=True)
    def warmBorder(self) -> str:
        return COLORS["warm_border"]

    @Property(str, constant=True)
    def warmGrid(self) -> str:
        return COLORS["warm_grid"]

    @Property(str, constant=True)
    def warmGridStrong(self) -> str:
        return COLORS["warm_grid_strong"]

    @Property(str, constant=True)
    def warmShadow(self) -> str:
        return COLORS["warm_shadow"]

    @Property(str, constant=True)
    def warmAccent(self) -> str:
        return COLORS["warm_accent"]

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
    def pillAmber(self) -> str:
        return COLORS["pill_amber"]

    @Property(str, constant=True)
    def pillYellow(self) -> str:
        return COLORS["pill_yellow"]

    @Property(str, constant=True)
    def pillTeal(self) -> str:
        return COLORS["pill_teal"]

    @Property(str, constant=True)
    def pillGreen(self) -> str:
        return COLORS["pill_green"]

    @Property(str, constant=True)
    def pillOlive(self) -> str:
        return COLORS["pill_olive"]

    @Property(str, constant=True)
    def pillIndigo(self) -> str:
        return COLORS["pill_indigo"]

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
