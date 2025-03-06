use super::sm::JtagState;

#[derive(Default, Copy, Clone)]
struct Tap {
    ir_len: usize,
}

const MAX_TAPS: usize = 8; // @fixme As per ADI v5.2 A1-35 "Choices for JTAG-APs"

pub struct Taps {
    taps: [Tap; MAX_TAPS],
    num_taps: usize,
    active_tap: TapSelect,
    dangling_read: bool,
    queued_reads: usize,
}

#[derive(Default)]
struct TapSelect {
    // Selected TAP index
    index: usize,
    /// This many IR bits before selected TAP
    ir_prev: usize,
    /// This many TAPs before selected
    dr_prev: usize,
    /// IR bits in the selected TAP
    ir_len: usize,
    /// This many IR bits after the selected TAP
    ir_post: usize,
    /// This many TAPs after the selected TAP
    dr_post: usize,
}

impl Default for Taps {
    fn default() -> Self {
        Self {
            taps: [Tap::default(); MAX_TAPS],
            num_taps: 0,
            active_tap: TapSelect::default(),
            dangling_read: false,
            queued_reads: 0,
        }
    }
}

fn add_ones_to_end(input: &mut [u8], this_len: usize, shift: usize) -> Result<usize> {
    let bytes_needed = (this_len + shift + 7) / 8;
    if input.len() < bytes_needed {
        return Err(); // Not enough space in  buffer
    }

    let top_bits = (1 << (this_len % 8)) - 1;
    let end = this_len - 1;
    input[end] |= !top_bits;

    // Fill remaining bytes with 0xFF
    for i in this_len..bytes_needed {
        input[i] = 0xff;
    }
    Ok(bytes_needed)
}

impl Taps {
    pub fn setup(&mut self, chain_count: usize, ir_lens: &[u8]) {
        assert!(chain_count <= MAX_TAPS); // replace with ERROR?

        for i in 0..chain_count {
            self.taps[i].ir_len = ir_lens[i].into();
        }
        self.num_taps = chain_count;
    }

    //==================================================================================================================
    // fixme: the methods below should just generate bit patterns for JTAG_sequence/transfer_wo/_rw impl?
    //==================================================================================================================

    /// Select which TAP in the scan chain to operate upon. `ir` will be shifted into its
    /// instruction register, and the other TAPs put into bypass.
    pub fn select_tap(&mut self, tap: usize, ir: &[u8]) {
        assert!(tap <= self.num_taps); // make into an ERROR!

        // Reset JTAG sm!
        // self.jtag_state.reset()
        // tms_sequence([1,1,1,1,1,1], boom); // self.jtag_state = JtagState::Reset;

        // self.sm.mode_reset();
        self.active_tap.index = tap;
        self.write_ir(ir); // ??
    }

    fn write_ones(&mut self, mut bits: usize) {
        let bytes = bits / 8;
        bits %= 8;

        if bytes > 0 {
            let buf = vec![0xff; bytes];
            self.sm.write_ir(&buf, 8, false);
        }
        if bits > 0 {
            let buf = vec![(1 << bits) - 1];
            self.sm.write_ir(&buf, bits as u8, false);
        }
    }

    /// Shift `ir` into the instruction register of the TAP selected by `select_tap`
    pub fn write_ir(&mut self, ir: &[u8]) {
        assert!(self.active_tap.index < self.taps.len());
        let this_irlen = self.taps[self.active_tap.index].ir_len;
        assert_eq!(ir.len(), (this_irlen + 7) / 8);

        // Put downstream taps into BYPASS
        let mut after_pad = 0;
        for t in &self.taps[self.active_tap.index + 1..] {
            after_pad += t.ir_len;
        }
        self.write_ones(after_pad);

        let mut pad_bits = 0;
        for t in &self.taps[0..self.active_tap.index] {
            pad_bits += t.ir_len;
        }
        let mut total_bits = (pad_bits + this_irlen) % 8;
        if total_bits == 0 {
            total_bits = 8;
        }
        add_ones_to_end(ir, this_irlen, pad_bits)?;
        self.sm.write_ir(&ir, total_bits as u8, true);
        self.sm.change_mode(JtagState::Idle);
    }

    /// Read the instruction register of the TAP selected by `select_tap`
    pub fn read_ir(&mut self) -> Vec<u8> {
        // @todo write into passed-in rxbuf slice
        assert!(self.active_tap.index < self.taps.len());
        let this_irlen = self.taps[self.active_tap.index].ir_len;
        let mut pad_bits = 0;
        for t in &self.taps[self.active_tap.index + 1..] {
            pad_bits += t.ir_len;
        }

        // Discard the unwanted bits
        self.sm.change_mode(JtagState::Idle); // no need for this. sm can change itself
        if pad_bits > 0 {
            self.sm.read_ir(pad_bits);
        }
        self.sm.read_ir(this_irlen)
    }

    /// Shift `dr` into the data register of the TAP selected by `select_tap`. `bits` indicates
    /// how many bits of the final byte should be written (a value of 8 will write the entire byte)
    pub fn write_dr(&mut self, dr: &[u8], bits: usize) {
        assert!(self.active_tap.index < self.taps.len());
        let this_len = (dr.len() - 1) * 8 + bits;
        let pad_bits = self.active_tap.index;

        let mut total_bits = (pad_bits + this_len) % 8;
        if total_bits == 0 {
            total_bits = 8;
        }
        add_ones_to_end(dr, this_len, pad_bits)?;
        self.sm.write_dr(&dr, total_bits as u8, true);
        self.sm.change_mode(JtagState::Idle);
    }

    /// Shift `dr` into the data register of the TAP selected by `select_tap`. `bits` indicates
    /// how many bits of the final byte should be written (a value of 8 will
    /// write the entire byte).  Returns the bits that were shifted out while `dr` was
    /// shifted in.
    pub fn read_write_dr(&mut self, dr: &[u8], bits: usize) -> Vec<u8> {
        assert_eq!(self.queued_reads, 0);
        self.queue_dr_read_write(dr, bits);
        let total_bits = (dr.len() - 1) * 8 + bits;
        self.finish_dr_read(total_bits)
    }

    pub fn queue_dr_read_write(&mut self, dr: &[u8], bits: usize) -> bool {
        assert!(self.active_tap.index < self.taps.len());
        let this_len = (dr.len() - 1) * 8 + bits;
        let pad_bits = self.active_tap.index;
        let discard_bits = self.taps.len() - self.active_tap.index - 1;

        let mut total_bits = (pad_bits + this_len) % 8;
        if total_bits == 0 {
            total_bits = 8;
        }
        add_ones_to_end(dr, this_len, pad_bits)?;
        if discard_bits > 0 {
            if !self.sm.queue_read_dr(discard_bits) {
                return false;
            }
        }
        if self.sm.queue_read_write_dr(&dr, total_bits as u8, true) {
            self.sm.change_mode(JtagState::Idle);
            self.queued_reads += 1;
            true
        } else {
            self.sm.change_mode(JtagState::Idle);
            self.dangling_read = discard_bits > 0;
            false
        }
    }

    /// Read the data register of the TAP selected by `select_tap`. `bits` indicates the length of
    /// the data register for the current instruction.
    pub fn read_dr(&mut self, bits: usize) -> Vec<u8> {
        assert_eq!(self.queued_reads, 0);
        self.queue_dr_read(bits);
        self.finish_dr_read(bits)
    }

    pub fn queue_dr_read(&mut self, bits: usize) -> bool {
        assert!(self.active_tap.index < self.taps.len());
        let pad_bits = self.active_tap.index;
        let discard_bits = self.taps.len() - self.active_tap.index - 1;
        let total_bits = pad_bits + bits;

        // Discard the bypass bits
        self.sm.change_mode(JtagState::Idle);
        if discard_bits > 0 {
            if !self.sm.queue_read_dr(discard_bits) {
                return false;
            }
        }
        if !self.sm.queue_read_dr(total_bits) {
            self.dangling_read = discard_bits > 0;
            false
        } else {
            self.queued_reads += 1;
            true
        }
    }

    pub fn finish_dr_read(&mut self, bits: usize) -> Vec<u8> {
        assert!(self.active_tap.index < self.taps.len());
        let pad_bits = self.active_tap.index;
        let discard_bits = self.taps.len() - self.active_tap.index - 1;
        let total_bits = pad_bits + bits;

        // Discard the bypass bits
        if discard_bits > 0 {
            self.sm.cable.finish_read(discard_bits);
        }
        let mut ret = self.sm.cable.finish_read(total_bits);

        // Remove the pad bits
        if pad_bits > 0 {
            let bytes = (bits + 7) / 8;
            // Trim off any extra bytes
            ret.resize(bytes, 0);

            // Mask off high bits
            if bits % 8 != 0 {
                ret[bytes - 1] &= (1 << (bits % 8)) - 1;
            }
        }

        // Handle the case where we were able to queue the read of the discard bits, but not of the
        // interesting data.
        self.queued_reads -= 1;
        if self.queued_reads == 0 && self.dangling_read {
            if discard_bits > 0 {
                self.sm.cable.finish_read(discard_bits);
            }
            self.dangling_read = false;
        }
        ret
    }
}
