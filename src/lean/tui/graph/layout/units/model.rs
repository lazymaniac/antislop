pub(super) struct Unit {
    pub(super) task: usize,
    pub(super) options: Option<usize>,
    pub(super) parents: Vec<usize>,
    pub(super) visible: bool,
    pub(super) folded: Vec<bool>,
}

#[derive(Clone)]
pub(super) struct Placed {
    pub(super) unit: usize,
    pub(super) row: usize,
    pub(super) x: u16,
    pub(super) width: u16,
    pub(super) height: u16,
}

/// One dependency between two visible units.
pub(super) struct Link {
    pub(super) from: usize,
    pub(super) to: usize,
}

/// The links into one unit: they share its entry line and arrow, and the
/// ones from rows other than the one right above share one lane.
pub(super) struct Bus {
    pub(super) to: usize,
    pub(super) lane: Option<Lane>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

/// A column beside the rows, from gap `span.0` to gap `span.1`; index 0 is
/// the outermost column of its side.
pub(super) struct Lane {
    pub(super) side: Side,
    pub(super) index: u16,
    pub(super) span: (usize, usize),
    pub(super) x: u16,
}

impl Lane {
    /// Whether the lane runs beside `row`: it passes the rows between the
    /// gaps it joins, gap `g` lying above row `g`.
    pub(super) fn crosses(&self, row: usize) -> bool {
        self.span.0 <= row && row < self.span.1
    }
}

pub(super) struct Tops {
    pub(super) admin: Vec<u16>,
    pub(super) units: Vec<u16>,
    pub(super) gaps: Vec<u16>,
    pub(super) bottom: u16,
}
