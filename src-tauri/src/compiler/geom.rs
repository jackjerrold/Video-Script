//! Tiny 2D affine transform, used to compose group transforms.
//!
//! Maps a point `(x, y)` to `(a*x + c*y + e, b*x + d*y + f)`. Angles are in
//! degrees, y points down (so positive rotation turns clockwise on screen),
//! matching the rasterizer.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(tx: f64, ty: f64) -> Affine {
        Affine { e: tx, f: ty, ..Affine::IDENTITY }
    }

    pub fn rotate(deg: f64) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    pub fn scale(s: f64) -> Affine {
        Affine { a: s, d: s, ..Affine::IDENTITY }
    }

    /// `self.then(other)` applies `other` first, then `self`
    /// (i.e. the matrix product `self * other`). A parent's transform is
    /// `parent.then(child_local)`.
    pub fn then(self, o: Affine) -> Affine {
        Affine {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// Local transform of a node: translate, then rotate, then scale (about its own origin).
    pub fn local(x: f64, y: f64, rot_deg: f64, scale: f64) -> Affine {
        Affine::translate(x, y).then(Affine::rotate(rot_deg)).then(Affine::scale(scale))
    }
}
