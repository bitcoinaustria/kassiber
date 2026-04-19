.pragma library

var buttonHeights = { sm: 28, md: 36, lg: 44 }
var buttonPadding = { sm: 12, md: 16, lg: 20 }
var buttonFontSizes = { sm: 12, md: 13, lg: 14 }

function _defined(value) {
    return value !== undefined && value !== null && value !== ""
}

function pick(value, fallback) {
    return _defined(value) ? value : fallback
}

function paper(theme) {
    return pick(theme && theme.paper, "#F7F0E4")
}

function paperRaised(theme) {
    return pick(theme && theme.paper3, "#FFFDF8")
}

function paperAlt(theme) {
    return pick(theme && theme.paper2, "#FBF6ED")
}

function ink(theme) {
    return pick(theme && theme.ink, "#1A1613")
}

function ink2(theme) {
    return pick(theme && theme.ink2, "#544E45")
}

function ink3(theme) {
    return pick(theme && theme.ink3, "#8E867B")
}

function line(theme) {
    return pick(theme && theme.line, "#D8CDBD")
}

function line2(theme) {
    return pick(theme && theme.line2, "#C9BFAD")
}

function accent(theme) {
    return pick(theme && theme.accent, "#8B0000")
}

function accentDim(theme) {
    return pick(theme && theme.accentDim, "#C08A8A")
}

function ok(theme) {
    return pick(theme && theme.ok, "#166534")
}

function warn(theme) {
    return pick(theme && theme.warn, "#B45309")
}

function err(theme) {
    return pick(theme && theme.err, "#991B1B")
}

function paperOnInk(_theme) {
    return "#FFF6EF"
}

function serif(theme) {
    return pick(theme && theme.serifFont, "Baskerville")
}

function mono(theme) {
    return pick(theme && theme.monoFont, "Menlo")
}

function sans(theme) {
    return pick(theme && theme.sansFont, "Avenir Next")
}

function heightFor(size) {
    return buttonHeights[size] || buttonHeights.md
}

function paddingFor(size) {
    return buttonPadding[size] || buttonPadding.md
}

function fontSizeFor(size) {
    return buttonFontSizes[size] || buttonFontSizes.md
}

function pillTone(theme, tone) {
    if (tone === "accent") {
        return { border: accent(theme), fg: accent(theme) }
    }
    if (tone === "muted") {
        return { border: line2(theme), fg: ink2(theme) }
    }
    return { border: ink(theme), fg: ink(theme) }
}

function buttonColors(theme, variant, enabled, down, hovered) {
    var bg = "transparent"
    var fg = ink(theme)
    var border = ink(theme)
    var opacity = enabled ? 1.0 : 0.45

    if (variant === "primary") {
        bg = enabled ? accent(theme) : accentDim(theme)
        fg = paperOnInk(theme)
        border = bg
        if (enabled && (down || hovered)) {
            opacity = down ? 0.82 : 0.92
        }
    } else if (variant === "secondary") {
        bg = down ? ink(theme) : (hovered ? paperAlt(theme) : "transparent")
        fg = down ? paper(theme) : ink(theme)
        border = ink(theme)
    } else if (variant === "danger") {
        bg = down ? err(theme) : (hovered ? accent(theme) : "transparent")
        fg = (down || hovered) ? paper(theme) : accent(theme)
        border = accent(theme)
    } else {
        bg = hovered ? paperAlt(theme) : "transparent"
        fg = ink2(theme)
        border = "transparent"
    }

    return { bg: bg, fg: fg, border: border, opacity: opacity }
}
