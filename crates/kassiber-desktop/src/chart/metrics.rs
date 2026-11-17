//! DM Sans advance widths (wght 400, the font's default instance) in 1/1000 em, read from
//! assets/fonts/dm-sans-wght.ttf's hmtx. For layout decisions the web takes from measured text
//! (Recharts' tick thinning and label wrapping). Kerning is ignored; unknown glyphs count 0.6 em.

const ADV: &[(char, u16)] = &[
    (' ', 266),
    ('!', 250),
    ('"', 291),
    ('#', 804),
    ('$', 580),
    ('%', 786),
    ('&', 734),
    ('\'', 159),
    ('(', 373),
    (')', 373),
    ('*', 480),
    ('+', 550),
    (',', 182),
    ('-', 541),
    ('.', 198),
    ('/', 392),
    ('0', 684),
    ('1', 312),
    ('2', 576),
    ('3', 591),
    ('4', 607),
    ('5', 610),
    ('6', 628),
    ('7', 534),
    ('8', 608),
    ('9', 628),
    (':', 202),
    (';', 227),
    ('<', 550),
    ('=', 550),
    ('>', 550),
    ('?', 524),
    ('@', 1005),
    ('A', 664),
    ('B', 603),
    ('C', 717),
    ('D', 688),
    ('E', 565),
    ('F', 535),
    ('G', 758),
    ('H', 681),
    ('I', 234),
    ('J', 501),
    ('K', 584),
    ('L', 527),
    ('M', 841),
    ('N', 691),
    ('O', 774),
    ('P', 580),
    ('Q', 774),
    ('R', 594),
    ('S', 580),
    ('T', 561),
    ('U', 655),
    ('V', 669),
    ('W', 968),
    ('X', 603),
    ('Y', 580),
    ('Z', 541),
    ('[', 314),
    ('\\', 392),
    (']', 314),
    ('^', 622),
    ('_', 660),
    ('`', 213),
    ('a', 544),
    ('b', 626),
    ('c', 572),
    ('d', 627),
    ('e', 568),
    ('f', 340),
    ('g', 558),
    ('h', 575),
    ('i', 240),
    ('j', 243),
    ('k', 504),
    ('l', 223),
    ('m', 891),
    ('n', 574),
    ('o', 593),
    ('p', 626),
    ('q', 627),
    ('r', 370),
    ('s', 502),
    ('t', 392),
    ('u', 573),
    ('v', 527),
    ('w', 763),
    ('x', 496),
    ('y', 556),
    ('z', 458),
    ('{', 427),
    ('|', 234),
    ('}', 427),
    ('~', 550),
    ('\u{a0}', 266),
    ('\u{20ac}', 771),
    ('\u{b7}', 198),
    ('\u{2014}', 873),
    ('\u{2013}', 669),
    ('\u{2212}', 550),
    ('\u{2026}', 566),
    ('\u{2019}', 210),
    ('\u{e4}', 544),
    ('\u{f6}', 593),
    ('\u{fc}', 573),
    ('\u{c4}', 664),
    ('\u{d6}', 774),
    ('\u{dc}', 655),
    ('\u{df}', 659),
    ('\u{e9}', 568),
];

/// The width of `s` in DM Sans 400 at `size` points.
pub fn text_w(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| {
            ADV.iter()
                .find(|(k, _)| *k == c)
                .map(|(_, a)| *a as f32 / 1000.0)
                .unwrap_or(0.6)
        })
        .sum::<f32>()
        * size
}

/// Recharts `Text` word wrapping: words joined while the line fits `max` points.
pub fn wrap(s: &str, size: f32, max: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in s.split(' ') {
        match lines.last_mut() {
            Some(line) if text_w(&format!("{line} {word}"), size) <= max => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}
