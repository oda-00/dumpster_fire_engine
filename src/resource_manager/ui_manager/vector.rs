//! Handrolled, zero-dependency vector rasterizer.
//!
//! Parses the SVG subset used by the bundled Lucide icons — `<path>`
//! (commands M/L/H/V/C/S/Q/T/A/Z, absolute + relative), `<circle>`, and
//! `<rect>` — and rasterizes round-capped / round-joined strokes into an R8
//! alpha buffer via a signed-distance coverage pass (each stroke segment is a
//! capsule; pixel coverage = clamp(half_width + 0.5 − dist_to_nearest, 0, 1)).
//!
//! This is the engine's own vector path, in the spirit of the hand-coded
//! `font.rs` bitmap: no `image`/`usvg`/`resvg`/`cairo` crate, no offline
//! system tool. It runs at *runtime* — the UI atlas bakes icons through it at
//! startup, and gameplay code can rasterize arbitrary SVG-subset art the same
//! way (`rasterize_svg`).
//!
//! Round caps and joins fall out of the capsule (point-to-segment) distance
//! for free, so closed shapes (circles, rounded rects, `Z`-closed paths) and
//! open polylines share one rasterization path.

/// Flattened stroke geometry in source (SVG user) space: a set of polylines.
/// Open polylines are open strokes; closed shapes include their closing edge.
#[derive(Default, Clone)]
pub struct VectorShape {
    pub subpaths: Vec<Vec<(f32, f32)>>,
}

impl VectorShape {
    fn push_segments(&self, scale: f32, out: &mut Vec<[(f32, f32); 2]>) {
        for sp in &self.subpaths {
            for w in sp.windows(2) {
                let a = (w[0].0 * scale, w[0].1 * scale);
                let b = (w[1].0 * scale, w[1].1 * scale);
                out.push([a, b]);
            }
            // A lone point (degenerate subpath) still renders a round dot.
            if sp.len() == 1 {
                let a = (sp[0].0 * scale, sp[0].1 * scale);
                out.push([a, a]);
            }
        }
    }
}

// ── SVG number / command lexer for path data ────────────────────────────────

struct Lex<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Lex<'a> {
    fn new(s: &'a str) -> Self {
        Self { b: s.as_bytes(), i: 0 }
    }

    fn skip_sep(&mut self) {
        while self.i < self.b.len() {
            match self.b[self.i] {
                b' ' | b',' | b'\n' | b'\t' | b'\r' => self.i += 1,
                _ => break,
            }
        }
    }

    /// Peek whether the next non-separator byte is a command letter.
    fn next_is_cmd(&mut self) -> bool {
        self.skip_sep();
        self.i < self.b.len() && self.b[self.i].is_ascii_alphabetic()
    }

    fn cmd(&mut self) -> Option<u8> {
        self.skip_sep();
        if self.i < self.b.len() && self.b[self.i].is_ascii_alphabetic() {
            let c = self.b[self.i];
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn num(&mut self) -> Option<f32> {
        self.skip_sep();
        let b = self.b;
        let start = self.i;
        if self.i >= b.len() {
            return None;
        }
        if b[self.i] == b'+' || b[self.i] == b'-' {
            self.i += 1;
        }
        let mut any = false;
        while self.i < b.len() && b[self.i].is_ascii_digit() {
            self.i += 1;
            any = true;
        }
        if self.i < b.len() && b[self.i] == b'.' {
            self.i += 1;
            while self.i < b.len() && b[self.i].is_ascii_digit() {
                self.i += 1;
                any = true;
            }
        }
        if !any {
            self.i = start;
            return None;
        }
        if self.i < b.len() && (b[self.i] == b'e' || b[self.i] == b'E') {
            let save = self.i;
            self.i += 1;
            if self.i < b.len() && (b[self.i] == b'+' || b[self.i] == b'-') {
                self.i += 1;
            }
            let mut e = false;
            while self.i < b.len() && b[self.i].is_ascii_digit() {
                self.i += 1;
                e = true;
            }
            if !e {
                self.i = save;
            }
        }
        std::str::from_utf8(&b[start..self.i]).ok()?.parse::<f32>().ok()
    }

    /// Arc flags are single `0`/`1` digits that may be glued to the next
    /// number; read exactly one digit when possible, else fall back to a number.
    fn flag(&mut self) -> Option<f32> {
        self.skip_sep();
        if self.i < self.b.len() {
            match self.b[self.i] {
                b'0' => {
                    self.i += 1;
                    Some(0.0)
                }
                b'1' => {
                    self.i += 1;
                    Some(1.0)
                }
                _ => self.num(),
            }
        } else {
            None
        }
    }
}

const CUBIC_STEPS: usize = 18;
const QUAD_STEPS: usize = 14;
const ARC_STEPS: usize = 24;
const CIRCLE_STEPS: usize = 64;

fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    out: &mut Vec<(f32, f32)>,
) {
    for i in 1..=CUBIC_STEPS {
        let t = i as f32 / CUBIC_STEPS as f32;
        let u = 1.0 - t;
        let w0 = u * u * u;
        let w1 = 3.0 * u * u * t;
        let w2 = 3.0 * u * t * t;
        let w3 = t * t * t;
        out.push((
            w0 * p0.0 + w1 * p1.0 + w2 * p2.0 + w3 * p3.0,
            w0 * p0.1 + w1 * p1.1 + w2 * p2.1 + w3 * p3.1,
        ));
    }
}

fn flatten_quad(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), out: &mut Vec<(f32, f32)>) {
    for i in 1..=QUAD_STEPS {
        let t = i as f32 / QUAD_STEPS as f32;
        let u = 1.0 - t;
        out.push((
            u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
            u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
        ));
    }
}

/// Endpoint → center elliptic arc, then sampled to line segments.
/// Implements the SVG implementation notes (F.6.5).
#[allow(clippy::too_many_arguments)]
fn flatten_arc(
    p0: (f32, f32),
    mut rx: f32,
    mut ry: f32,
    x_rot_deg: f32,
    large_arc: bool,
    sweep: bool,
    p1: (f32, f32),
    out: &mut Vec<(f32, f32)>,
) {
    if rx.abs() < 1e-6 || ry.abs() < 1e-6 || (p0.0 == p1.0 && p0.1 == p1.1) {
        out.push(p1);
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    let phi = x_rot_deg.to_radians();
    let (sp, cp) = phi.sin_cos();

    let dx = (p0.0 - p1.0) * 0.5;
    let dy = (p0.1 - p1.1) * 0.5;
    let x1p = cp * dx + sp * dy;
    let y1p = -sp * dx + cp * dy;

    // Correct out-of-range radii.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }

    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
    if large_arc == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p) / ry;
    let cyp = coef * -(ry * x1p) / rx;

    let cx = cp * cxp - sp * cyp + (p0.0 + p1.0) * 0.5;
    let cy = sp * cxp + cp * cyp + (p0.1 + p1.1) * 0.5;

    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };

    let theta1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dtheta = ang(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dtheta > 0.0 {
        dtheta -= std::f32::consts::TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += std::f32::consts::TAU;
    }

    for i in 1..=ARC_STEPS {
        let t = theta1 + dtheta * (i as f32 / ARC_STEPS as f32);
        let (st, ct) = t.sin_cos();
        let ex = cx + rx * ct * cp - ry * st * sp;
        let ey = cy + rx * ct * sp + ry * st * cp;
        out.push((ex, ey));
    }
}

/// Parse one SVG path `d` string into flattened subpaths (user-space).
pub fn parse_path(d: &str) -> Vec<Vec<(f32, f32)>> {
    let mut lex = Lex::new(d);
    let mut subs: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    let (mut cx, mut cy) = (0f32, 0f32);
    let (mut sx, mut sy) = (0f32, 0f32);
    // Reflection control point for S / T smoothing, and the command that set it.
    let mut prev_ctrl: Option<(f32, f32)> = None;
    let mut prev_kind: u8 = 0;

    while let Some(raw) = lex.cmd() {
        let rel = raw.is_ascii_lowercase();
        let c = raw.to_ascii_uppercase();
        let mut first = true;
        loop {
            if c != b'Z' && lex.next_is_cmd() {
                break;
            }
            match c {
                b'M' => {
                    let Some(x) = lex.num() else { break };
                    let y = lex.num().unwrap_or(0.0);
                    let (nx, ny) = if rel { (cx + x, cy + y) } else { (x, y) };
                    if first {
                        if !cur.is_empty() {
                            subs.push(std::mem::take(&mut cur));
                        }
                        cx = nx;
                        cy = ny;
                        sx = nx;
                        sy = ny;
                        cur.push((cx, cy));
                    } else {
                        cx = nx;
                        cy = ny;
                        cur.push((cx, cy));
                    }
                    prev_ctrl = None;
                }
                b'L' => {
                    let Some(x) = lex.num() else { break };
                    let y = lex.num().unwrap_or(0.0);
                    let (nx, ny) = if rel { (cx + x, cy + y) } else { (x, y) };
                    cx = nx;
                    cy = ny;
                    cur.push((cx, cy));
                    prev_ctrl = None;
                }
                b'H' => {
                    let Some(x) = lex.num() else { break };
                    cx = if rel { cx + x } else { x };
                    cur.push((cx, cy));
                    prev_ctrl = None;
                }
                b'V' => {
                    let Some(y) = lex.num() else { break };
                    cy = if rel { cy + y } else { y };
                    cur.push((cx, cy));
                    prev_ctrl = None;
                }
                b'C' => {
                    let Some(x1) = lex.num() else { break };
                    let y1 = lex.num().unwrap_or(0.0);
                    let x2 = lex.num().unwrap_or(0.0);
                    let y2 = lex.num().unwrap_or(0.0);
                    let x = lex.num().unwrap_or(0.0);
                    let y = lex.num().unwrap_or(0.0);
                    let c1 = if rel { (cx + x1, cy + y1) } else { (x1, y1) };
                    let c2 = if rel { (cx + x2, cy + y2) } else { (x2, y2) };
                    let p = if rel { (cx + x, cy + y) } else { (x, y) };
                    flatten_cubic((cx, cy), c1, c2, p, &mut cur);
                    cx = p.0;
                    cy = p.1;
                    prev_ctrl = Some(c2);
                    prev_kind = b'C';
                }
                b'S' => {
                    let Some(x2) = lex.num() else { break };
                    let y2 = lex.num().unwrap_or(0.0);
                    let x = lex.num().unwrap_or(0.0);
                    let y = lex.num().unwrap_or(0.0);
                    let c1 = match (prev_ctrl, prev_kind == b'C' || prev_kind == b'S') {
                        (Some(pc), true) => (2.0 * cx - pc.0, 2.0 * cy - pc.1),
                        _ => (cx, cy),
                    };
                    let c2 = if rel { (cx + x2, cy + y2) } else { (x2, y2) };
                    let p = if rel { (cx + x, cy + y) } else { (x, y) };
                    flatten_cubic((cx, cy), c1, c2, p, &mut cur);
                    cx = p.0;
                    cy = p.1;
                    prev_ctrl = Some(c2);
                    prev_kind = b'S';
                }
                b'Q' => {
                    let Some(x1) = lex.num() else { break };
                    let y1 = lex.num().unwrap_or(0.0);
                    let x = lex.num().unwrap_or(0.0);
                    let y = lex.num().unwrap_or(0.0);
                    let c1 = if rel { (cx + x1, cy + y1) } else { (x1, y1) };
                    let p = if rel { (cx + x, cy + y) } else { (x, y) };
                    flatten_quad((cx, cy), c1, p, &mut cur);
                    cx = p.0;
                    cy = p.1;
                    prev_ctrl = Some(c1);
                    prev_kind = b'Q';
                }
                b'T' => {
                    let Some(x) = lex.num() else { break };
                    let y = lex.num().unwrap_or(0.0);
                    let c1 = match (prev_ctrl, prev_kind == b'Q' || prev_kind == b'T') {
                        (Some(pc), true) => (2.0 * cx - pc.0, 2.0 * cy - pc.1),
                        _ => (cx, cy),
                    };
                    let p = if rel { (cx + x, cy + y) } else { (x, y) };
                    flatten_quad((cx, cy), c1, p, &mut cur);
                    cx = p.0;
                    cy = p.1;
                    prev_ctrl = Some(c1);
                    prev_kind = b'T';
                }
                b'A' => {
                    let Some(rx) = lex.num() else { break };
                    let ry = lex.num().unwrap_or(0.0);
                    let rot = lex.num().unwrap_or(0.0);
                    let large = lex.flag().unwrap_or(0.0) != 0.0;
                    let sweep = lex.flag().unwrap_or(0.0) != 0.0;
                    let x = lex.num().unwrap_or(0.0);
                    let y = lex.num().unwrap_or(0.0);
                    let p = if rel { (cx + x, cy + y) } else { (x, y) };
                    flatten_arc((cx, cy), rx, ry, rot, large, sweep, p, &mut cur);
                    cx = p.0;
                    cy = p.1;
                    prev_ctrl = None;
                }
                b'Z' => {
                    cur.push((sx, sy));
                    if !cur.is_empty() {
                        subs.push(std::mem::take(&mut cur));
                    }
                    cx = sx;
                    cy = sy;
                    prev_ctrl = None;
                    break;
                }
                _ => break,
            }
            first = false;
        }
    }
    if !cur.is_empty() {
        subs.push(cur);
    }
    subs
}

/// Extract the value of attribute `name` from a single element string.
fn attr(el: &str, name: &str) -> Option<f32> {
    let key = format!("{name}=\"");
    let start = el.find(&key)? + key.len();
    let rest = &el[start..];
    let end = rest.find('"')?;
    rest[..end].trim().parse::<f32>().ok()
}

fn attr_str<'a>(el: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let start = el.find(&key)? + key.len();
    let rest = &el[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Collect all stroke subpaths from an SVG source (path/circle/rect subset).
pub fn parse_svg(svg: &str) -> Vec<VectorShape> {
    let mut shapes = Vec::new();
    let bytes = svg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(close) = svg[i..].find('>') else { break };
        let el = &svg[i..i + close + 1];
        i += close + 1;

        if let Some(rest) = el.strip_prefix("<path") {
            if let Some(d) = attr_str(rest, "d") {
                let sub = parse_path(d);
                if !sub.is_empty() {
                    shapes.push(VectorShape { subpaths: sub });
                }
            }
        } else if el.starts_with("<circle") {
            let cx = attr(el, "cx").unwrap_or(0.0);
            let cy = attr(el, "cy").unwrap_or(0.0);
            let r = attr(el, "r").unwrap_or(0.0);
            if r > 0.0 {
                let mut poly = Vec::with_capacity(CIRCLE_STEPS + 1);
                for k in 0..=CIRCLE_STEPS {
                    let a = k as f32 / CIRCLE_STEPS as f32 * std::f32::consts::TAU;
                    poly.push((cx + r * a.cos(), cy + r * a.sin()));
                }
                shapes.push(VectorShape { subpaths: vec![poly] });
            }
        } else if el.starts_with("<rect") {
            let x = attr(el, "x").unwrap_or(0.0);
            let y = attr(el, "y").unwrap_or(0.0);
            let w = attr(el, "width").unwrap_or(0.0);
            let h = attr(el, "height").unwrap_or(0.0);
            let rx = attr(el, "rx").or_else(|| attr(el, "ry")).unwrap_or(0.0);
            if w > 0.0 && h > 0.0 {
                shapes.push(VectorShape {
                    subpaths: vec![rounded_rect(x, y, w, h, rx)],
                });
            }
        }
    }
    shapes
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<(f32, f32)> {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    if r <= 0.0 {
        return vec![
            (x, y),
            (x + w, y),
            (x + w, y + h),
            (x, y + h),
            (x, y),
        ];
    }
    let mut p = Vec::new();
    let corner = |cx: f32, cy: f32, a0: f32, a1: f32, out: &mut Vec<(f32, f32)>| {
        let steps = 8;
        for k in 0..=steps {
            let a = a0 + (a1 - a0) * (k as f32 / steps as f32);
            out.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    };
    use std::f32::consts::PI;
    p.push((x + r, y));
    p.push((x + w - r, y));
    corner(x + w - r, y + r, -PI * 0.5, 0.0, &mut p);
    p.push((x + w, y + h - r));
    corner(x + w - r, y + h - r, 0.0, PI * 0.5, &mut p);
    p.push((x + r, y + h));
    corner(x + r, y + h - r, PI * 0.5, PI, &mut p);
    p.push((x, y + r));
    corner(x + r, y + r, PI, PI * 1.5, &mut p);
    p.push((x + r, y));
    p
}

#[inline]
fn dist_to_seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let wx = px - a.0;
    let wy = py - a.1;
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 1e-12 {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = px - (a.0 + t * vx);
    let dy = py - (a.1 + t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// Rasterize SVG-subset `svg` into a `cell × cell` R8 alpha buffer.
/// `view` is the source viewBox extent (Lucide = 24.0); `stroke_units` is the
/// stroke width in source units (Lucide = 2.0). Coverage is 1px antialiased.
pub fn rasterize_svg(svg: &str, cell: u32, view: f32, stroke_units: f32) -> Vec<u8> {
    let shapes = parse_svg(svg);
    let scale = cell as f32 / view;
    let mut segs: Vec<[(f32, f32); 2]> = Vec::new();
    for s in &shapes {
        s.push_segments(scale, &mut segs);
    }
    let half = stroke_units * 0.5 * scale;
    let mut out = vec![0u8; (cell * cell) as usize];
    if segs.is_empty() {
        return out;
    }
    for py in 0..cell {
        for px in 0..cell {
            let fx = px as f32 + 0.5;
            let fy = py as f32 + 0.5;
            let mut d = f32::MAX;
            for s in &segs {
                let dd = dist_to_seg(fx, fy, s[0], s[1]);
                if dd < d {
                    d = dd;
                }
            }
            let cov = (half + 0.5 - d).clamp(0.0, 1.0);
            out[(py * cell + px) as usize] = (cov * 255.0 + 0.5) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_reads_glued_negatives() {
        let mut lx = Lex::new("15 19-3 3-3-3");
        let mut got = Vec::new();
        while let Some(n) = lx.num() {
            got.push(n);
        }
        assert_eq!(got, vec![15.0, 19.0, -3.0, 3.0, -3.0, -3.0]);
    }

    #[test]
    fn move_lines_make_a_polyline() {
        // "M12 2v20" → vertical line of 20 units.
        let subs = parse_path("M12 2v20");
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].first().copied(), Some((12.0, 2.0)));
        assert_eq!(subs[0].last().copied(), Some((12.0, 22.0)));
    }

    #[test]
    fn rasterize_produces_marks() {
        let svg = r#"<svg viewBox="0 0 24 24" stroke-width="2"><path d="M2 12h20"/></svg>"#;
        let buf = rasterize_svg(svg, 24, 24.0, 2.0);
        assert_eq!(buf.len(), 24 * 24);
        let lit = buf.iter().filter(|&&v| v > 0).count();
        assert!(lit > 0, "horizontal stroke should mark pixels");
    }

    #[test]
    fn arc_path_does_not_panic_and_marks() {
        let svg = r#"<path d="M16.47214 7.52786 A 5 10 0 1 0 13 21.79796" />"#;
        let buf = rasterize_svg(svg, 24, 24.0, 2.0);
        assert!(buf.iter().any(|&v| v > 0));
    }
}
