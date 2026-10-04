//! The window-management rules of Aqua as plain data and functions: Spaces (per display),
//! where new windows go, zoom/restore geometry, rescuing windows from unplugged displays and
//! the Mission Control grid and tiling (halves / quarters / fill). The compositor feeds in rectangles and applies the results to
//! Smithay; everything here is unit-tested without a display server.
pub mod geom;
pub mod mission;
pub mod place;
pub mod spaces;
pub mod stage;
pub mod tile;

pub use geom::{Rect, RectF};
pub use spaces::{Desks, Workspaces};
