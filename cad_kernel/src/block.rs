//! Blocks — named, reusable groups of dobjects (AutoCAD BLOCK / INSERT).
//!
//! A `Block` is a DEFINITION: a name, a base point, and the contained
//! dobjects stored in DEFINITION SPACE (their coordinates as captured at
//! create time; the base point is the handle they're carried by). It is
//! never drawn directly.
//!
//! A `BlockRef` (`Geom::BlockRef`) is a placed INSTANCE: it references a
//! definition by id and carries a similarity transform —
//! `p_world = insert + R(rotation) · scale · (p_def − base)`.
//!
//! v1 constraints (documented, deliberate):
//! - **Uniform scale only.** A similarity transform maps circles→circles
//!   and arcs→arcs, so every contained Geom transforms exactly with the
//!   existing `scaled`/`rotated`/`translated` methods. Non-uniform scale
//!   (AutoCAD's arc→ellipse pathology) is deferred.
//! - **No true mirrored instances.** `Geom::mirrored` on a BlockRef
//!   reflects the insertion point and rotation but the content keeps its
//!   handedness (a `mirrored: bool` flag is the B2 fix). Mirror a block
//!   you care about by exploding first.
//! - **Cycles cannot form**: a definition gets its id only at creation
//!   and there is no block-redefinition yet, so a block can never
//!   (transitively) contain itself.

use crate::dobject::DObject;
use crate::geom::Geom;
use crate::math::Vec2;

/// A placed instance of a block definition.
#[derive(Clone, Copy, Debug)]
pub struct BlockRef {
    /// Id into `Document.blocks`.
    pub block:    u32,
    /// World-space insertion point (where the definition's base lands).
    pub insert:   Vec2,
    /// Uniform scale factor (v1; > 0).
    pub scale:    f64,
    /// Rotation in radians, CCW.
    pub rotation: f64,
}

impl BlockRef {
    /// Map a definition-space geom into world space for this instance:
    /// scale about the base, rotate about the base, then carry the base
    /// to the insertion point. Works for every Geom variant — including
    /// nested `BlockRef`s, which is what makes nested blocks render and
    /// explode for free.
    pub fn transform_geom(&self, g: &Geom, base: Vec2) -> Geom {
        g.scaled(base, self.scale)
            .rotated(base, self.rotation)
            .translated(self.insert - base)
    }
}

/// A block definition. Contained dobjects keep their full per-dobject
/// style (layer / color / linetype); `Color::ByBlock` entries resolve to
/// the instance's color at render time.
#[derive(Clone, Debug)]
pub struct Block {
    pub name:     String,
    /// Base point in definition space (the "grip" the instance carries).
    pub base:     Vec2,
    pub dobjects: Vec<DObject>,
    /// Smart-block marker. When true the definition is intended to be
    /// re-derived by a (forthcoming) smart-block algorithm rather than
    /// treated as a static instance. No behaviour is attached yet — the
    /// flag is carried so the editor/UI can mark it and the algorithm can
    /// hook in later. NOT yet persisted to RSM (reader defaults it false,
    /// like the dim/wall style tables — see rsm.rs).
    pub smart:    bool,
}

/// Table of block definitions on the Document. Unlike the style tables
/// there is NO reserved entry at id 0 — an empty drawing has no blocks.
#[derive(Clone, Debug, Default)]
pub struct BlockTable {
    pub blocks: Vec<Block>,
}

impl BlockTable {
    pub fn get(&self, id: u32) -> Option<&Block> {
        self.blocks.get(id as usize)
    }
    pub fn add(&mut self, b: Block) -> u32 {
        let id = self.blocks.len() as u32;
        self.blocks.push(b);
        id
    }
    pub fn find(&self, name: &str) -> Option<u32> {
        self.blocks.iter().position(|b| b.name.eq_ignore_ascii_case(name))
            .map(|i| i as u32)
    }
    pub fn len(&self) -> usize { self.blocks.len() }
    pub fn is_empty(&self) -> bool { self.blocks.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Circle, Line};

    fn close(p: Vec2, q: Vec2) -> bool { (p - q).len() < 1e-9 }

    #[test]
    fn transform_geom_composes_scale_rotate_translate() {
        // Definition: line (1,0)→(2,0), base (1,0). Instance at (10,10),
        // scale 2, rotation 90° CCW. Expected: start lands ON the insert
        // point; end = insert + R90·2·(1,0) = (10,12).
        let br = BlockRef {
            block: 0,
            insert: Vec2::new(10.0, 10.0),
            scale: 2.0,
            rotation: std::f64::consts::FRAC_PI_2,
        };
        let g = Geom::Line(Line { a: Vec2::new(1.0, 0.0), b: Vec2::new(2.0, 0.0) });
        let out = br.transform_geom(&g, Vec2::new(1.0, 0.0));
        let Geom::Line(l) = out else { panic!("expected line") };
        assert!(close(l.a, Vec2::new(10.0, 10.0)), "a={:?}", l.a);
        assert!(close(l.b, Vec2::new(10.0, 12.0)), "b={:?}", l.b);
    }

    #[test]
    fn transform_geom_keeps_circles_circular() {
        // Uniform scale ⇒ circle stays a circle with scaled radius.
        let br = BlockRef {
            block: 0,
            insert: Vec2::new(5.0, 0.0),
            scale: 3.0,
            rotation: 1.234,
        };
        let g = Geom::Circle(Circle { center: Vec2::new(0.0, 0.0), radius: 2.0 });
        let out = br.transform_geom(&g, Vec2::new(0.0, 0.0));
        let Geom::Circle(c) = out else { panic!("expected circle") };
        assert!(close(c.center, Vec2::new(5.0, 0.0)));
        assert!((c.radius - 6.0).abs() < 1e-9);
    }
}
