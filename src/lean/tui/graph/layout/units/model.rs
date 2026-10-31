pub(super) struct Unit {
    pub(super) task: usize,
    pub(super) options: Option<usize>,
    pub(super) parents: Vec<usize>,
    pub(super) visible: bool,
    pub(super) folded: Vec<bool>,
}

pub(super) struct Placed {
    pub(super) unit: usize,
    pub(super) row: usize,
    pub(super) x: u16,
    pub(super) width: u16,
    pub(super) height: u16,
}

pub(super) struct Link {
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) lane: Option<u16>,
}

pub(super) struct Tops {
    pub(super) admin: Vec<u16>,
    pub(super) units: Vec<u16>,
    pub(super) gaps: Vec<u16>,
    pub(super) bottom: u16,
}
