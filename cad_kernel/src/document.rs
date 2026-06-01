// Document — the in-memory drawing container.
//
// Analog of AutoCAD's AcDbDatabase / LibreCAD's RS_Graphic. Holds the
// Dobject list plus all the resource tables they reference (layers,
// linetypes). Future tables (blocks, text_styles, dim_styles, ucs_list,
// named_views) slot in as new fields without touching call sites that
// already work with the current ones.

use crate::dobject::DObject;
use crate::layer::LayerTable;
use crate::linetype::LinetypeTable;
use crate::pen::PenTable;

pub struct Document {
    pub dobjects:  Vec<DObject>,
    pub layers:    LayerTable,
    pub linetypes: LinetypeTable,
    pub pens:      PenTable,
    // Reserved for future slices — leave the field list extensible:
    // pub blocks:      BlockTable,
    // pub text_styles: TextStyleTable,
    // pub dim_styles:  DimStyleTable,
    // pub ucs_list:    UcsList,
    // pub named_views: NamedViewList,
    // pub doc_settings: DocSettings,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            dobjects:  Vec::new(),
            layers:    LayerTable::with_defaults(),
            linetypes: LinetypeTable::with_defaults(),
            pens:      PenTable::default(),
        }
    }
}

impl Document {
    /// Append a Dobject. Returns its new index in `dobjects`.
    pub fn push(&mut self, mut d: DObject) -> usize {
        // If the Dobject is being added with the default Style, pull the
        // active layer in so it inherits the user's current layer choice.
        if d.style.layer == LayerTable::LAYER_ZERO && self.layers.active != LayerTable::LAYER_ZERO {
            d.style.layer = self.layers.active;
        }
        let i = self.dobjects.len();
        self.dobjects.push(d);
        i
    }

    /// Count of layers (always ≥ 1 thanks to layer "0").
    pub fn layer_count(&self) -> usize { self.layers.len() }

    /// Convenience — does this Dobject pass per-layer render gating?
    pub fn is_visible(&self, dobj_index: usize) -> bool {
        let Some(d) = self.dobjects.get(dobj_index) else { return false; };
        if !d.style.visible { return false; }
        self.layers.renders(d.style.layer)
    }

    /// Convenience — can this Dobject be selected / edited?
    pub fn is_selectable(&self, dobj_index: usize) -> bool {
        let Some(d) = self.dobjects.get(dobj_index) else { return false; };
        if !d.style.visible { return false; }
        self.layers.selectable(d.style.layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Circle, Geom};
    use crate::layer::Layer;
    use crate::math::Vec2;

    #[test]
    fn push_inherits_active_layer() {
        let mut doc = Document::default();
        let walls = doc.layers.add(Layer {
            name: "WALLS".into(), ..Layer::layer_zero()
        });
        doc.layers.active = walls;
        let i = doc.push(DObject::new(Geom::Circle(Circle {
            center: Vec2::ZERO, radius: 5.0,
        })));
        assert_eq!(doc.dobjects[i].style.layer, walls);
    }

    #[test]
    fn invisible_layer_hides_dobject() {
        let mut doc = Document::default();
        let hidden = doc.layers.add(Layer {
            name: "HIDDEN".into(), visible: false, ..Layer::layer_zero()
        });
        doc.layers.active = hidden;
        let i = doc.push(DObject::new(Geom::Circle(Circle {
            center: Vec2::ZERO, radius: 5.0,
        })));
        assert!(!doc.is_visible(i));
        assert!(!doc.is_selectable(i));
    }

    #[test]
    fn locked_layer_blocks_selection_but_renders() {
        let mut doc = Document::default();
        let locked = doc.layers.add(Layer {
            name: "LOCKED".into(), locked: true, ..Layer::layer_zero()
        });
        doc.layers.active = locked;
        let i = doc.push(DObject::new(Geom::Circle(Circle {
            center: Vec2::ZERO, radius: 5.0,
        })));
        assert!(doc.is_visible(i));
        assert!(!doc.is_selectable(i));
    }
}
