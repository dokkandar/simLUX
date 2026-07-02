//! Command metadata registry — **Phase 1 (schema only)**.
//!
//! See `mentor MD/COMMAND_REGISTRY_MENTOR.md`. This defines the TYPES that
//! *describe* commands (id, dispatch token, title, tooltip, category, icon). It
//! does **not** execute anything — execution still flows through
//! `run_command(cmd.dispatch)`, unchanged (Phase 0 freeze). No data is populated
//! here (that is Phase 2) and nothing renders from it yet (Phase 3+), so the app
//! is visually identical.
//!
//! The [`CommandInfo`] struct ACCUMULATES in later phases (never re-declared):
//! `keywords` / `group` in Phase 5; `visible` / `enabled` predicates in Phase 6b.
//! There is **no `aliases` field, ever** (parser aliases stay kernel-internal).
#![allow(dead_code)] // Phase 1: the types exist but are not wired up yet.

use std::collections::HashMap;

use crate::app::GlyphKind;

/// Stable, namespaced command identity — e.g. `"draw.line"`. This is the
/// registry / UI / palette identity, and the value persisted in `draw_items`
/// (Phase 3), so it must be stable before persistence.
///
/// It is a **string, not an int/enum** (Phase 1 lock — an enum would break
/// "reuse the existing dispatch"). It is **owned** (`String`) rather than
/// `&'static str` because Phase 2 DERIVES it by concatenation
/// (`"<category>." + dispatch`), which allocates. The execution token,
/// [`CommandInfo::dispatch`], stays `&'static str`.
pub type CommandId = String;

/// Which glyph family paints a command's icon — spans BOTH painters:
/// `DrawGlyph(id)` → `draw_draw_glyph` (the draw-rail id string, col 1 of
/// `DRAW_CMDS`); `ModifyGlyph(kind)` → `draw_cmd_glyph` (col 1 of `MODIFY_CMDS`).
/// A flat id could not serve both. Lucide icons arrive later (Phase 7 refactor).
#[derive(Clone, Copy, Debug)]
pub enum IconId {
    DrawGlyph(&'static str),
    ModifyGlyph(GlyphKind),
}

/// Top-level command category. Extensible.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandCategory {
    Draw,
    Modify,
}

/// Metadata that DESCRIBES a command — never how it executes.
///
/// Phase 1 shape. This struct is EXTENDED in later phases, never re-declared.
/// Holds no closures / `FnMut` / `&mut CadApp`.
#[derive(Clone, Debug)]
pub struct CommandInfo {
    /// Namespaced identity, e.g. `"draw.line"` (the HashMap key). See [`CommandId`].
    pub id: CommandId,
    /// The `run_command` token, e.g. `"line"`. The ONLY value ever passed to
    /// `run_command` — surfaces call `run_command(cmd.dispatch)`, never
    /// `run_command(cmd.id)`.
    pub dispatch: &'static str,
    /// Display name, e.g. `"Line"`.
    pub title: String,
    /// Hover text, e.g. `"Line  (L)"`.
    pub tooltip: String,
    /// `Draw` | `Modify`.
    pub category: CommandCategory,
    /// How the command's icon is painted.
    pub icon: IconId,
}

/// The metadata registry: lookup by [`CommandId`]. **Empty in Phase 1**
/// (populated in Phase 2). Holds no execution state — description only.
///
/// (The canonical ordered index used for deterministic menu order is added in
/// Phase 6; it is not needed yet.)
#[derive(Default)]
pub struct CommandRegistry {
    pub commands: HashMap<CommandId, CommandInfo>,
}

impl CommandRegistry {
    /// A new, empty registry.
    pub fn new() -> Self {
        Self { commands: HashMap::new() }
    }

    /// Defensive lookup by id — `None` for a stale / unknown id (never panics).
    pub fn get(&self, id: &str) -> Option<&CommandInfo> {
        self.commands.get(id)
    }
}

/// Derive the display `title` from a rail tooltip by stripping a trailing
/// `(KEY)` hint: `"Line  (L)"` → `"Line"`, `"Rectangle  (REC)"` → `"Rectangle"`,
/// `"Wall"` → `"Wall"` (no hint), `"Elliptical arc"` → `"Elliptical arc"`.
/// (Wording is refined in a later phase; this is the derived seed.)
fn derive_title(tooltip: &str) -> String {
    match (tooltip.rfind('('), tooltip.ends_with(')')) {
        (Some(i), true) => tooltip[..i].trim_end().to_string(),
        _ => tooltip.trim().to_string(),
    }
}

/// Build the registry by DERIVING every entry from the existing rail arrays
/// (`DRAW_CMDS` / `MODIFY_CMDS`) — the arrays stay the single source of truth
/// (Phase 2). Called once at startup by `CadApp`. Nothing renders from the
/// result yet.
///
/// Per entry (all derived, no hand-typing): `dispatch` = col 2; `id` =
/// `"<category>." + dispatch`; `tooltip` = col 3; `title` = [`derive_title`];
/// `icon` = `DrawGlyph`/`ModifyGlyph` of col 1.
pub fn build(
    draw: &[(&'static str, &'static str, &'static str)],
    modify: &[(GlyphKind, &'static str, &'static str)],
) -> CommandRegistry {
    let mut reg = CommandRegistry::new();
    for &(icon_id, dispatch, tooltip) in draw {
        let info = CommandInfo {
            id: format!("draw.{}", dispatch),
            dispatch,
            title: derive_title(tooltip),
            tooltip: tooltip.to_string(),
            category: CommandCategory::Draw,
            icon: IconId::DrawGlyph(icon_id),
        };
        reg.commands.insert(info.id.clone(), info);
    }
    for &(kind, dispatch, tooltip) in modify {
        let info = CommandInfo {
            id: format!("modify.{}", dispatch),
            dispatch,
            title: derive_title(tooltip),
            tooltip: tooltip.to_string(),
            category: CommandCategory::Modify,
            icon: IconId::ModifyGlyph(kind),
        };
        reg.commands.insert(info.id.clone(), info);
    }
    reg
}
