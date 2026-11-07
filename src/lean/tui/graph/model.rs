use super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub visible: bool,
    pub group: Option<String>,
    pub collapsed: bool,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            key: String::new(),
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
            visible: true,
            group: None,
            collapsed: false,
        }
    }
}

impl Node {
    pub fn new(key: String, x: u16, y: u16) -> Self {
        Self {
            key,
            x,
            y,
            width: WIDTH,
            height: HEIGHT,
            visible: true,
            group: None,
            collapsed: false,
        }
    }

    pub(super) fn mid_y(&self) -> u16 {
        self.y.saturating_add(self.height / 2)
    }

    pub(super) fn right(&self) -> u16 {
        self.x + self.width
    }
}

/// Animation steps from a normal card to a fully expanded one.
pub const FULL_STEP: u8 = 16;

/// The card size of one layout: full size, or smaller so that a unit's
/// three cards fit across the viewport and two rows down it.
#[derive(Debug, Clone)]
pub(super) struct Size {
    pub(super) w: u16,
    pub(super) h: u16,
    /// The session's own agents stand two across, not three.
    pub(super) admin_w: u16,
    pub(super) expanded_w: u16,
    pub(super) expanded_h: u16,
    pub(super) expanded_key: Option<String>,
    /// The fully grown expanded card, before the animation step applies.
    full_w: u16,
    full_h: u16,
    step: u8,
}

impl Size {
    pub(super) fn fit(viewport: Option<(u16, u16)>, expanded_key: Option<String>) -> Self {
        let Some((vp_w, vp_h)) = viewport else {
            return Self {
                w: WIDTH,
                h: HEIGHT,
                admin_w: WIDTH,
                expanded_w: 52,
                expanded_h: 20,
                expanded_key,
                full_w: 52,
                full_h: 20,
                step: FULL_STEP,
            };
        };
        // Three cards of a unit side by side, in their frame.
        let unit = MARGIN + 2 * GAP_X + 2 * FRAME_X;
        // An expanded card keeps `MARGIN` on either side.
        let full_w = vp_w.saturating_sub(2 * MARGIN).max(MIN_WIDTH);
        let full_h = vp_h.saturating_sub(2).max(MIN_HEIGHT);
        Self {
            w: (vp_w.saturating_sub(unit) / 3).clamp(MIN_WIDTH, WIDTH),
            admin_w: (vp_w.saturating_sub(MARGIN + 2 * GAP_X) / 2).clamp(MIN_WIDTH, WIDTH),
            h: (vp_h.saturating_sub(GAP_Y) / 2).clamp(MIN_HEIGHT, HEIGHT),
            expanded_w: full_w,
            expanded_h: full_h,
            expanded_key,
            full_w,
            full_h,
            step: FULL_STEP,
        }
    }

    /// Grow the expanded card from its normal size: `step` of `FULL_STEP`.
    pub(super) fn animated(mut self, step: u8) -> Self {
        self.step = step.min(FULL_STEP);
        self.grow();
        self
    }

    /// Narrow the fully grown expanded card to `width` at most, so that it
    /// and its unit's other cards stay within the viewport.
    pub(super) fn limit_expanded_width(&mut self, width: u16) {
        self.full_w = self.full_w.min(width).max(MIN_WIDTH);
        self.grow();
    }

    fn grow(&mut self) {
        let step = u16::from(self.step);
        let full = u16::from(FULL_STEP);
        let lerp = |from: u16, to: u16| {
            if to <= from {
                to
            } else {
                from + (to - from) * step / full
            }
        };
        self.expanded_w = lerp(self.w.max(MIN_WIDTH), self.full_w);
        self.expanded_h = lerp(self.h.max(MIN_HEIGHT), self.full_h);
    }

    pub(super) fn width(&self, key: &str, admin: bool) -> u16 {
        if self.expanded_key.as_deref() == Some(key) {
            self.expanded_w
        } else if admin {
            self.admin_w
        } else {
            self.w
        }
    }

    pub(super) fn layout_height(&self, key: &str, _admin: bool) -> u16 {
        if self.expanded_key.as_deref() == Some(key) {
            self.expanded_h
        } else {
            self.h
        }
    }
}
