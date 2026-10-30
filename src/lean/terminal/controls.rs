/// Drop string controls (including bitmap protocols) before the emulator can
/// allocate image buffers. Keep ANSI/CSI streaming, with a bounded sequence.
#[derive(Default)]
pub(super) struct Filter {
    mode: u8,
    pub(super) sequence: Vec<u8>,
    osc: bool,
}

impl Filter {
    fn finish_string(&mut self, terminator: &[u8], output: &mut Vec<u8>) {
        // Preserve bounded palette queries needed by native CLI theme
        // detection. Graphics, clipboard and other string controls stay out.
        if self.osc && self.sequence.starts_with(b"\x1b]") {
            let query = std::str::from_utf8(&self.sequence[2..]).unwrap_or("");
            let parts: Vec<_> = query.split(';').collect();
            let dynamic = matches!(parts.as_slice(), ["10" | "11" | "12", "?"]);
            let indexed =
                matches!(parts.as_slice(), ["4", index, "?"] if index.parse::<u8>().is_ok());
            if dynamic || indexed {
                output.append(&mut self.sequence);
                output.extend_from_slice(terminator);
            }
        }
        self.sequence.clear();
        self.osc = false;
        self.mode = 0;
    }
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            self.feed_byte(byte, &mut output);
        }
        output
    }

    fn feed_byte(&mut self, byte: u8, output: &mut Vec<u8>) {
        match self.mode {
            0 => self.ground(byte, output),
            1 => self.escape(byte, output),
            2 => self.csi(byte, output),
            3 => self.string(byte, output),
            4 => self.string_escape(byte, output),
            _ if (0x40..=0x7e).contains(&byte) => self.mode = 0,
            _ => {}
        }
    }

    fn ground(&mut self, byte: u8, output: &mut Vec<u8>) {
        if byte == 27 {
            self.mode = 1;
        } else {
            output.push(byte);
        }
    }

    fn escape(&mut self, byte: u8, output: &mut Vec<u8>) {
        match byte {
            b'[' => {
                self.sequence = vec![27, b'['];
                self.mode = 2;
            }
            b']' | b'P' | b'_' | b'^' => self.begin_string(byte),
            _ => {
                output.extend_from_slice(&[27, byte]);
                self.mode = 0;
            }
        }
    }

    fn begin_string(&mut self, introducer: u8) {
        self.osc = introducer == b']';
        self.sequence.clear();
        if self.osc {
            self.sequence.extend_from_slice(b"\x1b]");
        }
        self.mode = 3;
    }

    fn csi(&mut self, byte: u8, output: &mut Vec<u8>) {
        if self.sequence.len() < 1024 {
            self.sequence.push(byte);
        } else {
            self.sequence.clear();
            self.mode = 5;
        }
        if (0x40..=0x7e).contains(&byte) {
            output.append(&mut self.sequence);
            self.mode = 0;
        }
    }

    fn string(&mut self, byte: u8, output: &mut Vec<u8>) {
        if byte == 7 {
            self.finish_string(&[7], output);
        } else if byte == 27 {
            self.mode = 4;
        } else if self.osc {
            self.append_osc(byte);
        }
    }

    fn append_osc(&mut self, byte: u8) {
        if self.sequence.len() < 1024 {
            self.sequence.push(byte);
        } else {
            self.sequence.clear();
            self.osc = false;
        }
    }

    fn string_escape(&mut self, byte: u8, output: &mut Vec<u8>) {
        if byte == b'\\' || byte == 7 {
            self.finish_string(&[27, byte], output);
        } else if byte != 27 {
            self.osc = false;
            self.sequence.clear();
            self.mode = 3;
        }
    }
}
