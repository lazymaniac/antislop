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

    /// Summary and effect lines that fit between the header slots and the
    /// meta line of a card this tall.
    pub(super) fn body_lines(&self) -> (usize, usize) {
        body_lines(self.height)
    }
}

/// How a card's height is shared between summary and effect lines; the
/// full-height card gets `MAX_SUMMARY_LINES` and `MAX_EFFECT_LINES`.
pub(super) fn body_lines(height: u16) -> (usize, usize) {
    let body = usize::from(height.saturating_sub(BODY_ROW + 1)).max(1);
    let effect = match body {
        7.. => MAX_EFFECT_LINES,
        3..=6 => 1,
        _ => 0,
    };
    ((body - effect).min(MAX_SUMMARY_LINES), effect)
}

/// The card size of one layout: full size, or smaller so that two cards fit
/// across and down the viewport.
#[derive(Debug, Clone, Copy)]
pub(super) struct Size {
    pub(super) w: u16,
    pub(super) h: u16,
}

impl Size {
    pub(super) fn fit(viewport: Option<(u16, u16)>) -> Self {
        let full = Self {
            w: WIDTH,
            h: HEIGHT,
        };
        let Some((vp_w, vp_h)) = viewport else {
            return full;
        };
        Self {
            w: (vp_w.saturating_sub(MARGIN + 2 * GAP_X) / 2).clamp(MIN_WIDTH, WIDTH),
            h: (vp_h.saturating_sub(GAP_Y) / 2).clamp(MIN_HEIGHT, HEIGHT),
        }
    }

    pub(super) fn node(self, key: String, x: u16, y: u16) -> Node {
        Node {
            width: self.w,
            height: self.h,
            ..Node::new(key, x, y)
        }
    }
}
