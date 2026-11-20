//! Brand and rail logos, sized by their artwork. The web sizes each logo by its canvas with a
//! per-logo class (`size-7`, `size-9`, `scale-150` …), so the same Bitcoin coin is 28 px for one
//! connection source and 36 px for the next, and a padded SVG reads smaller than a full-bleed PNG.
//! Here every logo is measured once (`tools/logo-bounds.mjs` writes `BOUNDS`) and drawn so its
//! painted artwork's longer side is the size of its role, centred on the artwork, rasterised at
//! the device scale. The files are the ones the web app uses too (assets/logos/integrations at the repository root).

use std::cell::RefCell;
use std::collections::HashMap;

use rune::{Cx, El, Pixels, el, row};

use crate::app::App;

macro_rules! web_asset {
    ($file:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/logos/integrations/",
            $file
        ))
    };
}

pub static BITCOIN: &[u8] = web_asset!("bitcoin.svg");
pub static LIQUID: &[u8] = web_asset!("liquid.svg");
pub static LIGHTNING: &[u8] = web_asset!("lightning.svg");
pub static CORE_LIGHTNING: &[u8] = web_asset!("core-lightning.svg");
pub static LIGHTNING_LABS_SVG: &[u8] = web_asset!("lightning-labs.svg");
pub static LIGHTNING_LABS_PNG: &[u8] = web_asset!("lightning-labs.png");
pub static COINTRACKING: &[u8] = web_asset!("cointracking.svg");
pub static BLOCKPIT: &[u8] = web_asset!("blockpit.png");

/// Artwork size per role, in rem (the web's dominant size for that role; see the module docs).
pub mod role {
    /// `RailIcon size="compact"` (Transfers, Review, Overview holdings): the `size-5` logo in
    /// its `size-6` slot.
    pub const RAIL: (f32, f32) = (1.25, 1.5);
    /// The add-connection source list's `size-7 p-0.5` tile: the whole 22 px inner box.
    pub const SOURCE_ROW: f32 = 1.375;
    /// The add-connection summary's `size-12` tile: `size-9`, the catalog's usual logo size.
    pub const SOURCE_SUMMARY: f32 = 2.25;
    /// The Imports card's `size-10` box: `size-9`, as in the summary.
    pub const SOURCE_CARD: f32 = 2.25;
    /// `ConnectionAssetBadge` `sm` (`size-8`) and `md` (`size-9`): the bitcoin coin's
    /// `p-0.5 scale-115` size, which every other asset now shares.
    pub const ASSET_BADGE_SM: f32 = 1.875;
    pub const ASSET_BADGE_MD: f32 = 2.125;
    /// `NetworkMark` (`size-9 p-1.5` tile): the bitcoin coin's `size-6` (the web zooms the
    /// Liquid disc and the Core-LN bolt by 1.5 on top).
    pub const NETWORK_MARK: f32 = 1.5;
    /// `RouteLegAssetIcon` (`size-4` box): the bitcoin coin's `p-0.5` size. Unused since the
    /// route card (txdetail/graph/route.rs) shows the rail logo in the table kit's slot.
    #[allow(dead_code)]
    pub const ROUTE_LEG: f32 = 0.625;
}

/// The painted box of each logo, `[x0, y0, x1, y1]` in units of the image's longer side, by the
/// file's FNV-1a hash. Regenerate with `node tools/logo-bounds.mjs ../../assets/logos/integrations`.
const BOUNDS: &[(u64, [f32; 4])] = &[
    (0x4b6659f8b144ae41, [0.000, 0.000, 1.000, 1.000]), // 21bitcoin.png
    (0x7db9874a9dfcbeda, [0.000, 0.000, 1.000, 1.000]), // binance.svg
    (0x6311a3b36c9cbdde, [0.000, 0.000, 0.885, 0.987]), // bisq.svg
    (0x24e278ad2b7fce45, [0.207, 0.041, 0.793, 0.959]), // bitbox.svg
    (0x648daec469342a7f, [0.000, 0.000, 1.000, 1.000]), // bitcoin.svg
    (0x6a23b2c946006f26, [0.000, 0.070, 0.570, 0.930]), // bitpanda.svg
    (0xd47fd81727ab93db, [0.000, 0.000, 1.000, 1.000]), // blockpit.png
    (0x0725fe6b7ae35352, [0.000, 0.000, 1.000, 1.000]), // blockstream-green.svg
    (0xc8c35f7e8daec450, [0.027, 0.031, 0.789, 0.971]), // bluewallet.png
    (0xb0cbdca062d9ee70, [0.000, 0.000, 0.561, 1.000]), // btcpay.svg
    (0xed50d37f67e2a08f, [0.063, 0.119, 0.938, 0.883]), // bullbitcoin-mark.png
    (0xb1ba5851802f8529, [0.000, 0.000, 0.998, 1.000]), // coinbase.svg
    (0x34fcd9d09906cdf0, [0.000, 0.000, 1.000, 1.000]), // coinfinity-mark.svg
    (0x4c8951eabeab8bbf, [0.084, 0.199, 0.955, 0.670]), // cointracking.svg
    (0x13520bb838c057fe, [0.207, 0.041, 0.820, 0.959]), // coldcard.svg
    (0x1fceb6589e4e3b0d, [0.355, 0.178, 0.645, 0.822]), // core-lightning.svg
    (0xb940348da7a120c2, [0.207, 0.041, 0.793, 0.959]), // foundation-passport.svg
    (0x3fe3516ffb3c0121, [0.000, 0.021, 1.000, 0.820]), // kraken.svg
    (0x237e830ea3021a3d, [0.250, 0.041, 0.750, 0.959]), // ledger.svg
    (0xffcbe7aa5abb999c, [0.123, 0.109, 0.711, 0.891]), // liana.svg
    (0x314b229aa67146ef, [0.205, 0.012, 0.736, 0.998]), // lightning-labs.png
    (0x85b55ee47ed75904, [0.000, 0.000, 1.000, 1.000]), // lightning-labs.svg
    (0xce16f099b0ef0efc, [0.072, 0.000, 0.928, 1.000]), // lightning.svg
    (0x98f4082e884ed65f, [0.035, 0.035, 0.965, 0.965]), // liquid.svg
    (0x83194cfd3e83499a, [0.000, 0.006, 0.994, 1.000]), // mempool-space.svg
    (0x245c045899616a72, [0.109, 0.338, 0.875, 0.662]), // nunchuk.svg
    (0x10f57c2a29d05ed8, [0.000, 0.000, 1.000, 1.000]), // pocket-bitcoin.svg
    (0xed38bfb8c40da770, [0.000, 0.000, 1.000, 1.000]), // relai.svg
    (0x4eb53d3bdde60949, [0.000, 0.000, 1.000, 1.000]), // river.png
    (0xa826d3f8050b9522, [0.000, 0.000, 1.000, 1.000]), // samourai.png
    (0x6462752e979f9110, [0.041, 0.045, 0.959, 0.971]), // sparrow.png
    (0x018cc8eca40bf8d3, [0.000, 0.000, 1.000, 1.000]), // strike.jpg
    (0xe3b2adbb888fb228, [0.000, 0.000, 1.000, 1.000]), // swan-bitcoin.svg
    (0x8c8bfb7caacd6e65, [0.207, 0.041, 0.793, 0.959]), // trezor.svg
    (0x0ac3498bf1d4be40, [0.000, 0.000, 1.000, 1.000]), // wasabi.png
];

const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// FNV-1a 64 (the measuring tool's hash).
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// (asset address, device px, ink).
type DecodedKey = (usize, u32, Option<&'static str>);

thread_local! {
    /// Bounds by asset address, so the hash runs once per file.
    static KNOWN: RefCell<HashMap<(usize, usize), [f32; 4]>> = RefCell::new(HashMap::new());
    /// Decoded pixels by (asset address, device px, ink), so frames don't re-decode.
    static DECODED: RefCell<HashMap<DecodedKey, Pixels>> = RefCell::new(HashMap::new());
}

/// The painted box of `bytes` (the whole canvas for an unmeasured file).
pub fn bounds(bytes: &'static [u8]) -> [f32; 4] {
    let key = (bytes.as_ptr() as usize, bytes.len());
    KNOWN.with(|m| {
        *m.borrow_mut().entry(key).or_insert_with(|| {
            let h = fnv(bytes);
            BOUNDS
                .iter()
                .find(|(k, _)| *k == h)
                .map_or(FULL, |(_, b)| *b)
        })
    })
}

/// A logo file: SVG (optionally flattened to one `ink`, the web's `brightness-0 dark:invert`)
/// or a raster image.
#[derive(Clone, Copy)]
pub struct Logo {
    pub bytes: &'static [u8],
    pub svg: bool,
    pub ink: Option<&'static str>,
}

impl Logo {
    pub const fn svg(bytes: &'static [u8]) -> Logo {
        Logo {
            bytes,
            svg: true,
            ink: None,
        }
    }
    pub const fn raster(bytes: &'static [u8]) -> Logo {
        Logo {
            bytes,
            svg: false,
            ink: None,
        }
    }
}

/// `logo` drawn so its artwork's longer side is `art` points, centred in a `frame` × `frame`
/// box (layout size; transparent canvas may overhang it). Hidden from AT: callers label the
/// element that means something.
pub fn fitted(app: &App, cx: &Cx<App>, logo: Logo, art: f32, frame: f32) -> El {
    fitted_at(app, cx.env().scale, logo, art, frame)
}

/// [`fitted`] at a known device `scale`.
pub fn fitted_at(app: &App, scale: f32, logo: Logo, art: f32, frame: f32) -> El {
    let scale = scale.max(1.0);
    let b = bounds(logo.bytes);
    let span = (b[2] - b[0]).max(b[3] - b[1]).max(0.05);
    // The image's longer side, in points, that puts the artwork at `art`.
    let side = art / span;
    let px_side = (side * scale).ceil() as u32;
    let key = (
        logo.bytes.as_ptr() as usize,
        if logo.svg { px_side } else { 0 },
        logo.ink,
    );
    let px = DECODED
        .with(|m| m.borrow().get(&key).cloned())
        .unwrap_or_else(|| {
            let img = match (logo.svg, logo.ink) {
                (true, Some(ink)) => app
                    .ov
                    .images()
                    .svg(silhouette(logo.bytes, ink), side, scale),
                (true, None) => app.ov.images().svg(logo.bytes, side, scale),
                (false, _) => app.ov.images().load(logo.bytes),
            };
            // A logo decodes in about a millisecond on the cache's worker; waiting keeps the first
            // frame (and headless snapshots) from drawing an empty tile.
            if !img.is_ready() {
                let _ = img.wait();
            }
            let p = img.pixels().clone();
            DECODED.with(|m| m.borrow_mut().insert(key, p.clone()));
            p
        });
    let (w, h) = px.size();
    let boxed = row().size(frame, frame).a11y_hidden();
    if w == 0 || h == 0 {
        return boxed;
    }
    let k = side / w.max(h) as f32;
    let (dw, dh) = (w as f32 * k, h as f32 * k);
    // Centre the artwork, not the canvas, on whole device pixels.
    let snap = |v: f32| (v * scale).round() / scale;
    let x = snap(frame / 2.0 - (b[0] + b[2]) / 2.0 * side);
    let y = snap(frame / 2.0 - (b[1] + b[3]) / 2.0 * side);
    boxed.child(el().absolute(x, y).size(dw, dh).pixels(&px))
}

/// An SVG with every fill and stroke colour (attributes and inline styles) set to `ink`;
/// `none` and gradient/url references stay as they are.
pub fn silhouette(svg: &[u8], ink: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(svg);
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_ref();
    let keys = [
        "fill=\"",
        "stroke=\"",
        "fill:",
        "stroke:",
        "stop-color=\"",
        "stop-color:",
    ];
    while let Some((pos, key)) = keys
        .iter()
        .filter_map(|k| rest.find(k).map(|p| (p, *k)))
        .min_by_key(|(p, _)| *p)
    {
        out.push_str(&rest[..pos + key.len()]);
        rest = &rest[pos + key.len()..];
        let end = rest.find(['"', ';', '}']).unwrap_or(rest.len());
        let value = rest[..end].trim();
        if value == "none" || value.starts_with("url(") || value.is_empty() {
            out.push_str(&rest[..end]);
        } else {
            out.push_str(ink);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

/// Which rail logo a leg or holding shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RailLogo {
    Onchain,
    Lightning,
    Liquid,
}

/// `RailIcon size="compact"`: the rail's logo on its own, no white disc or ring (the owner
/// removed the disc; the web's BitcoinRail.tsx still draws `bg-white ring-1`). Every rail's
/// artwork is the same 20 px in the 24 px slot, so the bitcoin coin, the Liquid disc and the
/// Lightning hexagon line up; `title` is the hover help.
pub fn rail(app: &App, cx: &Cx<App>, tk: &crate::theme::Tk, rail: RailLogo, title: &str) -> El {
    let bytes = match rail {
        RailLogo::Onchain => BITCOIN,
        RailLogo::Lightning => LIGHTNING,
        RailLogo::Liquid => LIQUID,
    };
    let (art, slot) = role::RAIL;
    fitted(app, cx, Logo::svg(bytes), tk.r(art), tk.r(slot)).help(title.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drawn_logo_is_measured() {
        let mut files: Vec<&'static [u8]> = vec![
            BITCOIN,
            LIQUID,
            LIGHTNING,
            CORE_LIGHTNING,
            LIGHTNING_LABS_SVG,
            LIGHTNING_LABS_PNG,
            COINTRACKING,
            BLOCKPIT,
        ];
        for s in crate::overview::catalog::SOURCES {
            if let crate::overview::add_connection::Art::File(bytes, ..) = s.art {
                files.push(bytes);
            }
        }
        for f in files {
            let h = fnv(f);
            assert!(
                BOUNDS.iter().any(|(k, _)| *k == h),
                "logo {h:016x} has no measured bounds; rerun tools/logo-bounds.mjs"
            );
        }
    }

    #[test]
    fn padded_artwork_is_measured_inside_its_canvas() {
        // liquid.svg's disc leaves 3.5 % of the viewBox each side; bitcoin.svg's coin fills it.
        let l = bounds(LIQUID);
        assert!((l[2] - l[0] - 0.93).abs() < 0.01);
        assert_eq!(bounds(BITCOIN), FULL);
    }
}
