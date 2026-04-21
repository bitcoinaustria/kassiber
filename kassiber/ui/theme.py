from __future__ import annotations

from PySide6.QtCore import QObject, Property


COLORS = {
    "bg": "#FFFFFF",
    "paper": "#FFFFFF",
    "paper_2": "#FAFAFA",
    "paper_3": "#F6F6F6",
    "card": "#FAFAFA",
    "card_border": "#D6D6D6",
    "card_alt": "#FAFAFA",
    "warm_bg": "#EFE6D8",
    "warm_paper": "#F6EEDD",
    "warm_border": "#D8CDBD",
    "warm_grid": "#E7DDCF",
    "warm_grid_strong": "#DDD1C0",
    "warm_shadow": "#1E1916",
    "warm_accent": "#CD6873",
    "line": "#D6D6D6",
    "line_2": "#BBBBBB",
    "ink": "#222222",
    "ink_muted": "#555555",
    "ink_2": "#555555",
    "ink_3": "#888888",
    "accent": "#E3000F",
    "accent_dim": "#F28A91",
    "chip_border": "#BBBBBB",
    "pill_amber": "#D97706",
    "pill_yellow": "#CA8A04",
    "pill_teal": "#0F766E",
    "pill_green": "#166534",
    "pill_olive": "#65A30D",
    "pill_indigo": "#4338CA",
    "ok": "#166534",
    "warn": "#B45309",
    "err": "#991B1B",
    "positive": "#3FA66A",
    "type_income": "#3FA66A",
    "type_expense": "#E3000F",
    "type_transfer": "#6B7280",
    "type_swap": "#8B6F3C",
    "type_consolidation": "#5D6B7A",
    "type_rebalance": "#7D6B8A",
    "type_mint": "#3F7AA6",
    "type_melt": "#A66A3F",
    "type_fee": "#8A7F71",
}

FONTS = {
    "display": "Blinker",
    "serif": "Blinker",
    "sans": "Blinker",
    "body": "Blinker",
    "mono": "Blinker",
}

SPACING = {"xs": 4, "sm": 8, "md": 16, "lg": 24, "xl": 40, "xxl": 56}
RADIUS = {"sm": 2, "md": 8, "lg": 12, "xl": 18}

# Semantic tokens (named, fluid-layout-friendly)
SEMANTIC = {
    "page_padding": 12,
    "grid_gap": 10,
    "card_padding": 14,
    "card_header_height": 36,
    "chrome_gap": 18,
    "header_height": 54,
    "footer_height": 28,
    "control_height_sm": 22,
    "control_height_md": 26,
    "control_height_lg": 36,
    "row_height_compact": 32,
    "row_height_default": 36,
    "badge_padding_h": 6,
    "badge_padding_v": 2,
    "dot_size": 6,
    "icon_tile_size": 34,
}

FONT_PX = {
    "micro": 9,
    "caption": 10,
    "body_small": 11,
    "body": 12,
    "body_strong": 13,
    "heading_xs": 14,
    "heading_sm": 16,
    "heading_md": 18,
    "heading_lg": 20,
    "heading_xl": 26,
    "display": 32,
}


class Theme(QObject):
    @Property(str, constant=True)
    def bg(self) -> str:
        return COLORS["bg"]

    @Property(str, constant=True)
    def paper(self) -> str:
        return COLORS["paper"]

    @Property(str, constant=True)
    def paper2(self) -> str:
        return COLORS["paper_2"]

    @Property(str, constant=True)
    def paper3(self) -> str:
        return COLORS["paper_3"]

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
    def line(self) -> str:
        return COLORS["line"]

    @Property(str, constant=True)
    def line2(self) -> str:
        return COLORS["line_2"]

    @Property(str, constant=True)
    def ink(self) -> str:
        return COLORS["ink"]

    @Property(str, constant=True)
    def inkMuted(self) -> str:
        return COLORS["ink_muted"]

    @Property(str, constant=True)
    def ink2(self) -> str:
        return COLORS["ink_2"]

    @Property(str, constant=True)
    def ink3(self) -> str:
        return COLORS["ink_3"]

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
    def positive(self) -> str:
        return COLORS["positive"]

    @Property(str, constant=True)
    def typeIncome(self) -> str:
        return COLORS["type_income"]

    @Property(str, constant=True)
    def typeExpense(self) -> str:
        return COLORS["type_expense"]

    @Property(str, constant=True)
    def typeTransfer(self) -> str:
        return COLORS["type_transfer"]

    @Property(str, constant=True)
    def typeSwap(self) -> str:
        return COLORS["type_swap"]

    @Property(str, constant=True)
    def typeConsolidation(self) -> str:
        return COLORS["type_consolidation"]

    @Property(str, constant=True)
    def typeRebalance(self) -> str:
        return COLORS["type_rebalance"]

    @Property(str, constant=True)
    def typeMint(self) -> str:
        return COLORS["type_mint"]

    @Property(str, constant=True)
    def typeMelt(self) -> str:
        return COLORS["type_melt"]

    @Property(str, constant=True)
    def typeFee(self) -> str:
        return COLORS["type_fee"]

    @Property(str, constant=True)
    def displayFont(self) -> str:
        return FONTS["display"]

    @Property(str, constant=True)
    def serifFont(self) -> str:
        return FONTS["serif"]

    @Property(str, constant=True)
    def sansFont(self) -> str:
        return FONTS["sans"]

    @Property(str, constant=True)
    def bodyFont(self) -> str:
        return FONTS["body"]

    @Property(str, constant=True)
    def monoFont(self) -> str:
        return FONTS["mono"]

    @Property(int, constant=True)
    def spacingXs(self) -> int:
        return SPACING["xs"]

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
    def spacingXxl(self) -> int:
        return SPACING["xxl"]

    @Property(int, constant=True)
    def radiusSm(self) -> int:
        return RADIUS["sm"]

    @Property(int, constant=True)
    def radiusMd(self) -> int:
        return RADIUS["md"]

    @Property(int, constant=True)
    def radiusLg(self) -> int:
        return RADIUS["lg"]

    @Property(int, constant=True)
    def radiusXl(self) -> int:
        return RADIUS["xl"]

    # Semantic layout tokens --------------------------------------------
    @Property(int, constant=True)
    def pagePadding(self) -> int:
        return SEMANTIC["page_padding"]

    @Property(int, constant=True)
    def gridGap(self) -> int:
        return SEMANTIC["grid_gap"]

    @Property(int, constant=True)
    def cardPadding(self) -> int:
        return SEMANTIC["card_padding"]

    @Property(int, constant=True)
    def cardHeaderHeight(self) -> int:
        return SEMANTIC["card_header_height"]

    @Property(int, constant=True)
    def chromeGap(self) -> int:
        return SEMANTIC["chrome_gap"]

    @Property(int, constant=True)
    def headerHeight(self) -> int:
        return SEMANTIC["header_height"]

    @Property(int, constant=True)
    def footerHeight(self) -> int:
        return SEMANTIC["footer_height"]

    @Property(int, constant=True)
    def controlHeightSm(self) -> int:
        return SEMANTIC["control_height_sm"]

    @Property(int, constant=True)
    def controlHeightMd(self) -> int:
        return SEMANTIC["control_height_md"]

    @Property(int, constant=True)
    def controlHeightLg(self) -> int:
        return SEMANTIC["control_height_lg"]

    @Property(int, constant=True)
    def rowHeightCompact(self) -> int:
        return SEMANTIC["row_height_compact"]

    @Property(int, constant=True)
    def rowHeightDefault(self) -> int:
        return SEMANTIC["row_height_default"]

    @Property(int, constant=True)
    def badgePaddingH(self) -> int:
        return SEMANTIC["badge_padding_h"]

    @Property(int, constant=True)
    def badgePaddingV(self) -> int:
        return SEMANTIC["badge_padding_v"]

    @Property(int, constant=True)
    def dotSize(self) -> int:
        return SEMANTIC["dot_size"]

    @Property(int, constant=True)
    def iconTileSize(self) -> int:
        return SEMANTIC["icon_tile_size"]

    # Font size scale ----------------------------------------------------
    @Property(int, constant=True)
    def fontMicro(self) -> int:
        return FONT_PX["micro"]

    @Property(int, constant=True)
    def fontCaption(self) -> int:
        return FONT_PX["caption"]

    @Property(int, constant=True)
    def fontBodySmall(self) -> int:
        return FONT_PX["body_small"]

    @Property(int, constant=True)
    def fontBody(self) -> int:
        return FONT_PX["body"]

    @Property(int, constant=True)
    def fontBodyStrong(self) -> int:
        return FONT_PX["body_strong"]

    @Property(int, constant=True)
    def fontHeadingXs(self) -> int:
        return FONT_PX["heading_xs"]

    @Property(int, constant=True)
    def fontHeadingSm(self) -> int:
        return FONT_PX["heading_sm"]

    @Property(int, constant=True)
    def fontHeadingMd(self) -> int:
        return FONT_PX["heading_md"]

    @Property(int, constant=True)
    def fontHeadingLg(self) -> int:
        return FONT_PX["heading_lg"]

    @Property(int, constant=True)
    def fontHeadingXl(self) -> int:
        return FONT_PX["heading_xl"]

    @Property(int, constant=True)
    def fontDisplay(self) -> int:
        return FONT_PX["display"]


__all__ = ["Theme", "COLORS", "FONTS", "SPACING", "RADIUS", "SEMANTIC", "FONT_PX"]
